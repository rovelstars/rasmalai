//! In-house relocatable WebAssembly linker.
//!
//! The compiler emits one LLVM wasm object per codegen unit. Each object
//! carries the sections a final module needs plus a `linking` custom
//! section with a symbol table and `reloc.CODE` / `reloc.DATA` custom
//! sections with the relocations to apply. Archive inputs arrive as `ar`
//! archives; members are pulled on demand, exactly like a native linker.
//!
//! The output is one command module: a `_start` export supplied by the WASI
//! runtime archive, a `memory` export, imports from
//! `wasi_snapshot_preview1`, and deterministic section bytes.
//!
//! Layout decisions that the emitted code depends on:
//!
//! - Linear memory starts with a guard region: static data begins at
//!   `DATA_BASE`, one stack region follows it, and `__heap_base` points at
//!   the first byte the runtime allocator owns.
//! - Slot 0 of the function table stays empty, so a null function reference
//!   stays distinguishable from a real one. Every function whose address is
//!   taken gets the next slot, and `TABLE_INDEX_*` relocations read the same
//!   map.
//! - Relocation sites are padded LEBs, so patching a site never changes a
//!   body length and the code-section layout can be computed before
//!   relocations are applied.

use crate::LinkError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const DATA_BASE: u32 = 1024;
const STACK_SIZE: u32 = 1024 * 1024;
const PAGE_SIZE: u32 = 65536;
const TABLE_SLOT_BASE: u32 = 1;

const WASM_MAGIC: &[u8] = b"\0asm";

const SEC_TYPE: u8 = 1;
const SEC_IMPORT: u8 = 2;
const SEC_FUNCTION: u8 = 3;
const SEC_TABLE: u8 = 4;
const SEC_MEMORY: u8 = 5;
const SEC_GLOBAL: u8 = 6;
const SEC_EXPORT: u8 = 7;
const SEC_ELEMENT: u8 = 9;
const SEC_CODE: u8 = 10;
const SEC_DATA: u8 = 11;
const SEC_DATACOUNT: u8 = 12;

const SYM_UNDEFINED: u32 = 0x10;
const SYM_EXPLICIT_NAME: u32 = 0x40;
const SYM_BINDING_WEAK: u32 = 0x1;

const SYM_KIND_FUNCTION: u8 = 0;
const SYM_KIND_DATA: u8 = 1;
const SYM_KIND_GLOBAL: u8 = 2;
const SYM_KIND_SECTION: u8 = 3;
const SYM_KIND_TABLE: u8 = 5;

const R_FUNCTION_INDEX_LEB: u8 = 0;
const R_TABLE_INDEX_SLEB: u8 = 1;
const R_TABLE_INDEX_I32: u8 = 2;
const R_MEMORY_ADDR_LEB: u8 = 3;
const R_MEMORY_ADDR_SLEB: u8 = 4;
const R_MEMORY_ADDR_I32: u8 = 5;
const R_TYPE_INDEX_LEB: u8 = 6;
const R_GLOBAL_INDEX_LEB: u8 = 7;
const R_FUNCTION_OFFSET_I32: u8 = 8;
const R_SECTION_OFFSET_I32: u8 = 9;
const R_MEMORY_ADDR_REL_SLEB: u8 = 11;
const R_TABLE_INDEX_REL_SLEB: u8 = 12;
const R_GLOBAL_INDEX_I32: u8 = 13;
const R_TABLE_NUMBER_LEB: u8 = 20;
const R_FUNCTION_INDEX_I32: u8 = 26;

#[derive(Debug)]
pub enum WasmInput {
    Object(Vec<u8>),
    ObjectPath(PathBuf),
    Archive(Vec<u8>),
    ArchivePath(PathBuf),
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct FuncType {
    params: Vec<u8>,
    rets: Vec<u8>,
}

#[derive(Debug, Clone)]
enum ImportKind {
    Func(u32),
    Table { min: u32 },
    Memory { min: u32 },
    Global { ty: u8, mutable: bool },
}

#[derive(Debug, Clone)]
struct Import {
    module: String,
    name: String,
    kind: ImportKind,
}

#[derive(Debug, Clone)]
struct GlobalDef {
    ty: u8,
    mutable: bool,
    init: Vec<u8>,
}

#[derive(Debug, Clone)]
struct DataSegment {
    offset: i64,
    bytes: Vec<u8>,
    payload_offset: u32,
    assigned: u32,
}

#[derive(Debug, Clone)]
struct Symbol {
    kind: u8,
    flags: u32,
    index: u32,
    name: String,
    module: String,
    data_offset: u32,
}

impl Symbol {
    fn is_undefined(&self) -> bool {
        self.flags & SYM_UNDEFINED != 0
    }

    fn is_weak(&self) -> bool {
        self.flags & SYM_BINDING_WEAK != 0
    }

    fn is_external_module(&self) -> bool {
        !self.module.is_empty() && self.module != "env"
    }
}

#[derive(Debug, Clone, Copy)]
struct Reloc {
    ty: u8,
    offset: u32,
    sym: u32,
    addend: i64,
}

#[derive(Debug)]
struct Obj {
    types: Vec<FuncType>,
    imports: Vec<Import>,
    func_types: Vec<u32>,
    globals: Vec<GlobalDef>,
    elems: Vec<Vec<u32>>,
    code: Vec<Vec<u8>>,
    data: Vec<DataSegment>,
    symbols: Vec<Symbol>,
    reloc_code: Vec<Reloc>,
    reloc_data: Vec<Reloc>,
    imported_funcs: u32,
    imported_globals: u32,
    table_min: u32,
    data_addr: u32,
    code_section_ordinal: Option<u32>,
    data_section_ordinal: Option<u32>,
    memory_base: Option<u32>,
    table_base: Option<u32>,
    body_offsets: Vec<u32>,
    out_body_offsets: Vec<u32>,
}

struct Reader<'a> {
    bytes: &'a [u8],
    pos: usize,
    what: &'static str,
}

impl<'a> Reader<'a> {
    fn new(bytes: &'a [u8], what: &'static str) -> Reader<'a> {
        Reader {
            bytes,
            pos: 0,
            what,
        }
    }

    fn fail(&self, message: impl Into<String>) -> LinkError {
        LinkError::Unsupported(format!("wasm {}: {}", self.what, message.into()))
    }

    fn done(&self) -> bool {
        self.pos >= self.bytes.len()
    }

    fn u8(&mut self) -> Result<u8, LinkError> {
        let value = *self
            .bytes
            .get(self.pos)
            .ok_or_else(|| self.fail("truncated section"))?;
        self.pos += 1;
        Ok(value)
    }

    fn take(&mut self, count: usize) -> Result<&'a [u8], LinkError> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| self.fail("section length overflow"))?;
        let slice = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| self.fail("truncated section"))?;
        self.pos = end;
        Ok(slice)
    }

    fn uleb(&mut self) -> Result<u32, LinkError> {
        let mut result: u32 = 0;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            let part = (byte & 0x7f) as u32;
            if shift >= 32 || (shift == 28 && part > 0x0f) {
                return Err(self.fail("LEB128 value out of range"));
            }
            result |= part << shift;
            if byte & 0x80 == 0 {
                return Ok(result);
            }
            shift += 7;
        }
    }

    fn sleb(&mut self) -> Result<i64, LinkError> {
        let mut result: i64 = 0;
        let mut shift = 0;
        loop {
            let byte = self.u8()?;
            result |= ((byte & 0x7f) as i64) << shift;
            shift += 7;
            if byte & 0x80 == 0 {
                if shift < 64 && byte & 0x40 != 0 {
                    result |= -1i64 << shift;
                }
                return Ok(result);
            }
            if shift >= 64 {
                return Err(self.fail("LEB128 value out of range"));
            }
        }
    }

    fn name(&mut self) -> Result<String, LinkError> {
        let len = self.uleb()? as usize;
        let bytes = self.take(len)?;
        core::str::from_utf8(bytes)
            .map(str::to_string)
            .map_err(|_| self.fail("name is not UTF-8"))
    }
}

fn write_uleb_width(out: &mut [u8], value: u32) -> Result<(), LinkError> {
    let width = out.len();
    if width == 0 || width > 5 || (width < 5 && value >= (1u32 << (7 * width))) {
        return Err(LinkError::Unsupported(format!(
            "wasm relocation value {value} does not fit its {width}-byte LEB site"
        )));
    }
    let mut rest = value;
    for (index, slot) in out.iter_mut().enumerate() {
        let mut byte = (rest & 0x7f) as u8;
        rest >>= 7;
        if index + 1 < width {
            byte |= 0x80;
        }
        *slot = byte;
    }
    Ok(())
}

fn write_sleb_width(out: &mut [u8], value: i64) -> Result<(), LinkError> {
    let width = out.len();
    if width == 0 || width > 10 {
        return Err(LinkError::Unsupported(
            "wasm sleb relocation has an empty or oversized site".to_string(),
        ));
    }
    let mut rest = value;
    for (index, slot) in out.iter_mut().enumerate() {
        let byte = (rest & 0x7f) as u8;
        rest >>= 7;
        let last = index + 1 == width;
        if !last {
            *slot = byte | 0x80;
            continue;
        }
        let sign_ok = (rest == 0 && byte & 0x40 == 0) || (rest == -1 && byte & 0x40 != 0);
        if !sign_ok {
            return Err(LinkError::Unsupported(format!(
                "wasm relocation value {value} does not fit its {width}-byte SLEB site"
            )));
        }
        *slot = byte;
    }
    Ok(())
}

fn read_leb_width(bytes: &[u8], offset: usize) -> Result<usize, LinkError> {
    let mut reader = Reader::new(bytes, "relocation site");
    reader.pos = offset;
    reader.uleb()?;
    Ok(reader.pos - offset)
}

fn write_u32_le(bytes: &mut [u8], offset: usize, value: u32) -> Result<(), LinkError> {
    let end = offset + 4;
    if end > bytes.len() {
        return Err(LinkError::Unsupported(
            "wasm 32-bit relocation site out of range".to_string(),
        ));
    }
    bytes[offset..end].copy_from_slice(&value.to_le_bytes());
    Ok(())
}

fn reloc_has_addend(ty: u8) -> bool {
    matches!(ty, 3 | 4 | 5 | 8 | 9 | 11 | 14 | 15 | 16 | 17 | 21 | 22 | 23 | 27)
}

pub(crate) fn add_uleb(out: &mut Vec<u8>, mut value: u32) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            return;
        }
    }
}

fn add_sleb(out: &mut Vec<u8>, mut value: i64) {
    loop {
        let byte = (value & 0x7f) as u8;
        value >>= 7;
        let done = (value == 0 && byte & 0x40 == 0) || (value == -1 && byte & 0x40 != 0);
        out.push(if done { byte } else { byte | 0x80 });
        if done {
            return;
        }
    }
}

fn uleb_len(value: u32) -> u32 {
    let mut out = Vec::new();
    add_uleb(&mut out, value);
    out.len() as u32
}

fn add_name(out: &mut Vec<u8>, name: &str) {
    add_uleb(out, name.len() as u32);
    out.extend_from_slice(name.as_bytes());
}

fn add_section(out: &mut Vec<u8>, id: u8, payload: &[u8]) {
    out.push(id);
    add_uleb(out, payload.len() as u32);
    out.extend_from_slice(payload);
}

fn align_up(value: u32, align: u32) -> u32 {
    if align == 0 {
        value
    } else {
        value.wrapping_add(align - 1) & !(align - 1)
    }
}

fn read_const_expr(reader: &mut Reader<'_>) -> Result<Vec<u8>, LinkError> {
    let start = reader.pos;
    let opcode = reader.u8()?;
    match opcode {
        0x41 | 0x42 => {
            reader.sleb()?;
        }
        0x43 => {
            reader.take(4)?;
        }
        0x44 => {
            reader.take(8)?;
        }
        0x23 => {
            reader.uleb()?;
        }
        0xd0 => {
            reader.u8()?;
        }
        0xd2 => {
            reader.uleb()?;
        }
        other => {
            return Err(reader.fail(format!(
                "unsupported constant expression opcode {other:#04x}"
            )));
        }
    }
    if reader.u8()? != 0x0b {
        return Err(reader.fail("constant expression is not terminated"));
    }
    Ok(reader.bytes[start..reader.pos].to_vec())
}

fn parse_object(bytes: &[u8]) -> Result<Obj, LinkError> {
    if bytes.len() < 8 || &bytes[0..4] != WASM_MAGIC {
        return Err(LinkError::Unsupported(
            "wasm input is not a WebAssembly object".to_string(),
        ));
    }
    if bytes[4..8] != [1, 0, 0, 0] {
        return Err(LinkError::Unsupported(
            "wasm input has an unsupported binary version".to_string(),
        ));
    }
    let mut obj = Obj {
        types: Vec::new(),
        imports: Vec::new(),
        func_types: Vec::new(),
        globals: Vec::new(),
        elems: Vec::new(),
        code: Vec::new(),
        data: Vec::new(),
        symbols: Vec::new(),
        reloc_code: Vec::new(),
        reloc_data: Vec::new(),
        imported_funcs: 0,
        imported_globals: 0,
        table_min: 0,
        data_addr: 0,
        code_section_ordinal: None,
        data_section_ordinal: None,
        memory_base: None,
        table_base: None,
        body_offsets: Vec::new(),
        out_body_offsets: Vec::new(),
    };
    let mut section_ordinal: u32 = 0;
    let mut pos = 8;
    while pos < bytes.len() {
        let id = bytes[pos];
        pos += 1;
        let mut header = Reader::new(&bytes[pos..], "section header");
        let size = header.uleb()? as usize;
        pos += header.pos;
        let end = pos
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| LinkError::Unsupported("wasm object: truncated section".to_string()))?;
        let body = &bytes[pos..end];
        match id {
            SEC_TYPE => {
                let mut reader = Reader::new(body, "type section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let form = reader.u8()?;
                    if form != 0x60 {
                        return Err(reader.fail(format!("unsupported function form {form:#04x}")));
                    }
                    let params = read_valtypes(&mut reader)?;
                    let rets = read_valtypes(&mut reader)?;
                    obj.types.push(FuncType { params, rets });
                }
            }
            SEC_IMPORT => {
                let mut reader = Reader::new(body, "import section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let module = reader.name()?;
                    let name = reader.name()?;
                    let kind = reader.u8()?;
                    let import_kind = match kind {
                        0 => {
                            let ty = reader.uleb()?;
                            obj.imported_funcs += 1;
                            ImportKind::Func(ty)
                        }
                        1 => {
                            let _elem = reader.u8()?;
                            let flags = reader.uleb()?;
                            let min = reader.uleb()?;
                            if flags & 1 != 0 {
                                reader.uleb()?;
                            }
                            obj.table_min = obj.table_min.max(min);
                            ImportKind::Table { min }
                        }
                        2 => {
                            let flags = reader.uleb()?;
                            let min = reader.uleb()?;
                            if flags & 1 != 0 {
                                reader.uleb()?;
                            }
                            if min > 65536 {
                                return Err(reader.fail("imported memory is too large"));
                            }
                            ImportKind::Memory { min }
                        }
                        3 => {
                            let ty = reader.u8()?;
                            let flags = reader.uleb()?;
                            obj.imported_globals += 1;
                            ImportKind::Global {
                                ty,
                                mutable: flags & 1 != 0,
                            }
                        }
                        other => {
                            return Err(reader.fail(format!("unsupported import kind {other}")));
                        }
                    };
                    obj.imports.push(Import {
                        module,
                        name,
                        kind: import_kind,
                    });
                }
            }
            SEC_FUNCTION => {
                let mut reader = Reader::new(body, "function section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    obj.func_types.push(reader.uleb()?);
                }
            }
            SEC_TABLE => {
                let mut reader = Reader::new(body, "table section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let elem = reader.u8()?;
                    if elem != 0x70 {
                        return Err(reader.fail("only funcref tables are supported"));
                    }
                    let flags = reader.uleb()?;
                    let min = reader.uleb()?;
                    if flags & 1 != 0 {
                        reader.uleb()?;
                    }
                    obj.table_min = obj.table_min.max(min);
                }
            }
            SEC_MEMORY => {
                let mut reader = Reader::new(body, "memory section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let flags = reader.uleb()?;
                    reader.uleb()?;
                    if flags & 1 != 0 {
                        reader.uleb()?;
                    }
                }
            }
            SEC_GLOBAL => {
                let mut reader = Reader::new(body, "global section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let ty = reader.u8()?;
                    let flags = reader.u8()?;
                    let init = read_const_expr(&mut reader)?;
                    obj.globals.push(GlobalDef {
                        ty,
                        mutable: flags & 1 != 0,
                        init,
                    });
                }
            }
            SEC_ELEMENT => {
                let mut reader = Reader::new(body, "element section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let flags = reader.uleb()?;
                    match flags {
                        0 | 2 => {
                            if flags == 2 {
                                reader.uleb()?;
                                read_const_expr(&mut reader)?;
                                if reader.u8()? != 0x00 {
                                    return Err(reader.fail("only funcref element segments are supported"));
                                }
                            } else {
                                read_const_expr(&mut reader)?;
                            }
                            let len = reader.uleb()?;
                            let mut funcs = Vec::with_capacity(len as usize);
                            for _ in 0..len {
                                funcs.push(reader.uleb()?);
                            }
                            obj.elems.push(funcs);
                        }
                        other => {
                            return Err(reader.fail(format!(
                                "unsupported element segment flags {other}"
                            )));
                        }
                    }
                }
            }
            SEC_CODE => {
                obj.code_section_ordinal = Some(section_ordinal);
                let mut reader = Reader::new(body, "code section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let len = reader.uleb()? as usize;
                    obj.code.push(reader.take(len)?.to_vec());
                }
            }
            SEC_DATA => {
                obj.data_section_ordinal = Some(section_ordinal);
                let mut reader = Reader::new(body, "data section");
                let count = reader.uleb()?;
                for _ in 0..count {
                    let flags = reader.uleb()?;
                    let offset = match flags {
                        0 => Some(const_value(&read_const_expr(&mut reader)?)),
                        2 => {
                            reader.uleb()?;
                            Some(const_value(&read_const_expr(&mut reader)?))
                        }
                        other => {
                            return Err(reader.fail(format!(
                                "unsupported data segment flags {other}"
                            )));
                        }
                    };
                    let offset = offset
                        .flatten()
                        .ok_or_else(|| reader.fail("data offset is not an i32 constant"))?;
                    let len = reader.uleb()? as usize;
                    let payload_offset = reader.pos as u32;
                    let bytes = reader.take(len)?.to_vec();
                    obj.data.push(DataSegment {
                        offset,
                        bytes,
                        payload_offset,
                        assigned: 0,
                    });
                }
            }
            SEC_DATACOUNT => {
                let mut reader = Reader::new(body, "data count section");
                reader.uleb()?;
            }
            8 => {
                return Err(LinkError::Unsupported(
                    "wasm object: a relocatable object cannot carry a start section".to_string(),
                ));
            }
            0 => {
                let mut reader = Reader::new(body, "custom section");
                let name = reader.name()?;
                if name == "linking" {
                    parse_linking(&mut obj, body, reader.pos)?;
                } else if name == "reloc.CODE" {
                    obj.reloc_code = parse_relocs(&body[reader.pos..])?;
                } else if name == "reloc.DATA" {
                    obj.reloc_data = parse_relocs(&body[reader.pos..])?;
                }
            }
            _ => {}
        }
        pos = end;
        section_ordinal += 1;
    }
    Ok(obj)
}

fn const_value(expr: &[u8]) -> Option<i64> {
    let mut reader = Reader::new(expr, "constant expression");
    let opcode = reader.u8().ok()?;
    let value = match opcode {
        0x41 | 0x42 => reader.sleb().ok()?,
        _ => return None,
    };
    (reader.u8().ok()? == 0x0b).then_some(value)
}

fn read_valtypes(reader: &mut Reader<'_>) -> Result<Vec<u8>, LinkError> {
    let count = reader.uleb()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        out.push(reader.u8()?);
    }
    Ok(out)
}

fn parse_relocs(body: &[u8]) -> Result<Vec<Reloc>, LinkError> {
    let mut reader = Reader::new(body, "relocation section");
    let _target = reader.uleb()?;
    let count = reader.uleb()?;
    let mut out = Vec::with_capacity(count as usize);
    for _ in 0..count {
        let ty = reader.u8()?;
        let offset = reader.uleb()?;
        let sym = reader.uleb()?;
        let addend = if reloc_has_addend(ty) { reader.sleb()? } else { 0 };
        out.push(Reloc {
            ty,
            offset,
            sym,
            addend,
        });
    }
    if !reader.done() {
        return Err(LinkError::Unsupported(
            "wasm object: trailing bytes in a relocation section".to_string(),
        ));
    }
    Ok(out)
}

fn parse_linking(obj: &mut Obj, body: &[u8], start: usize) -> Result<(), LinkError> {
    let mut reader = Reader::new(body, "linking section");
    reader.pos = start;
    let version = reader.uleb()?;
    if version != 2 {
        return Err(LinkError::Unsupported(format!(
            "wasm object: unsupported linking metadata version {version}"
        )));
    }
    while !reader.done() {
        let id = reader.u8()?;
        let size = reader.uleb()? as usize;
        let sub = reader.take(size)?;
        if id == 8 {
            parse_symbols(obj, sub)?;
        }
    }
    Ok(())
}

fn parse_symbols(obj: &mut Obj, body: &[u8]) -> Result<(), LinkError> {
    let mut reader = Reader::new(body, "symbol table");
    let count = reader.uleb()?;
    for _ in 0..count {
        let kind = reader.u8()?;
        let flags = reader.uleb()?;
        match kind {
            SYM_KIND_FUNCTION | SYM_KIND_GLOBAL | SYM_KIND_TABLE => {
                let index = reader.uleb()?;
                let explicit = flags & SYM_UNDEFINED == 0 || flags & SYM_EXPLICIT_NAME != 0;
                let mut name = if explicit { reader.name()? } else { String::new() };
                let mut module = String::new();
                if flags & SYM_UNDEFINED != 0 {
                    let (import_module, import_name) = import_for(obj, kind, index)?;
                    module = import_module;
                    name = import_name;
                }
                obj.symbols.push(Symbol {
                    kind,
                    flags,
                    index,
                    name,
                    module,
                    data_offset: 0,
                });
            }
            SYM_KIND_DATA => {
                let name = reader.name()?;
                if flags & SYM_UNDEFINED != 0 {
                    obj.symbols.push(Symbol {
                        kind,
                        flags,
                        index: 0,
                        name,
                        module: String::new(),
                        data_offset: 0,
                    });
                } else {
                    let segment = reader.uleb()?;
                    let offset = reader.uleb()?;
                    let _size = reader.uleb()?;
                    obj.symbols.push(Symbol {
                        kind,
                        flags,
                        index: segment,
                        name,
                        module: String::new(),
                        data_offset: offset,
                    });
                }
            }
            SYM_KIND_SECTION => {
                let section = reader.uleb()?;
                obj.symbols.push(Symbol {
                    kind,
                    flags,
                    index: section,
                    name: String::new(),
                    module: String::new(),
                    data_offset: 0,
                });
            }
            other => {
                return Err(LinkError::Unsupported(format!(
                    "wasm object: unsupported symbol kind {other}"
                )));
            }
        }
    }
    Ok(())
}

fn import_for(obj: &Obj, kind: u8, index: u32) -> Result<(String, String), LinkError> {
    let mut cursor = 0u32;
    for import in &obj.imports {
        let matches = matches!(
            (kind, &import.kind),
            (SYM_KIND_FUNCTION, ImportKind::Func(_))
                | (SYM_KIND_GLOBAL, ImportKind::Global { .. })
                | (SYM_KIND_TABLE, ImportKind::Table { .. })
        );
        if !matches {
            continue;
        }
        if cursor == index {
            return Ok((import.module.clone(), import.name.clone()));
        }
        cursor += 1;
    }
    Err(LinkError::Unsupported(format!(
        "wasm object: symbol references import index {index} outside the import list"
    )))
}

fn archive_members(bytes: &[u8]) -> Result<Vec<Vec<u8>>, LinkError> {
    if bytes.len() < 8 || &bytes[..8] != b"!<arch>\n" {
        return Err(LinkError::Unsupported(
            "archive input is not an `ar` archive".to_string(),
        ));
    }
    let mut out = Vec::new();
    let mut pos = 8usize;
    let mut long_names: Vec<u8> = Vec::new();
    while pos + 60 <= bytes.len() {
        let header = &bytes[pos..pos + 60];
        let raw_name = String::from_utf8_lossy(&header[0..16]).trim().to_string();
        let size_field = String::from_utf8_lossy(&header[48..58]).trim().to_string();
        let size: usize = size_field.parse().map_err(|_| {
            LinkError::Unsupported("archive member size is not a number".to_string())
        })?;
        let start = pos + 60;
        let end = start
            .checked_add(size)
            .filter(|end| *end <= bytes.len())
            .ok_or_else(|| LinkError::Unsupported("archive member is truncated".to_string()))?;
        let data = &bytes[start..end];
        if raw_name == "//" {
            long_names = data.to_vec();
        } else if raw_name == "/" || raw_name == "/SYM64/" {
        } else if let Some(offset) = raw_name
            .strip_prefix('/')
            .and_then(|rest| rest.parse::<usize>().ok())
        {
            if let Some(tail) = long_names.get(offset..)
                && let Some(stop) = tail.windows(2).position(|window| window == b"/\n")
                && data.starts_with(WASM_MAGIC)
            {
                let _name = &tail[..stop];
                out.push(data.to_vec());
            }
        } else if data.starts_with(WASM_MAGIC) {
            out.push(data.to_vec());
        }
        pos = end + (size & 1);
    }
    Ok(out)
}

fn type_key(value: &FuncType) -> (Vec<u8>, Vec<u8>) {
    (value.params.clone(), value.rets.clone())
}

struct Plan {
    objects: Vec<Obj>,
    labels: Vec<String>,
    imports: Vec<Import>,
    import_func_slots: BTreeMap<(String, String), u32>,
    import_global_slots: BTreeMap<(String, String), u32>,
    defs: BTreeMap<String, (usize, usize)>,
    func_bases: Vec<u32>,
    global_bases: Vec<u32>,
    imported_func_map: Vec<BTreeMap<u32, u32>>,
    imported_global_map: Vec<BTreeMap<u32, u32>>,
    type_map: Vec<Vec<u32>>,
    types: Vec<FuncType>,
    table_slots: BTreeMap<(usize, u32), u32>,
    table_funcs: Vec<u32>,
    table_size: u32,
    data_image: Vec<u8>,
    data_end: u32,
    stack_top: u32,
    heap_base: u32,
    stack_pointer: u32,
    synthetic_globals: u32,
    start_function: u32,
    func_type_of_output: Vec<u32>,
}

impl Plan {
    fn fail(&self, message: impl Into<String>) -> LinkError {
        LinkError::Unsupported(format!("wasm link: {}", message.into()))
    }

    fn resolve(&self, obj: usize, sym: usize) -> Result<Target, LinkError> {
        let symbol = &self.objects[obj].symbols[sym];
        if symbol.is_undefined() {
            return self.resolve_undefined(obj, symbol);
        }
        match symbol.kind {
            SYM_KIND_FUNCTION => {
                let index = symbol.index;
                if index < self.objects[obj].imported_funcs {
                    let mapped = self.imported_func_map[obj]
                        .get(&index)
                        .copied()
                        .ok_or_else(|| self.fail("function import without an output slot"))?;
                    Ok(Target::Func(mapped))
                } else {
                    Ok(Target::Func(
                        self.func_bases[obj] + (index - self.objects[obj].imported_funcs),
                    ))
                }
            }
            SYM_KIND_GLOBAL => {
                let index = symbol.index;
                if index < self.objects[obj].imported_globals {
                    let mapped = self.imported_global_map[obj]
                        .get(&index)
                        .copied()
                        .ok_or_else(|| self.fail("global import without an output slot"))?;
                    Ok(Target::Global(mapped))
                } else {
                    Ok(Target::Global(
                        self.global_bases[obj] + (index - self.objects[obj].imported_globals),
                    ))
                }
            }
            SYM_KIND_DATA => {
                let segment = self.objects[obj]
                    .data
                    .get(symbol.index as usize)
                    .ok_or_else(|| self.fail("data symbol references a missing segment"))?;
                let address = segment
                    .assigned
                    .checked_add(symbol.data_offset)
                    .ok_or_else(|| self.fail("data symbol address overflow"))?;
                Ok(Target::Address(address))
            }
            SYM_KIND_TABLE => Ok(Target::Table(0)),
            SYM_KIND_SECTION => {
                let object = &self.objects[obj];
                if Some(symbol.index) == object.code_section_ordinal {
                    Ok(Target::CodeOffset(0))
                } else if Some(symbol.index) == object.data_section_ordinal {
                    Ok(Target::DataOffset(0))
                } else {
                    Err(self.fail("section symbol outside the code or data section"))
                }
            }
            other => Err(self.fail(format!("unsupported symbol kind {other}"))),
        }
    }

    fn resolve_undefined(&self, obj: usize, symbol: &Symbol) -> Result<Target, LinkError> {
        if symbol.is_external_module() {
            return self.external_slot(obj, symbol);
        }
        match (symbol.kind, symbol.name.as_str()) {
            (_, "__stack_pointer") => Ok(Target::Global(self.stack_pointer)),
            (_, "__heap_base") => Ok(Target::Address(self.heap_base)),
            (_, "__data_end") => Ok(Target::Address(self.data_end)),
            (_, "__memory_base") => self
                .objects[obj]
                .memory_base
                .map(Target::Global)
                .ok_or_else(|| self.fail("`__memory_base` used without a synthesized global")),
            (_, "__table_base") => self
                .objects[obj]
                .table_base
                .map(Target::Global)
                .ok_or_else(|| self.fail("`__table_base` used without a synthesized global")),
            (SYM_KIND_TABLE, _) => Ok(Target::Table(0)),
            _ => self
                .defs
                .get(&symbol.name)
                .map(|(object, sym)| self.resolve(*object, *sym))
                .unwrap_or_else(|| {
                    Err(self.fail(format!(
                        "undefined symbol `{}` needed by `{}`",
                        symbol.name, self.labels[obj]
                    )))
                }),
        }
    }

    fn external_slot(&self, obj: usize, symbol: &Symbol) -> Result<Target, LinkError> {
        match symbol.kind {
            SYM_KIND_FUNCTION => self
                .import_func_slots
                .get(&(symbol.module.clone(), symbol.name.clone()))
                .copied()
                .map(Target::Func)
                .ok_or_else(|| {
                    self.fail(format!(
                        "import `{}.{}` from `{}` has no slot",
                        symbol.module, symbol.name, self.labels[obj]
                    ))
                }),
            SYM_KIND_GLOBAL => self
                .import_global_slots
                .get(&(symbol.module.clone(), symbol.name.clone()))
                .copied()
                .map(Target::Global)
                .ok_or_else(|| {
                    self.fail(format!(
                        "import `{}.{}` from `{}` has no slot",
                        symbol.module, symbol.name, self.labels[obj]
                    ))
                }),
            SYM_KIND_TABLE => Ok(Target::Table(0)),
            _ => Err(self.fail("external data imports are unsupported")),
        }
    }

    fn patch_leb(
        &self,
        bytes: &mut [u8],
        offset: usize,
        value: i64,
        signed: bool,
    ) -> Result<(), LinkError> {
        let width = read_leb_width(bytes, offset)?;
        let end = offset
            .checked_add(width)
            .ok_or_else(|| self.fail("relocation site overflow"))?;
        let site = bytes
            .get_mut(offset..end)
            .ok_or_else(|| self.fail("relocation site past the end of a section"))?;
        if signed {
            write_sleb_width(site, value).map_err(|e| match e {
                LinkError::Unsupported(message) => LinkError::Unsupported(format!(
                    "{message} (offset {offset}, width {width})"
                )),
                other => other,
            })
        } else {
            if value < 0 || value > u32::MAX as i64 {
                return Err(self.fail(format!("relocation value {value} is out of range")));
            }
            write_uleb_width(site, value as u32)
        }
    }

    fn apply_reloc(
        &self,
        obj: usize,
        reloc: &Reloc,
        section: &mut [u8],
        site: usize,
        memory_relative: bool,
    ) -> Result<(), LinkError> {
        let target = self.resolve(obj, reloc.sym as usize)?;
        match reloc.ty {
            R_FUNCTION_INDEX_LEB | R_FUNCTION_INDEX_I32 => {
                let Target::Func(index) = target else {
                    return Err(self.fail("function index relocation against a non-function"));
                };
                let value = index as i64 + reloc.addend;
                if reloc.ty == R_FUNCTION_INDEX_I32 {
                    write_u32_le(section, site, value as u32)?;
                } else {
                    self.patch_leb(section, site, value, false)?;
                }
            }
            // LLVM stores the object-local type index in the relocation's
            // index field for this type instead of a symbol-table index;
            // verified against every call_indirect site in the runtime
            // archive.
            R_TYPE_INDEX_LEB => {
                let mapped = self.type_map[obj]
                    .get(reloc.sym as usize)
                    .copied()
                    .ok_or_else(|| self.fail("type index outside the object's type section"))?;
                self.patch_leb(section, site, mapped as i64 + reloc.addend, false)?;
            }
            R_TABLE_INDEX_SLEB | R_TABLE_INDEX_I32 | R_TABLE_INDEX_REL_SLEB => {
                let index = match target {
                    Target::Func(_) => self.table_slot_for_symbol(obj, reloc.sym as usize)?,
                    Target::Table(0) => 0,
                    _ => return Err(self.fail("table index relocation against a non-function")),
                };
                let value = index as i64 + reloc.addend;
                if reloc.ty == R_TABLE_INDEX_I32 {
                    write_u32_le(section, site, value as u32)?;
                } else {
                    self.patch_leb(section, site, value, true)?;
                }
            }
            R_TABLE_NUMBER_LEB => {
                self.patch_leb(section, site, 0, false)?;
            }
            R_MEMORY_ADDR_LEB | R_MEMORY_ADDR_SLEB | R_MEMORY_ADDR_I32 | R_MEMORY_ADDR_REL_SLEB => {
                let Target::Address(address) = target else {
                    return Err(self.fail("memory relocation against a non-data symbol"));
                };
                let mut value = address as i64 + reloc.addend;
                if reloc.ty == R_MEMORY_ADDR_REL_SLEB {
                    value -= self.objects[obj].data_addr as i64;
                }
                if reloc.ty == R_MEMORY_ADDR_I32 {
                    write_u32_le(section, site, value as u32)?;
                } else {
                    self.patch_leb(section, site, value, true)?;
                }
            }
            R_GLOBAL_INDEX_LEB | R_GLOBAL_INDEX_I32 => {
                let Target::Global(index) = target else {
                    return Err(self.fail("global index relocation against a non-global"));
                };
                let value = index as i64 + reloc.addend;
                if reloc.ty == R_GLOBAL_INDEX_I32 {
                    write_u32_le(section, site, value as u32)?;
                } else {
                    self.patch_leb(section, site, value, false)?;
                }
            }
            R_FUNCTION_OFFSET_I32 => {
                let Target::Func(index) = target else {
                    return Err(self.fail("function offset relocation against a non-function"));
                };
                let offset = self.function_offset(obj, index)?;
                write_u32_le(section, site, (offset as i64 + reloc.addend) as u32)?;
            }
            R_SECTION_OFFSET_I32 => {
                let base = match target {
                    Target::CodeOffset(base) | Target::DataOffset(base) => base,
                    Target::Address(address) if memory_relative => address,
                    _ => return Err(self.fail("section offset relocation against a symbol")),
                };
                write_u32_le(section, site, (base as i64 + reloc.addend) as u32)?;
            }
            other => {
                return Err(self.fail(format!(
                    "unsupported relocation type {other} in `{}`",
                    self.labels[obj]
                )));
            }
        }
        Ok(())
    }

    fn table_slot_for_symbol(&self, obj: usize, sym: usize) -> Result<u32, LinkError> {
        let symbol = &self.objects[obj].symbols[sym];
        if symbol.is_undefined() {
            if let Some((object, definition)) = self.defs.get(&symbol.name) {
                let defined = &self.objects[*object].symbols[*definition];
                return self.slot_of(*object, defined.index);
            }
            return Err(self.fail(format!(
                "table index relocation against the unresolved symbol `{}`",
                symbol.name
            )));
        }
        self.slot_of(obj, symbol.index)
    }

    fn slot_of(&self, obj: usize, func: u32) -> Result<u32, LinkError> {
        self.table_slots.get(&(obj, func)).copied().ok_or_else(|| {
            let name = self.objects[obj]
                .symbols
                .iter()
                .find(|symbol| !symbol.is_undefined() && symbol.index == func)
                .map(|symbol| symbol.name.clone())
                .unwrap_or_else(|| format!("index {func}"));
            self.fail(format!(
                "a function in `{}` has no table slot: `{name}`",
                self.labels[obj]
            ))
        })
    }

    fn function_offset(&self, obj: usize, index: u32) -> Result<u32, LinkError> {
        let local = index
            .checked_sub(self.func_bases[obj])
            .ok_or_else(|| self.fail("function offset relocation against an imported function"))?;
        self.objects[obj]
            .out_body_offsets
            .get(local as usize)
            .copied()
            .ok_or_else(|| self.fail("function offset outside the object"))
    }
}

struct PlanView<'a> {
    plan: &'a Plan,
}

#[derive(Clone, Copy)]
enum Target {
    Func(u32),
    Global(u32),
    Table(u32),
    Address(u32),
    CodeOffset(u32),
    DataOffset(u32),
}

/// Link the inputs into one WASI command module and write it to `output_path`.
pub fn link(inputs: &[WasmInput], output_path: &Path) -> Result<(), LinkError> {
    let bytes = link_bytes(inputs)?;
    std::fs::write(output_path, bytes).map_err(|e| LinkError::Io(e.to_string()))
}

/// Link the inputs and return the module bytes.
pub fn link_bytes(inputs: &[WasmInput]) -> Result<Vec<u8>, LinkError> {
    let mut objects: Vec<Obj> = Vec::new();
    let mut labels: Vec<String> = Vec::new();
    let mut archives: Vec<Vec<Vec<u8>>> = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        match input {
            WasmInput::Object(bytes) => {
                objects.push(parse_object(bytes)?);
                labels.push(format!("object{index}"));
            }
            WasmInput::ObjectPath(path) => {
                let bytes = std::fs::read(path)
                    .map_err(|e| LinkError::Io(format!("cannot read {}: {e}", path.display())))?;
                objects.push(parse_object(&bytes)?);
                labels.push(path.display().to_string());
            }
            WasmInput::Archive(bytes) => archives.push(archive_members(bytes)?),
            WasmInput::ArchivePath(path) => {
                let bytes = std::fs::read(path)
                    .map_err(|e| LinkError::Io(format!("cannot read {}: {e}", path.display())))?;
                archives.push(archive_members(&bytes)?);
            }
        }
    }
    if objects.is_empty() {
        return Err(LinkError::Unsupported(
            "wasm link: no object inputs".to_string(),
        ));
    }

    let mut archive_index: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    for (archive, members) in archives.iter().enumerate() {
        for (member, bytes) in members.iter().enumerate() {
            let object = parse_object(bytes)?;
            for symbol in &object.symbols {
                if symbol.is_undefined() || symbol.name.is_empty() {
                    continue;
                }
                archive_index
                    .entry(symbol.name.clone())
                    .or_insert((archive, member));
            }
        }
    }

    let mut defs: BTreeMap<String, (usize, usize)> = BTreeMap::new();
    add_definitions(&objects, 0, &mut defs)?;
    let mut pulled: BTreeSet<(usize, usize)> = BTreeSet::new();
    let mut required: Vec<String> = vec!["_start".to_string()];
    loop {
        required.extend(undefined_names(&objects));
        required.sort();
        required.dedup();
        let mut progress = false;
        for name in std::mem::take(&mut required) {
            if defs.contains_key(&name) {
                continue;
            }
            let Some(&(archive, member)) = archive_index.get(&name) else {
                continue;
            };
            if !pulled.insert((archive, member)) {
                continue;
            }
            let object = parse_object(&archives[archive][member])?;
            objects.push(object);
            let index = objects.len() - 1;
            labels.push(format!("archive{archive}:{member}"));
            add_definitions(&objects[index..=index], index, &mut defs)?;
            progress = true;
        }
        if !progress {
            break;
        }
    }

    let mut plan = build_plan(objects, labels, defs)?;
    emit(&mut plan)
}

/// Record the definitions an object adds. A later strong definition
/// replaces an earlier weak one; two strong definitions of the same name
/// are a hard error, which is how a duplicated archive member or a
/// mismatched runtime archive surfaces.
fn add_definitions(
    objects: &[Obj],
    base: usize,
    defs: &mut BTreeMap<String, (usize, usize)>,
) -> Result<(), LinkError> {
    for (offset, object) in objects.iter().enumerate() {
        let object_index = base + offset;
        for (sym_index, symbol) in object.symbols.iter().enumerate() {
            if symbol.is_undefined() || symbol.name.is_empty() {
                continue;
            }
            match defs.get(&symbol.name).copied() {
                None => {
                    defs.insert(symbol.name.clone(), (object_index, sym_index));
                }
                Some((previous_object, previous_symbol)) => {
                    let previous = objects
                        .get(previous_object.checked_sub(base).unwrap_or(usize::MAX))
                        .map(|object| object.symbols[previous_symbol].is_weak())
                        .unwrap_or(false);
                    if previous && !symbol.is_weak() {
                        defs.insert(symbol.name.clone(), (object_index, sym_index));
                    } else if !previous && !symbol.is_weak() {
                        return Err(LinkError::Unsupported(format!(
                            "wasm link: duplicate definition of `{}`",
                            symbol.name
                        )));
                    }
                }
            }
        }
    }
    Ok(())
}

fn undefined_symbol(object: &Obj, kind: u8, index: u32) -> Option<&Symbol> {
    object
        .symbols
        .iter()
        .find(|symbol| symbol.is_undefined() && symbol.kind == kind && symbol.index == index)
}

fn is_synthetic(name: &str) -> bool {
    matches!(
        name,
        "__stack_pointer" | "__heap_base" | "__data_end" | "__memory_base" | "__table_base"
    )
}

fn undefined_names(objects: &[Obj]) -> Vec<String> {
    let mut out = Vec::new();
    for object in objects {
        for symbol in &object.symbols {
            if symbol.is_undefined() && !symbol.is_external_module() && !symbol.name.is_empty() {
                out.push(symbol.name.clone());
            }
        }
    }
    out
}

fn build_plan(
    mut objects: Vec<Obj>,
    labels: Vec<String>,
    defs: BTreeMap<String, (usize, usize)>,
) -> Result<Plan, LinkError> {
    if objects.len() != labels.len() {
        return Err(LinkError::Unsupported(
            "wasm link: internal label mismatch".to_string(),
        ));
    }
    let mut types: Vec<FuncType> = Vec::new();
    let mut type_keys: BTreeMap<(Vec<u8>, Vec<u8>), u32> = BTreeMap::new();
    let mut type_map: Vec<Vec<u32>> = Vec::with_capacity(objects.len());
    for object in &objects {
        let mut map = Vec::with_capacity(object.types.len());
        for value in &object.types {
            let key = type_key(value);
            let index = match type_keys.get(&key) {
                Some(index) => *index,
                None => {
                    let index = types.len() as u32;
                    type_keys.insert(key, index);
                    types.push(value.clone());
                    index
                }
            };
            map.push(index);
        }
        type_map.push(map);
    }

    let mut plan = Plan {
        objects: Vec::new(),
        labels,
        imports: Vec::new(),
        import_func_slots: BTreeMap::new(),
        import_global_slots: BTreeMap::new(),
        defs: BTreeMap::new(),
        func_bases: Vec::new(),
        global_bases: Vec::new(),
        imported_func_map: Vec::new(),
        imported_global_map: Vec::new(),
        type_map,
        types,
        table_slots: BTreeMap::new(),
        table_funcs: Vec::new(),
        table_size: 0,
        data_image: Vec::new(),
        data_end: DATA_BASE,
        stack_top: DATA_BASE,
        heap_base: DATA_BASE,
        stack_pointer: 0,
        synthetic_globals: 0,
        start_function: 0,
        func_type_of_output: Vec::new(),
    };

    for (object_index, object) in objects.iter().enumerate() {
        let mut func_map: BTreeMap<u32, u32> = BTreeMap::new();
        let mut cursor = 0u32;
        for import in &object.imports {
            let ImportKind::Func(ty) = &import.kind else {
                continue;
            };
            let symbol = undefined_symbol(object, SYM_KIND_FUNCTION, cursor);
            let keep = match symbol {
                Some(symbol) if symbol.is_external_module() => true,
                Some(symbol) => {
                    if !defs.contains_key(&symbol.name) && !is_synthetic(&symbol.name) {
                        return Err(LinkError::Unsupported(format!(
                            "wasm link: undefined symbol `{}` needed by object{}",
                            symbol.name, object_index
                        )));
                    }
                    false
                }
                None => false,
            };
            cursor += 1;
            if !keep {
                continue;
            }
            let key = (import.module.clone(), import.name.clone());
            let slot = match plan.import_func_slots.get(&key) {
                Some(slot) => *slot,
                None => {
                    let slot = plan.import_func_slots.len() as u32;
                    let mapped_type = plan.type_map[object_index]
                        .get(*ty as usize)
                        .copied()
                        .ok_or_else(|| {
                            LinkError::Unsupported(
                                "wasm link: import type outside the object's type section"
                                    .to_string(),
                            )
                        })?;
                    plan.imports.push(Import {
                        module: import.module.clone(),
                        name: import.name.clone(),
                        kind: ImportKind::Func(mapped_type),
                    });
                    plan.import_func_slots.insert(key, slot);
                    slot
                }
            };
            func_map.insert(cursor - 1, slot);
        }
        plan.imported_func_map.push(func_map);
    }
    let imported_funcs = plan.import_func_slots.len() as u32;

    for object in &objects {
        let mut global_map: BTreeMap<u32, u32> = BTreeMap::new();
        let mut cursor = 0u32;
        for import in &object.imports {
            let ImportKind::Global { ty, mutable } = &import.kind else {
                continue;
            };
            let symbol = undefined_symbol(object, SYM_KIND_GLOBAL, cursor);
            let keep = match symbol {
                Some(symbol) if symbol.is_external_module() => true,
                Some(symbol) => {
                    if !is_synthetic(&symbol.name) && !defs.contains_key(&symbol.name) {
                        return Err(LinkError::Unsupported(format!(
                            "wasm link: undefined global `{}`",
                            symbol.name
                        )));
                    }
                    false
                }
                None => false,
            };
            cursor += 1;
            if !keep {
                continue;
            }
            let key = (import.module.clone(), import.name.clone());
            let slot = match plan.import_global_slots.get(&key) {
                Some(slot) => *slot,
                None => {
                    let slot = plan.import_global_slots.len() as u32;
                    plan.imports.push(Import {
                        module: import.module.clone(),
                        name: import.name.clone(),
                        kind: ImportKind::Global {
                            ty: *ty,
                            mutable: *mutable,
                        },
                    });
                    plan.import_global_slots.insert(key, slot);
                    slot
                }
            };
            global_map.insert(cursor - 1, slot);
        }
        plan.imported_global_map.push(global_map);
    }
    let imported_globals = plan.import_global_slots.len() as u32;

    plan.stack_pointer = imported_globals;
    let mut synthetic = 1u32;
    for object in &mut objects {
        let uses_memory_base = object
            .symbols
            .iter()
            .any(|symbol| symbol.is_undefined() && symbol.name == "__memory_base");
        if uses_memory_base {
            object.memory_base = Some(imported_globals + synthetic);
            synthetic += 1;
        }
        let uses_table_base = object
            .symbols
            .iter()
            .any(|symbol| symbol.is_undefined() && symbol.name == "__table_base");
        if uses_table_base {
            object.table_base = Some(imported_globals + synthetic);
            synthetic += 1;
        }
    }
    plan.synthetic_globals = synthetic;

    let mut func_base = imported_funcs;
    for object in &objects {
        plan.func_bases.push(func_base);
        func_base += object.code.len() as u32;
    }
    let start = defs
        .get("_start")
        .copied()
        .ok_or_else(|| {
            LinkError::Unsupported(
                "wasm link: no `_start` entry; the WASI runtime archive must define it".to_string(),
            )
        })?;
    if objects[start.0].symbols[start.1].kind != SYM_KIND_FUNCTION {
        return Err(LinkError::Unsupported(
            "wasm link: `_start` is not a function".to_string(),
        ));
    }
    let start_symbol = &objects[start.0].symbols[start.1];
    plan.start_function = if start_symbol.index < objects[start.0].imported_funcs {
        plan.imported_func_map[start.0]
            .get(&start_symbol.index)
            .copied()
            .ok_or_else(|| {
                LinkError::Unsupported(
                    "wasm link: `_start` resolves to an import without a slot".to_string(),
                )
            })?
    } else {
        plan.func_bases[start.0] + (start_symbol.index - objects[start.0].imported_funcs)
    };


    let mut code_size = uleb_len(objects.iter().map(|object| object.code.len() as u32).sum());
    for object in &mut objects {
        object.body_offsets.clear();
        object.out_body_offsets.clear();
        let mut source_size = uleb_len(object.code.len() as u32);
        for body in &object.code {
            object
                .body_offsets
                .push(source_size + uleb_len(body.len() as u32));
            object
                .out_body_offsets
                .push(code_size + uleb_len(body.len() as u32));
            source_size += uleb_len(body.len() as u32) + body.len() as u32;
            code_size += uleb_len(body.len() as u32) + body.len() as u32;
        }
    }

    let mut func_type_of_output: Vec<u32> = vec![0; func_base as usize];
    for slot in plan.import_func_slots.values() {
        if let Some(import) = plan.imports.get(*slot as usize)
            && let ImportKind::Func(ty) = import.kind
        {
            func_type_of_output[*slot as usize] = ty;
        }
    }
    for (object_index, object) in objects.iter().enumerate() {
        for (local, ty) in object.func_types.iter().enumerate() {
            let index = (plan.func_bases[object_index] + local as u32) as usize;
            let mapped = plan.type_map[object_index]
                .get(*ty as usize)
                .copied()
                .unwrap_or(0);
            if index < func_type_of_output.len() {
                func_type_of_output[index] = mapped;
            }
        }
    }
    plan.func_type_of_output = func_type_of_output;

    let mut global_base = imported_globals + plan.synthetic_globals;
    for object in &objects {
        plan.global_bases.push(global_base);
        global_base += object.globals.len() as u32;
    }

    let mut slot = TABLE_SLOT_BASE;
    let mut table_min = 0u32;
    let slot_of_func = |object_index: usize, func: u32| -> Result<u32, LinkError> {
        if func >= objects[object_index].imported_funcs {
            return Ok(
                plan.func_bases[object_index] + (func - objects[object_index].imported_funcs),
            );
        }
        let object = &objects[object_index];
        let Some(symbol) = undefined_symbol(object, SYM_KIND_FUNCTION, func) else {
            return Err(LinkError::Unsupported(
                "wasm link: table entry names a function import without a symbol".to_string(),
            ));
        };
        if symbol.is_external_module() {
            return plan.imported_func_map[object_index]
                .get(&func)
                .copied()
                .ok_or_else(|| {
                    LinkError::Unsupported(
                        "wasm link: table entry names a missing import".to_string(),
                    )
                });
        }
        let Some((owner, definition)) = defs.get(&symbol.name) else {
            return Err(LinkError::Unsupported(format!(
                "wasm link: table entry names the undefined function `{}`",
                symbol.name
            )));
        };
        let defined = &objects[*owner].symbols[*definition];
        Ok(plan.func_bases[*owner] + (defined.index - objects[*owner].imported_funcs))
    };
    for (object_index, object) in objects.iter().enumerate() {
        table_min = table_min.max(object.table_min);
        for segment in &object.elems {
            for func in segment {
                let key = (object_index, *func);
                if plan.table_slots.contains_key(&key) {
                    continue;
                }
                let index = slot_of_func(object_index, *func)?;
                plan.table_slots.insert(key, slot);
                plan.table_funcs.push(index);
                slot += 1;
            }
        }
    }
    for (object_index, object) in objects.iter().enumerate() {
        let relocs: Vec<Reloc> = object
            .reloc_code
            .iter()
            .chain(object.reloc_data.iter())
            .copied()
            .collect();
        for reloc in relocs {
            if !matches!(
                reloc.ty,
                R_TABLE_INDEX_SLEB | R_TABLE_INDEX_I32 | R_TABLE_INDEX_REL_SLEB
            ) {
                continue;
            }
            let symbol = &object.symbols[reloc.sym as usize];
            if symbol.kind != SYM_KIND_FUNCTION {
                continue;
            }
            let (owner, local) = if symbol.is_undefined() {
                let Some((owner, definition)) = defs.get(&symbol.name) else {
                    continue;
                };
                let defined = &objects[*owner].symbols[*definition];
                (*owner, defined.index)
            } else {
                (object_index, symbol.index)
            };
            let key = (owner, local);
            if plan.table_slots.contains_key(&key) {
                continue;
            }
            let index = slot_of_func(owner, local)?;
            plan.table_slots.insert(key, slot);
            plan.table_funcs.push(index);
            slot += 1;
        }
    }
    plan.table_size = if slot == TABLE_SLOT_BASE {
        table_min
    } else {
        slot.max(table_min)
    };

    let mut cursor = DATA_BASE;
    for object in &mut objects {
        object.data_addr = align_up(cursor, 16);
        let mut image_size = 0u32;
        for segment in &mut object.data {
            let offset = segment.offset.max(0) as u32;
            segment.assigned = object.data_addr.saturating_add(offset);
            image_size = image_size.max(offset.saturating_add(segment.bytes.len() as u32));
        }
        cursor = object.data_addr.saturating_add(image_size);
    }
    plan.data_end = align_up(cursor, 16);
    plan.stack_top = plan.data_end + STACK_SIZE;
    plan.heap_base = align_up(plan.stack_top, 16);

    plan.defs = defs;
    let image_len = (plan.data_end - DATA_BASE) as usize;
    plan.data_image = vec![0u8; image_len];
    for object in &objects {
        for segment in &object.data {
            let at = (segment.assigned - DATA_BASE) as usize;
            if at + segment.bytes.len() <= plan.data_image.len() {
                plan.data_image[at..at + segment.bytes.len()].copy_from_slice(&segment.bytes);
            }
        }
    }

    plan.objects = objects;
    Ok(plan)
}

fn emit(plan: &mut Plan) -> Result<Vec<u8>, LinkError> {
    reloc_code(plan)?;
    reloc_data(plan)?;
    encode(plan)
}

fn reloc_code(plan: &mut Plan) -> Result<(), LinkError> {
    for object_index in 0..plan.objects.len() {
        let relocs = std::mem::take(&mut plan.objects[object_index].reloc_code);
        let offsets = plan.objects[object_index].body_offsets.clone();
        let mut bodies = std::mem::take(&mut plan.objects[object_index].code);
        {
            let view = PlanView { plan: &*plan };
            for reloc in &relocs {
                let Some((body_index, start)) = offsets
                    .iter()
                    .enumerate()
                    .rev()
                    .find(|(_, start)| **start <= reloc.offset)
                else {
                    return Err(LinkError::Unsupported(format!(
                        "wasm link: relocation offset {} is outside the code section",
                        reloc.offset
                    )));
                };
                let start = *start;
                let inner = (reloc.offset - start) as usize;
                let body = bodies.get_mut(body_index).ok_or_else(|| {
                    LinkError::Unsupported("wasm link: missing code body".to_string())
                })?;
                if inner >= body.len() {
                    return Err(LinkError::Unsupported(
                        "wasm link: relocation offset is past its function body".to_string(),
                    ));
                }
                view.plan
                    .apply_reloc(object_index, reloc, body, inner, false)
                    .map_err(|e| match e {
                        LinkError::Unsupported(message) => LinkError::Unsupported(format!(
                            "{message} [code reloc type {} offset {} in {}]",
                            reloc.ty, reloc.offset, view.plan.labels[object_index]
                        )),
                        other => other,
                    })?;
            }
        }
        plan.objects[object_index].code = bodies;
        plan.objects[object_index].reloc_code = relocs;
    }
    Ok(())
}

fn reloc_data(plan: &mut Plan) -> Result<(), LinkError> {
    let mut image = std::mem::take(&mut plan.data_image);
    for object_index in 0..plan.objects.len() {
        let segments: Vec<(u32, u32, u32)> = plan.objects[object_index]
            .data
            .iter()
            .map(|segment| {
                (
                    segment.payload_offset,
                    segment.assigned - DATA_BASE,
                    segment.bytes.len() as u32,
                )
            })
            .collect();
        let relocs = std::mem::take(&mut plan.objects[object_index].reloc_data);
        {
            let view = PlanView { plan: &*plan };
            for reloc in &relocs {
                let mut matched = None;
                for (payload, base, len) in &segments {
                    if reloc.offset >= *payload && reloc.offset < payload + len {
                        matched = Some((*payload, *base));
                        break;
                    }
                }
                let Some((payload_offset, base)) = matched else {
                    return Err(LinkError::Unsupported(format!(
                        "wasm link: data relocation offset {} is outside every segment",
                        reloc.offset
                    )));
                };
                let site = base as usize + (reloc.offset - payload_offset) as usize;
                view.plan
                    .apply_reloc(object_index, reloc, &mut image[..], site, true)
                    .map_err(|e| match e {
                        LinkError::Unsupported(message) => LinkError::Unsupported(format!(
                            "{message} [data reloc type {} offset {} in {}]",
                            reloc.ty, reloc.offset, view.plan.labels[object_index]
                        )),
                        other => other,
                    })?;
            }
        }
        plan.objects[object_index].reloc_data = relocs;
    }
    plan.data_image = image;
    Ok(())
}

fn encode(plan: &Plan) -> Result<Vec<u8>, LinkError> {
    let mut out = Vec::new();
    out.extend_from_slice(WASM_MAGIC);
    out.extend_from_slice(&[1, 0, 0, 0]);

    let mut types = Vec::new();
    add_uleb(&mut types, plan.types.len() as u32);
    for value in &plan.types {
        types.push(0x60);
        add_uleb(&mut types, value.params.len() as u32);
        types.extend_from_slice(&value.params);
        add_uleb(&mut types, value.rets.len() as u32);
        types.extend_from_slice(&value.rets);
    }
    if !plan.types.is_empty() {
        add_section(&mut out, SEC_TYPE, &types);
    }

    if !plan.imports.is_empty() {
        let mut imports = Vec::new();
        add_uleb(&mut imports, plan.imports.len() as u32);
        for import in &plan.imports {
            add_name(&mut imports, &import.module);
            add_name(&mut imports, &import.name);
            match &import.kind {
                ImportKind::Func(ty) => {
                    imports.push(0);
                    add_uleb(&mut imports, *ty);
                }
                ImportKind::Global { ty, mutable } => {
                    imports.push(3);
                    imports.push(*ty);
                    imports.push(u8::from(*mutable));
                }
                ImportKind::Table { min } => {
                    imports.push(1);
                    imports.push(0x70);
                    add_uleb(&mut imports, 0);
                    add_uleb(&mut imports, *min);
                }
                ImportKind::Memory { min } => {
                    imports.push(2);
                    add_uleb(&mut imports, 0);
                    add_uleb(&mut imports, *min);
                }
            }
        }
        add_section(&mut out, SEC_IMPORT, &imports);
    }

    let mut functions = Vec::new();
    let total: u32 = plan.objects.iter().map(|object| object.code.len() as u32).sum();
    add_uleb(&mut functions, total);
    for (object_index, object) in plan.objects.iter().enumerate() {
        for ty in &object.func_types {
            let mapped = plan.type_map[object_index]
                .get(*ty as usize)
                .copied()
                .unwrap_or(0);
            add_uleb(&mut functions, mapped);
        }
    }
    if total > 0 {
        add_section(&mut out, SEC_FUNCTION, &functions);
    }

    if plan.table_size > 0 {
        let mut table = Vec::new();
        add_uleb(&mut table, 1);
        table.push(0x70);
        add_uleb(&mut table, 0);
        add_uleb(&mut table, plan.table_size);
        add_section(&mut out, SEC_TABLE, &table);
    }

    let pages = align_up(plan.heap_base.saturating_add(PAGE_SIZE), PAGE_SIZE) / PAGE_SIZE;
    let mut memory = Vec::new();
    add_uleb(&mut memory, 1);
    add_uleb(&mut memory, 0);
    add_uleb(&mut memory, pages.max(2));
    add_section(&mut out, SEC_MEMORY, &memory);

    let mut globals = Vec::new();
    add_uleb(&mut globals, plan.synthetic_globals + total_globals(plan));
    globals.push(0x7f);
    globals.push(1);
    globals.push(0x41);
    add_sleb(&mut globals, plan.stack_top as i64);
    globals.push(0x0b);
    for object in &plan.objects {
        if object.memory_base.is_some() {
            globals.push(0x7f);
            globals.push(0);
            globals.push(0x41);
            add_sleb(&mut globals, object.data_addr as i64);
            globals.push(0x0b);
        }
        if object.table_base.is_some() {
            globals.push(0x7f);
            globals.push(0);
            globals.push(0x41);
            globals.push(0x00);
            globals.push(0x0b);
        }
    }
    for (object_index, object) in plan.objects.iter().enumerate() {
        for global in &object.globals {
            globals.push(global.ty);
            globals.push(u8::from(global.mutable));
            let init = translate_init(plan, object_index, &global.init).ok_or_else(|| {
                LinkError::Unsupported(
                    "wasm link: global initializer uses a construct the linker cannot translate"
                        .to_string(),
                )
            })?;
            globals.extend_from_slice(&init);
        }
    }
    add_section(&mut out, SEC_GLOBAL, &globals);

    let mut exports = Vec::new();
    add_uleb(&mut exports, 2);
    add_name(&mut exports, "memory");
    exports.push(2);
    add_uleb(&mut exports, 0);
    add_name(&mut exports, "_start");
    exports.push(0);
    add_uleb(&mut exports, plan.start_function);
    add_section(&mut out, SEC_EXPORT, &exports);

    if !plan.table_funcs.is_empty() {
        let mut element = Vec::new();
        add_uleb(&mut element, 1);
        add_uleb(&mut element, 0);
        element.push(0x41);
        add_uleb(&mut element, TABLE_SLOT_BASE);
        element.push(0x0b);
        add_uleb(&mut element, plan.table_funcs.len() as u32);
        for func in &plan.table_funcs {
            add_uleb(&mut element, *func);
        }
        add_section(&mut out, SEC_ELEMENT, &element);
    }

    let data_end = trailing_nonzero(&plan.data_image);
    if data_end > 0 {
        let mut datacount = Vec::new();
        add_uleb(&mut datacount, 1);
        add_section(&mut out, SEC_DATACOUNT, &datacount);
    }

    let mut code = Vec::new();
    add_uleb(&mut code, total);
    for object in &plan.objects {
        for body in &object.code {
            add_uleb(&mut code, body.len() as u32);
            code.extend_from_slice(body);
        }
    }
    add_section(&mut out, SEC_CODE, &code);

    if data_end > 0 {
        let mut data = Vec::new();
        add_uleb(&mut data, 1);
        add_uleb(&mut data, 0);
        data.push(0x41);
        add_sleb(&mut data, DATA_BASE as i64);
        data.push(0x0b);
        add_uleb(&mut data, data_end as u32);
        data.extend_from_slice(&plan.data_image[..data_end]);
        add_section(&mut out, SEC_DATA, &data);
    }

    Ok(out)
}

fn translate_init(plan: &Plan, object_index: usize, expr: &[u8]) -> Option<Vec<u8>> {
    let mut reader = Reader::new(expr, "global initializer");
    let mut out = Vec::new();
    let opcode = reader.u8().ok()?;
    match opcode {
        0x41 | 0x42 => {
            let value = reader.sleb().ok()?;
            out.push(opcode);
            add_sleb(&mut out, value);
        }
        0x43 => {
            out.push(opcode);
            out.extend_from_slice(reader.take(4).ok()?);
        }
        0x44 => {
            out.push(opcode);
            out.extend_from_slice(reader.take(8).ok()?);
        }
        0x23 => {
            let index = reader.uleb().ok()?;
            let sym = symbol_index(plan, object_index, index)?;
            let Target::Global(mapped) = plan.resolve(object_index, sym).ok()? else {
                return None;
            };
            out.push(0x23);
            add_uleb(&mut out, mapped);
        }
        _ => return None,
    }
    out.push(0x0b);
    Some(out)
}

fn symbol_index(plan: &Plan, object_index: usize, global_index: u32) -> Option<usize> {
    let symbols = &plan.objects[object_index].symbols;
    symbols
        .iter()
        .position(|symbol| {
            symbol.kind == SYM_KIND_GLOBAL
                && !symbol.is_undefined()
                && symbol.index == global_index
        })
        .or_else(|| {
            symbols.iter().position(|symbol| {
                symbol.kind == SYM_KIND_GLOBAL && symbol.index == global_index
            })
        })
}

fn total_globals(plan: &Plan) -> u32 {
    plan.objects.iter().map(|object| object.globals.len() as u32).sum()
}

fn trailing_nonzero(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .rposition(|byte| *byte != 0)
        .map(|index| index + 1)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Default)]
    struct Builder {
        types: Vec<(Vec<u8>, Vec<u8>)>,
        imports: Vec<(String, String, u8, u32, u8)>,
        funcs: Vec<u32>,
        table_min: u32,
        memory: bool,
        globals: Vec<(u8, bool, i32)>,
        elems: Vec<Vec<u32>>,
        code: Vec<Vec<u8>>,
        data: Vec<Vec<u8>>,
        symbols: Vec<TestSymbol>,
        reloc_code: Vec<Reloc>,
        reloc_data: Vec<Reloc>,
    }

    struct TestSymbol {
        kind: u8,
        flags: u32,
        index: u32,
        name: String,
        module: String,
        segment: u32,
        offset: u32,
        size: u32,
    }

    impl TestSymbol {
        fn function(name: &str, index: u32) -> TestSymbol {
            TestSymbol {
                kind: SYM_KIND_FUNCTION,
                flags: 0,
                index,
                name: name.to_string(),
                module: String::new(),
                segment: 0,
                offset: 0,
                size: 0,
            }
        }

        fn undefined_function(name: &str, index: u32) -> TestSymbol {
            TestSymbol {
                flags: SYM_UNDEFINED | SYM_EXPLICIT_NAME,
                ..TestSymbol::function(name, index)
            }
        }

        fn data(name: &str, segment: u32, size: u32) -> TestSymbol {
            TestSymbol {
                kind: SYM_KIND_DATA,
                flags: 0,
                index: segment,
                name: name.to_string(),
                module: String::new(),
                segment,
                offset: 0,
                size,
            }
        }

        fn undefined_data(name: &str) -> TestSymbol {
            TestSymbol {
                kind: SYM_KIND_DATA,
                flags: SYM_UNDEFINED,
                index: 0,
                name: name.to_string(),
                module: String::new(),
                segment: 0,
                offset: 0,
                size: 0,
            }
        }
    }

    fn padded(value: i32) -> Vec<u8> {
        let mut out = Vec::new();
        add_sleb(&mut out, value as i64);
        while out.len() < 5 {
            let last = out.len() - 1;
            out[last] |= 0x80;
            out.push(0);
        }
        out
    }

    impl Builder {
        fn ty(&mut self, params: &[u8], rets: &[u8]) -> u32 {
            self.types.push((params.to_vec(), rets.to_vec()));
            (self.types.len() - 1) as u32
        }

        fn import(&mut self, module: &str, name: &str, ty: u32) -> u32 {
            self.imports
                .push((module.to_string(), name.to_string(), 0, ty, 0));
            (self.imports.len() - 1) as u32
        }

        fn import_global(&mut self, module: &str, name: &str) {
            self.imports
                .push((module.to_string(), name.to_string(), 3, 0x7f, 1));
        }

        fn function(&mut self, ty: u32, body: Vec<u8>) -> u32 {
            let imported = self.imports.iter().filter(|entry| entry.2 == 0).count() as u32;
            self.funcs.push(ty);
            self.code.push(body);
            imported + (self.funcs.len() as u32 - 1)
        }

        fn build(&self) -> Vec<u8> {
            let mut out = Vec::new();
            out.extend_from_slice(WASM_MAGIC);
            out.extend_from_slice(&[1, 0, 0, 0]);
            if !self.types.is_empty() {
                let mut section = Vec::new();
                add_uleb(&mut section, self.types.len() as u32);
                for (params, rets) in &self.types {
                    section.push(0x60);
                    add_uleb(&mut section, params.len() as u32);
                    section.extend_from_slice(params);
                    add_uleb(&mut section, rets.len() as u32);
                    section.extend_from_slice(rets);
                }
                add_section(&mut out, SEC_TYPE, &section);
            }
            if !self.imports.is_empty() {
                let mut section = Vec::new();
                add_uleb(&mut section, self.imports.len() as u32);
                for (module, name, kind, ty, flags) in &self.imports {
                    add_name(&mut section, module);
                    add_name(&mut section, name);
                    section.push(*kind);
                    if *kind == 3 {
                        section.push(*ty as u8);
                        section.push(*flags);
                    } else {
                        add_uleb(&mut section, *ty);
                    }
                }
                add_section(&mut out, SEC_IMPORT, &section);
            }
            if !self.funcs.is_empty() {
                let mut section = Vec::new();
                add_uleb(&mut section, self.funcs.len() as u32);
                for ty in &self.funcs {
                    add_uleb(&mut section, *ty);
                }
                add_section(&mut out, SEC_FUNCTION, &section);
            }
            if self.table_min > 0 {
                let mut section = Vec::new();
                add_uleb(&mut section, 1);
                section.push(0x70);
                add_uleb(&mut section, 0);
                add_uleb(&mut section, self.table_min);
                add_section(&mut out, SEC_TABLE, &section);
            }
            if self.memory {
                let mut section = Vec::new();
                add_uleb(&mut section, 1);
                add_uleb(&mut section, 0);
                add_uleb(&mut section, 1);
                add_section(&mut out, SEC_MEMORY, &section);
            }
            if !self.globals.is_empty() {
                let mut section = Vec::new();
                add_uleb(&mut section, self.globals.len() as u32);
                for (ty, mutable, init) in &self.globals {
                    section.push(*ty);
                    section.push(u8::from(*mutable));
                    section.push(0x41);
                    add_sleb(&mut section, *init as i64);
                    section.push(0x0b);
                }
                add_section(&mut out, SEC_GLOBAL, &section);
            }
            if !self.elems.is_empty() {
                let mut section = Vec::new();
                add_uleb(&mut section, self.elems.len() as u32);
                for funcs in &self.elems {
                    add_uleb(&mut section, 0);
                    section.push(0x41);
                    add_sleb(&mut section, 0);
                    section.push(0x0b);
                    add_uleb(&mut section, funcs.len() as u32);
                    for func in funcs {
                        add_uleb(&mut section, *func);
                    }
                }
                add_section(&mut out, SEC_ELEMENT, &section);
            }
            if !self.code.is_empty() {
                let mut section = Vec::new();
                add_uleb(&mut section, self.code.len() as u32);
                for body in &self.code {
                    add_uleb(&mut section, body.len() as u32);
                    section.extend_from_slice(body);
                }
                add_section(&mut out, SEC_CODE, &section);
            }
            if !self.data.is_empty() {
                let mut section = Vec::new();
                add_uleb(&mut section, self.data.len() as u32);
                for segment in &self.data {
                    add_uleb(&mut section, 0);
                    section.push(0x41);
                    add_sleb(&mut section, 0);
                    section.push(0x0b);
                    add_uleb(&mut section, segment.len() as u32);
                    section.extend_from_slice(segment);
                }
                add_section(&mut out, SEC_DATA, &section);
            }
            let mut linking = Vec::new();
            add_uleb(&mut linking, 2);
            let mut symbols = Vec::new();
            add_uleb(&mut symbols, self.symbols.len() as u32);
            for symbol in &self.symbols {
                symbols.push(symbol.kind);
                add_uleb(&mut symbols, symbol.flags);
                match symbol.kind {
                    SYM_KIND_DATA => {
                        add_name(&mut symbols, &symbol.name);
                        if symbol.flags & SYM_UNDEFINED == 0 {
                            add_uleb(&mut symbols, symbol.segment);
                            add_uleb(&mut symbols, symbol.offset);
                            add_uleb(&mut symbols, symbol.size);
                        }
                    }
                    _ => {
                        add_uleb(&mut symbols, symbol.index);
                        if symbol.flags & SYM_UNDEFINED == 0
                            || symbol.flags & SYM_EXPLICIT_NAME != 0
                        {
                            add_name(&mut symbols, &symbol.name);
                        }
                    }
                }
                let _ = &symbol.module;
            }
            linking.push(8);
            add_uleb(&mut linking, symbols.len() as u32);
            linking.extend_from_slice(&symbols);
            let mut custom = Vec::new();
            add_name(&mut custom, "linking");
            custom.extend_from_slice(&linking);
            add_section(&mut out, 0, &custom);
            for (name, relocs) in [("reloc.CODE", &self.reloc_code), ("reloc.DATA", &self.reloc_data)] {
                if relocs.is_empty() {
                    continue;
                }
                let mut body = Vec::new();
                add_uleb(&mut body, 0);
                add_uleb(&mut body, relocs.len() as u32);
                for reloc in relocs {
                    body.push(reloc.ty);
                    add_uleb(&mut body, reloc.offset);
                    add_uleb(&mut body, reloc.sym);
                    if reloc_has_addend(reloc.ty) {
                        add_sleb(&mut body, reloc.addend);
                    }
                }
                let mut custom = Vec::new();
                add_name(&mut custom, name);
                custom.extend_from_slice(&body);
                add_section(&mut out, 0, &custom);
            }
            out
        }
    }

    fn sections(bytes: &[u8]) -> Vec<(u8, &[u8])> {
        let mut out = Vec::new();
        let mut pos = 8;
        while pos < bytes.len() {
            let id = bytes[pos];
            pos += 1;
            let mut reader = Reader::new(&bytes[pos..], "section header");
            let size = reader.uleb().unwrap() as usize;
            pos += reader.pos;
            out.push((id, &bytes[pos..pos + size]));
            pos += size;
        }
        out
    }

    fn section<'a>(bytes: &'a [u8], id: u8) -> Option<&'a [u8]> {
        sections(bytes)
            .into_iter()
            .find(|(section_id, _)| *section_id == id)
            .map(|(_, body)| body)
    }

    fn count(body: &[u8]) -> u32 {
        Reader::new(body, "count").uleb().unwrap()
    }

    fn first_body(body: &[u8]) -> Vec<u8> {
        let mut reader = Reader::new(body, "code");
        let _count = reader.uleb().unwrap();
        let len = reader.uleb().unwrap() as usize;
        reader.take(len).unwrap().to_vec()
    }

    fn nth_body(code: &[u8], want: usize) -> Vec<u8> {
        let mut reader = Reader::new(code, "code");
        let count = reader.uleb().unwrap();
        for index in 0..count {
            let len = reader.uleb().unwrap() as usize;
            let body = reader.take(len).unwrap();
            if index as usize == want {
                return body.to_vec();
            }
        }
        panic!("no body {want}");
    }

    fn first_const(body: &[u8]) -> i64 {
        let mut reader = Reader::new(body, "body");
        let groups = reader.uleb().unwrap();
        for _ in 0..groups {
            reader.uleb().unwrap();
            reader.u8().unwrap();
        }
        assert_eq!(reader.u8().unwrap(), 0x41);
        reader.sleb().unwrap()
    }

    fn start_body(object: &mut Builder, funcs: &[(&str, Vec<u8>)]) {
        for (name, body) in funcs {
            let ty = object.ty(&[], &[0x7e]);
            let index = object.function(ty, body.clone());
            object.symbols.push(TestSymbol::function(name, index));
        }
    }

    fn simple_start() -> Vec<u8> {
        let mut object = Builder::default();
        let ty = object.ty(&[], &[0x7e]);
        let index = object.function(ty, vec![0x00, 0x42, 0x00, 0x0b]);
        object.symbols.push(TestSymbol::function("_start", index));
        object.build()
    }

    #[test]
    fn links_two_objects_and_exports_start() {
        let mut caller = Builder::default();
        let callee_ty = caller.ty(&[], &[0x7e]);
        let import = caller.import("env", "helper", callee_ty);
        caller
            .symbols
            .push(TestSymbol::undefined_function("helper", import));
        let ty = caller.ty(&[], &[0x7e]);
        let mut body = vec![0x00, 0x10];
        add_uleb(&mut body, 0);
        body.push(0x0b);
        let index = caller.function(ty, body);
        caller.symbols.push(TestSymbol::function("_start", index));
        caller.reloc_code.push(Reloc {
            ty: R_FUNCTION_INDEX_LEB,
            offset: 4,
            sym: 0,
            addend: 0,
        });

        let mut callee = Builder::default();
        let ty = callee.ty(&[], &[0x7e]);
        let index = callee.function(ty, vec![0x00, 0x42, 0x07, 0x0b]);
        callee.symbols.push(TestSymbol::function("helper", index));

        let bytes = link_bytes(&[
            WasmInput::Object(caller.build()),
            WasmInput::Object(callee.build()),
        ])
        .expect("link");
        let functions = section(&bytes, SEC_FUNCTION).expect("function section");
        assert_eq!(count(functions), 2);
        let code = section(&bytes, SEC_CODE).expect("code section");
        let mut reader = Reader::new(code, "code");
        assert_eq!(reader.uleb().unwrap(), 2);
        let len = reader.uleb().unwrap() as usize;
        let first = reader.take(len).unwrap();
        assert_eq!(&first[1..], &[0x10, 0x01, 0x0b]);
        let exports = section(&bytes, SEC_EXPORT).expect("exports");
        let text = String::from_utf8_lossy(exports);
        assert!(text.contains("_start"), "{text}");
        assert!(text.contains("memory"), "{text}");
    }

    #[test]
    fn patches_memory_addresses_and_is_deterministic() {
        let mut object = Builder::default();
        let ty = object.ty(&[], &[0x7e]);
        let mut body = vec![0x00, 0x41];
        body.extend_from_slice(&padded(0));
        body.push(0x0b);
        let index = object.function(ty, body);
        object.symbols.push(TestSymbol::function("_start", index));
        object.data.push(b"hello".to_vec());
        object.symbols.push(TestSymbol::data("text", 0, 5));
        object.reloc_code.push(Reloc {
            ty: R_MEMORY_ADDR_SLEB,
            offset: 4,
            sym: 1,
            addend: 0,
        });
        let first = link_bytes(&[WasmInput::Object(object.build())]).expect("link");
        let again = link_bytes(&[WasmInput::Object(object.build())]).expect("link");
        assert_eq!(first, again, "output must be byte-identical");
        let code = section(&first, SEC_CODE).expect("code");
        let body = first_body(code);
        let address = first_const(&body);
        assert_eq!(address, DATA_BASE as i64);
        let data = section(&first, SEC_DATA).expect("data");
        assert!(data.windows(5).any(|window| window == b"hello"));
    }

    #[test]
    fn assigns_table_slots_through_element_segments() {
        let mut object = Builder::default();
        object.table_min = 3;
        let ty = object.ty(&[], &[0x7e]);
        let target = object.function(ty, vec![0x00, 0x42, 0x00, 0x0b]);
        let mut body = vec![0x00, 0x41];
        body.extend_from_slice(&padded(0));
        body.push(0x0b);
        let start = object.function(ty, body);
        object.symbols.push(TestSymbol::function("target", target));
        object.symbols.push(TestSymbol::function("_start", start));
        object.elems.push(vec![target]);
        object.reloc_code.push(Reloc {
            ty: R_TABLE_INDEX_SLEB,
            offset: 9,
            sym: 0,
            addend: 0,
        });
        let bytes = link_bytes(&[WasmInput::Object(object.build())]).expect("link");
        let code = section(&bytes, SEC_CODE).expect("code");
        let body = nth_body(code, 1);
        assert_eq!(first_const(&body), TABLE_SLOT_BASE as i64);
        let element = section(&bytes, SEC_ELEMENT).expect("element");
        let mut reader = Reader::new(element, "element");
        assert_eq!(reader.uleb().unwrap(), 1);
        assert_eq!(reader.uleb().unwrap(), 0);
        assert_eq!(reader.u8().unwrap(), 0x41);
        assert_eq!(reader.sleb().unwrap(), TABLE_SLOT_BASE as i64);
        assert_eq!(reader.u8().unwrap(), 0x0b);
        assert_eq!(reader.uleb().unwrap(), 1);
        assert_eq!(reader.uleb().unwrap(), 0, "target is the only defined function");
    }

    #[test]
    fn pulls_archive_members_on_demand() {
        let mut object = Builder::default();
        let helper_ty = object.ty(&[], &[0x7e]);
        let import = object.import("env", "needed", helper_ty);
        object
            .symbols
            .push(TestSymbol::undefined_function("needed", import));
        let ty = object.ty(&[], &[0x7e]);
        let mut body = vec![0x00, 0x10];
        add_uleb(&mut body, 0);
        body.push(0x0b);
        let index = object.function(ty, body);
        object.symbols.push(TestSymbol::function("_start", index));
        object.reloc_code.push(Reloc {
            ty: R_FUNCTION_INDEX_LEB,
            offset: 4,
            sym: 1,
            addend: 0,
        });

        let mut needed = Builder::default();
        let ty = needed.ty(&[], &[0x7e]);
        let index = needed.function(ty, vec![0x00, 0x42, 0x01, 0x0b]);
        needed.symbols.push(TestSymbol::function("needed", index));

        let mut unused = Builder::default();
        let ty = unused.ty(&[], &[0x7e]);
        let index = unused.function(ty, vec![0x00, 0x42, 0x02, 0x0b]);
        unused.symbols.push(TestSymbol::function("unused", index));

        let bytes = link_bytes(&[
            WasmInput::Object(object.build()),
            WasmInput::Archive(archive(&[needed.build(), unused.build()])),
        ])
        .expect("link");
        let functions = section(&bytes, SEC_FUNCTION).expect("functions");
        assert_eq!(count(functions), 2, "only the needed member is pulled");
    }

    #[test]
    fn rejects_unknown_relocations_and_duplicate_symbols() {
        let mut object = Builder::default();
        let ty = object.ty(&[], &[0x7e]);
        let index = object.function(ty, vec![0x00, 0x42, 0x00, 0x0b]);
        object.symbols.push(TestSymbol::function("_start", index));
        object.reloc_code.push(Reloc {
            ty: 200,
            offset: 3,
            sym: 0,
            addend: 0,
        });
        let error = link_bytes(&[WasmInput::Object(object.build())]).unwrap_err();
        assert!(
            error.to_string().contains("unsupported relocation type 200"),
            "{error}"
        );

        let mut first = Builder::default();
        let ty = first.ty(&[], &[0x7e]);
        let index = first.function(ty, vec![0x00, 0x42, 0x00, 0x0b]);
        first.symbols.push(TestSymbol::function("_start", index));
        first.symbols.push(TestSymbol::function("twice", 0));
        let mut second = Builder::default();
        let ty = second.ty(&[], &[0x7e]);
        let index = second.function(ty, vec![0x00, 0x42, 0x00, 0x0b]);
        second.symbols.push(TestSymbol::function("twice", index));
        let error = link_bytes(&[
            WasmInput::Object(first.build()),
            WasmInput::Object(second.build()),
        ])
        .unwrap_err();
        assert!(error.to_string().contains("duplicate definition"), "{error}");
    }

    #[test]
    fn resolves_heap_base_and_stack_pointer() {
        let mut object = Builder::default();
        let ty = object.ty(&[], &[0x7e]);
        let mut body = vec![0x00, 0x23];
        body.extend_from_slice(&padded(0));
        body.push(0x1a);
        body.push(0x41);
        body.extend_from_slice(&padded(0));
        body.push(0x0b);
        let index = object.function(ty, body);
        object.symbols.push(TestSymbol::function("_start", index));
        object.import_global("env", "__stack_pointer");
        object.symbols.push(TestSymbol {
            kind: SYM_KIND_GLOBAL,
            flags: SYM_UNDEFINED,
            index: 0,
            name: "env.__stack_pointer".to_string(),
            module: "env".to_string(),
            segment: 0,
            offset: 0,
            size: 0,
        });
        object.symbols.push(TestSymbol::undefined_data("__heap_base"));
        object.reloc_code.push(Reloc {
            ty: R_GLOBAL_INDEX_LEB,
            offset: 4,
            sym: 1,
            addend: 0,
        });
        object.reloc_code.push(Reloc {
            ty: R_MEMORY_ADDR_SLEB,
            offset: 11,
            sym: 2,
            addend: 0,
        });
        let bytes = link_bytes(&[WasmInput::Object(object.build())]).expect("link");
        let globals = section(&bytes, SEC_GLOBAL).expect("globals");
        let mut reader = Reader::new(globals, "globals");
        assert_eq!(reader.uleb().unwrap(), 1);
        assert_eq!(reader.u8().unwrap(), 0x7f);
        assert_eq!(reader.u8().unwrap(), 1);
        assert_eq!(reader.u8().unwrap(), 0x41);
        let stack = reader.sleb().unwrap();
        assert!(stack > DATA_BASE as i64);
        let code = section(&bytes, SEC_CODE).expect("code");
        let body = first_body(code);
        assert_eq!(body[1], 0x23, "global.get");
        let global = Reader::new(&body[2..], "global").uleb().unwrap();
        assert_eq!(global, 0, "stack global keeps index 0");
        assert_eq!(body[7], 0x1a, "drop");
        assert_eq!(body[8], 0x41, "i32.const");
        let heap = Reader::new(&body[9..], "heap").sleb().unwrap();
        assert_eq!(heap, stack);
        let memory = section(&bytes, SEC_MEMORY).expect("memory");
        let mut reader = Reader::new(memory, "memory");
        assert_eq!(reader.uleb().unwrap(), 1);
        assert_eq!(reader.uleb().unwrap(), 0);
        assert!(reader.uleb().unwrap() >= 2);
        assert!(link_bytes(&[WasmInput::Object(simple_start())]).is_ok());
    }

    #[test]
    fn missing_start_is_reported() {
        let mut object = Builder::default();
        let ty = object.ty(&[], &[0x7e]);
        object.function(ty, vec![0x00, 0x42, 0x00, 0x0b]);
        let error = link_bytes(&[WasmInput::Object(object.build())]).unwrap_err();
        assert!(error.to_string().contains("_start"), "{error}");
    }

    fn archive(members: &[Vec<u8>]) -> Vec<u8> {
        let mut out = Vec::new();
        out.extend_from_slice(b"!<arch>\n");
        for (index, member) in members.iter().enumerate() {
            let name = format!("member{index}.o");
            let mut header = [b' '; 60];
            header[..name.len()].copy_from_slice(name.as_bytes());
            header[16] = b'0';
            let size = format!("{:<10}", member.len());
            header[48..58].copy_from_slice(size.as_bytes());
            header[58] = b'`';
            header[59] = b'\n';
            out.extend_from_slice(&header);
            out.extend_from_slice(member);
            if member.len() % 2 == 1 {
                out.push(b'\n');
            }
        }
        out
    }

    #[test]
    fn reads_short_archive_names() {
        let members = archive(&[simple_start()]);
        assert_eq!(archive_members(&members).unwrap().len(), 1);
    }
}
