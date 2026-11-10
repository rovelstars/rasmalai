use crate::LinkError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MACHINE_AMD64: u16 = 0x8664;
const IMAGE_BASE: u64 = 0x1_4000_0000;
const SECTION_ALIGN: u64 = 0x1000;
const FILE_ALIGN: u64 = 0x200;
const SCN_CNT_CODE: u32 = 0x0000_0020;
const SCN_CNT_INITIALIZED_DATA: u32 = 0x0000_0040;
const SCN_CNT_UNINITIALIZED_DATA: u32 = 0x0000_0080;
const SCN_LNK_COMDAT: u32 = 0x0000_1000;
const SCN_MEM_DISCARDABLE: u32 = 0x0200_0000;
const SCN_MEM_EXECUTE: u32 = 0x2000_0000;
const SCN_MEM_READ: u32 = 0x4000_0000;
const SCN_MEM_WRITE: u32 = 0x8000_0000;
const SCN_ALIGN_MASK: u32 = 0x00f0_0000;
const SYM_CLASS_EXTERNAL: u8 = 2;
const SYM_CLASS_STATIC: u8 = 3;
const SYM_CLASS_SECTION: u8 = 104;
const SYM_CLASS_WEAK_EXTERNAL: u8 = 105;

#[derive(Debug)]
pub enum PeInput {
    Object(Vec<u8>),
    ObjectPath(PathBuf),
    Library(Vec<u8>),
    LibraryPath(PathBuf),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PeStats {
    pub image_size: u64,
    pub imports: usize,
    pub base_relocations: usize,
    pub sections: usize,
}

#[derive(Debug)]
struct Section {
    name: String,
    data: Vec<u8>,
    relocs: Vec<Reloc>,
    chars: u32,
    align: u32,
    vsize: u32,
}

#[derive(Debug)]
struct Reloc {
    offset: u32,
    symbol: u32,
    kind: u16,
}

#[derive(Debug)]
struct Symbol {
    name: String,
    value: u32,
    section: i32,
    storage: u8,
    aux: Vec<[u8; 18]>,
}

#[derive(Debug)]
struct Object {
    label: String,
    sections: Vec<Section>,
    symbols: Vec<Symbol>,
    symbol_index: Vec<usize>,
}

#[derive(Debug, Clone)]
struct ImportDef {
    dll: String,
    export: String,
    ordinal: Option<u16>,
    hint: u16,
    code: bool,
}

struct Library {
    imports: Vec<ImportDef>,
    objects: Vec<Object>,
}

fn unsupported(message: impl Into<String>) -> LinkError {
    LinkError::Unsupported(message.into())
}

fn collect_undefined(object: &Object, out: &mut BTreeSet<String>) {
    for symbol in &object.symbols {
        if symbol.name.is_empty() {
            continue;
        }
        if symbol.section > 0 {
            continue;
        }
        if symbol.storage == SYM_CLASS_WEAK_EXTERNAL {
            if let Some(tag) = symbol
                .aux
                .first()
                .map(|aux| u32::from_le_bytes([aux[0], aux[1], aux[2], aux[3]]))
            {
                if let Some(alias) = object.symbols.get(tag as usize) {
                    out.insert(alias.name.clone());
                }
            }
            continue;
        }
        out.insert(symbol.name.clone());
    }
}

fn read_u16(bytes: &[u8], at: usize) -> Result<u16, LinkError> {
    let raw = bytes
        .get(at..at + 2)
        .ok_or_else(|| unsupported("truncated COFF field"))?;
    Ok(u16::from_le_bytes([raw[0], raw[1]]))
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, LinkError> {
    let raw = bytes
        .get(at..at + 4)
        .ok_or_else(|| unsupported("truncated COFF field"))?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn read_i16(bytes: &[u8], at: usize) -> Result<i16, LinkError> {
    Ok(read_u16(bytes, at)? as i16)
}

fn write_u16(dst: &mut [u8], at: usize, value: u16) {
    dst[at..at + 2].copy_from_slice(&value.to_le_bytes());
}

fn write_u32(dst: &mut [u8], at: usize, value: u32) {
    dst[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn write_u64(dst: &mut [u8], at: usize, value: u64) {
    dst[at..at + 8].copy_from_slice(&value.to_le_bytes());
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, LinkError> {
    let raw = bytes
        .get(at..at + 8)
        .ok_or_else(|| unsupported("truncated 64-bit COFF field"))?;
    Ok(u64::from_le_bytes([
        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
    ]))
}

fn align_up(value: u64, align: u64) -> u64 {
    if align <= 1 {
        return value;
    }
    value.div_ceil(align) * align
}

fn c_string(bytes: &[u8], at: usize) -> Result<String, LinkError> {
    let rest = bytes
        .get(at..)
        .ok_or_else(|| unsupported("string out of range"))?;
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

fn canonical_chars(name: &str, chars: u32) -> u32 {
    match name {
        ".text" => SCN_CNT_CODE | SCN_MEM_EXECUTE | SCN_MEM_READ,
        ".rdata" | ".pdata" | ".xdata" => SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ,
        ".data" | ".idata" => SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ | SCN_MEM_WRITE,
        ".bss" => SCN_CNT_UNINITIALIZED_DATA | SCN_MEM_READ | SCN_MEM_WRITE,
        _ => {
            chars
                & (SCN_CNT_CODE
                    | SCN_CNT_INITIALIZED_DATA
                    | SCN_CNT_UNINITIALIZED_DATA
                    | SCN_MEM_EXECUTE
                    | SCN_MEM_READ
                    | SCN_MEM_WRITE
                    | SCN_MEM_DISCARDABLE)
        }
    }
}

fn output_section_name(name: &str) -> Result<String, LinkError> {
    if name.len() <= 8 {
        return Ok(name.to_string());
    }
    for base in [
        ".text", ".rdata", ".rodata", ".data", ".bss", ".pdata", ".xdata",
    ] {
        if let Some(rest) = name.strip_prefix(base)
            && (rest.starts_with('.') || rest.starts_with('$'))
        {
            return Ok(if base == ".rodata" {
                ".rdata".to_string()
            } else {
                base.to_string()
            });
        }
    }
    if let Some((base, _)) = name.split_once('$')
        && base.len() <= 8
    {
        return Ok(base.to_string());
    }
    Err(unsupported(format!(
        "section name `{name}` cannot be shortened for a PE image"
    )))
}

fn is_metadata_section(name: &str) -> bool {
    matches!(
        name,
        ".llvmbc" | ".llvmcmd" | ".eh_frame" | ".debug" | ".drectve" | ".note.GNU-stack"
    ) || name.starts_with(".debug$")
        || name.starts_with(".debug.")
}

fn section_alignment(chars: u32) -> u32 {
    let encoded = (chars & SCN_ALIGN_MASK) >> 20;
    match encoded {
        0 | 1 => 1,
        2..=14 => 1u32 << (encoded - 1),
        _ => 0x2000,
    }
}

fn parse_object(label: &str, bytes: &[u8]) -> Result<Object, LinkError> {
    if bytes.len() < 20 {
        return Err(unsupported(format!("{label}: truncated COFF header")));
    }
    let machine = read_u16(bytes, 0)?;
    if machine != MACHINE_AMD64 {
        return Err(unsupported(format!(
            "{label}: machine type {machine:#x} is not x86_64"
        )));
    }
    let section_count = read_u16(bytes, 2)? as usize;
    let symbol_offset = read_u32(bytes, 8)? as usize;
    let symbol_count = read_u32(bytes, 12)? as usize;
    let optional_size = read_u16(bytes, 16)? as usize;
    let section_table = 20 + optional_size;
    let mut sections = Vec::with_capacity(section_count);
    for i in 0..section_count {
        let at = section_table + i * 40;
        let raw = bytes
            .get(at..at + 40)
            .ok_or_else(|| unsupported(format!("{label}: truncated section table")))?;
        let vsize = read_u32(raw, 8)?;
        let size = read_u32(raw, 16)? as usize;
        let raw_ptr = read_u32(raw, 20)? as usize;
        let reloc_ptr = read_u32(raw, 24)? as usize;
        let reloc_count = read_u16(raw, 32)? as usize;
        let chars = read_u32(raw, 36)?;
        let data = if size == 0 || raw_ptr == 0 {
            Vec::new()
        } else {
            bytes
                .get(raw_ptr..raw_ptr + size)
                .ok_or_else(|| unsupported(format!("{label}: section data out of range")))?
                .to_vec()
        };
        // An uninitialized object section keeps its byte count in
        // SizeOfRawData and leaves PointerToRawData at zero. The PE writer
        // needs that count as memory size, not as raw file bytes.
        let vsize = if data.is_empty() && size > 0 {
            size as u32
        } else {
            vsize
        };
        let mut relocs = Vec::with_capacity(reloc_count);
        if reloc_count > 0 {
            let end = reloc_ptr
                .checked_add(reloc_count * 10)
                .ok_or_else(|| unsupported(format!("{label}: relocation table overflow")))?;
            let table = bytes
                .get(reloc_ptr..end)
                .ok_or_else(|| unsupported(format!("{label}: relocation table out of range")))?;
            for r in 0..reloc_count {
                let at = r * 10;
                relocs.push(Reloc {
                    offset: read_u32(table, at)?,
                    symbol: read_u32(table, at + 4)?,
                    kind: read_u16(table, at + 8)?,
                });
            }
        }
        sections.push(Section {
            name: String::new(),
            data,
            relocs,
            chars,
            align: section_alignment(chars),
            vsize,
        });
    }
    let string_table = if symbol_offset == 0 || symbol_count == 0 || symbol_offset >= bytes.len() {
        Vec::new()
    } else {
        let at = symbol_offset + symbol_count * 18;
        let size = read_u32(bytes, at).unwrap_or(4) as usize;
        if size < 4 {
            Vec::new()
        } else {
            bytes.get(at..at + size).unwrap_or_default().to_vec()
        }
    };
    let long_name = |raw: &[u8]| -> Result<String, LinkError> {
        let text = String::from_utf8_lossy(raw).into_owned();
        if let Some(off) = text
            .strip_prefix('/')
            .and_then(|n| n.trim_end_matches('\0').parse::<usize>().ok())
        {
            let rest = string_table.get(off..).unwrap_or_default();
            let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
            return Ok(String::from_utf8_lossy(&rest[..end]).into_owned());
        }
        let end = raw.iter().position(|&b| b == 0).unwrap_or(raw.len());
        Ok(String::from_utf8_lossy(&raw[..end]).into_owned())
    };
    for i in 0..sections.len() {
        let at = section_table + i * 40;
        let raw = &bytes[at..at + 40];
        sections[i].name = long_name(&raw[0..8])?;
    }
    let mut symbols = Vec::new();
    let mut symbol_index = vec![usize::MAX; symbol_count];
    if symbol_count > 0 {
        let mut i = 0usize;
        while i < symbol_count {
            let at = symbol_offset + i * 18;
            let raw = bytes
                .get(at..at + 18)
                .ok_or_else(|| unsupported(format!("{label}: symbol table out of range")))?;
            let name = if raw[0..4] == [0, 0, 0, 0] {
                let off = read_u32(raw, 4)? as usize;
                let rest = string_table
                    .get(off..)
                    .ok_or_else(|| unsupported(format!("{label}: symbol name out of range")))?;
                let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
                String::from_utf8_lossy(&rest[..end]).into_owned()
            } else {
                let end = raw[0..8].iter().position(|&b| b == 0).unwrap_or(8);
                String::from_utf8_lossy(&raw[0..end]).into_owned()
            };
            let value = read_u32(raw, 8)?;
            let section = read_i16(raw, 12)? as i32;
            let storage = raw[16];
            let aux_count = raw[17] as usize;
            let mut aux = Vec::with_capacity(aux_count);
            for a in 0..aux_count {
                let aux_at = at + (1 + a) * 18;
                let entry = bytes.get(aux_at..aux_at + 18).ok_or_else(|| {
                    unsupported(format!("{label}: auxiliary symbol out of range"))
                })?;
                let mut copy = [0u8; 18];
                copy.copy_from_slice(entry);
                aux.push(copy);
            }
            symbol_index[i] = symbols.len();
            symbols.push(Symbol {
                name,
                value,
                section,
                storage,
                aux,
            });
            i += 1 + aux_count;
        }
    }
    Ok(Object {
        label: label.to_string(),
        sections,
        symbols,
        symbol_index,
    })
}

fn archive_members(bytes: &[u8], label: &str) -> Result<Vec<(String, Vec<u8>)>, LinkError> {
    if !bytes.starts_with(b"!<arch>\n") {
        return Err(unsupported(format!("{label}: not an archive")));
    }
    let mut out = Vec::new();
    let mut at = 8usize;
    let mut long_names: Vec<u8> = Vec::new();
    while at + 60 <= bytes.len() {
        let header = &bytes[at..at + 60];
        let raw_name = String::from_utf8_lossy(&header[0..16]).trim().to_string();
        let size_text = String::from_utf8_lossy(&header[48..58]).trim().to_string();
        let size = size_text
            .parse::<usize>()
            .map_err(|_| unsupported(format!("{label}: bad archive member size")))?;
        let data_at = at + 60;
        let data = bytes
            .get(data_at..data_at + size)
            .ok_or_else(|| unsupported(format!("{label}: archive member out of range")))?;
        let name = if raw_name == "/" {
            String::new()
        } else if raw_name == "//" {
            long_names = data.to_vec();
            String::new()
        } else if let Some(off) = raw_name
            .strip_prefix('/')
            .and_then(|n| n.parse::<usize>().ok())
        {
            let end = long_names[off..]
                .iter()
                .position(|&b| b == 0 || b == b'/')
                .map(|p| off + p)
                .unwrap_or(long_names.len());
            String::from_utf8_lossy(&long_names[off..end]).into_owned()
        } else {
            raw_name.trim_end_matches('/').to_string()
        };
        if !name.is_empty() {
            out.push((name, data.to_vec()));
        }
        at = data_at + size;
        if at % 2 == 1 {
            at += 1;
        }
    }
    Ok(out)
}

fn parse_import_member(data: &[u8]) -> Result<Option<ImportDef>, LinkError> {
    if data.len() < 20 {
        return Ok(None);
    }
    let sig1 = read_u16(data, 0)?;
    let sig2 = read_u16(data, 2)?;
    let long = sig1 == 0xffff && sig2 == 0;
    let short = sig1 == 0 && sig2 == 0xffff;
    if !long && !short {
        return Ok(None);
    }
    let machine = read_u16(data, 6)?;
    if machine != 0 && machine != MACHINE_AMD64 {
        return Err(unsupported(format!(
            "import object machine {machine:#x} is not x86_64"
        )));
    }
    let size = read_u32(data, 12)? as usize;
    let ordinal_hint = read_u16(data, 16)?;
    let flags = read_u16(data, 18)?;
    let kind = flags & 3;
    let name_type = (flags >> 2) & 7;
    let strings = data
        .get(20..20 + size)
        .ok_or_else(|| unsupported("import object data out of range"))?;
    let (_symbol, dll, export) = if long {
        let symbol = c_string(strings, 0)?;
        let rest = (symbol.len() + 1).min(strings.len());
        let dll = c_string(strings, rest)?;
        let rest2 = (rest + dll.len() + 1).min(strings.len());
        let export = if rest2 < strings.len() {
            c_string(strings, rest2).unwrap_or_else(|_| symbol.clone())
        } else {
            symbol.clone()
        };
        (symbol, dll, export)
    } else {
        let (symbol, rest) = match name_type {
            0 => (String::new(), 0),
            1..=3 => {
                let name = c_string(strings, 0)?;
                let next = name.len() + 1;
                (name, next)
            }
            4 => {
                let name = c_string(strings, 0)?;
                let next = name.len() + 1;
                (name, next)
            }
            _ => return Err(unsupported("unknown import name type")),
        };
        let dll = c_string(strings, rest)?;
        let export = if symbol.is_empty() {
            format!("ordinal_{ordinal_hint}")
        } else {
            symbol.clone()
        };
        (symbol, dll, export)
    };
    if dll.is_empty() {
        return Err(unsupported("import object names no DLL"));
    }
    let ordinal = (name_type == 0).then_some(ordinal_hint);
    Ok(Some(ImportDef {
        dll,
        export: export.trim_start_matches("__imp_").to_string(),
        ordinal,
        hint: ordinal_hint,
        code: kind == 0,
    }))
}

fn parse_library(label: &str, bytes: &[u8]) -> Result<Library, LinkError> {
    let mut imports = Vec::new();
    let mut objects = Vec::new();
    for (name, data) in archive_members(bytes, label)? {
        if let Some(import) = parse_import_member(&data)? {
            imports.push(import);
            continue;
        }
        let object = parse_object(&format!("{label}({name})"), &data)?;
        if object
            .sections
            .iter()
            .any(|section| section.name.starts_with(".idata$"))
        {
            continue;
        }
        objects.push(object);
    }
    Ok(Library { imports, objects })
}

#[derive(Clone, Copy, Debug)]
enum Target {
    Rva {
        rva: u64,
        section: Option<usize>,
        section_off: u64,
    },
    Absolute(u64),
}

impl Target {
    fn rva(self) -> u64 {
        match self {
            Target::Rva { rva, .. } => rva,
            Target::Absolute(value) => value,
        }
    }
}

#[derive(Clone, Copy)]
struct Placement {
    out: usize,
    off: u64,
}

#[derive(Clone, Copy)]
struct SymRef {
    obj: usize,
    sym: usize,
}

struct OutSection {
    name: String,
    chars: u32,
    align: u32,
    data: Vec<u8>,
    memsz: u64,
    rva: u64,
    raw_off: u64,
    raw_size: u64,
}

struct Linker {
    objects: Vec<Object>,
    pool: Vec<Object>,
    pulled: Vec<bool>,
    kept: Vec<Vec<bool>>,
    placement: Vec<Vec<Option<Placement>>>,
    discards: BTreeMap<(usize, usize), (usize, usize)>,
    out: Vec<OutSection>,
    defs: BTreeMap<String, SymRef>,
    absolute: BTreeMap<String, u64>,
    commons: BTreeMap<(usize, usize), (usize, u64)>,
    imports: Vec<ImportDef>,
    import_symbols: BTreeMap<String, usize>,
    used_imports: BTreeSet<usize>,
    thunks: BTreeMap<usize, u64>,
    iat: BTreeMap<usize, u64>,
    entry: String,
    subsystem: u16,
}

impl Linker {
    fn new(
        objects: Vec<Object>,
        pool: Vec<Object>,
        imports: Vec<ImportDef>,
        entry: &str,
        subsystem: u16,
    ) -> Linker {
        let kept = objects
            .iter()
            .map(|obj| vec![true; obj.sections.len()])
            .collect();
        let placement = objects
            .iter()
            .map(|obj| vec![None; obj.sections.len()])
            .collect();
        let pulled = vec![false; pool.len()];
        let mut import_symbols = BTreeMap::new();
        for (index, import) in imports.iter().enumerate() {
            import_symbols.insert(format!("__imp_{}", import.export), index);
            import_symbols.insert(import.export.clone(), index);
        }
        Linker {
            objects,
            pool,
            pulled,
            kept,
            placement,
            discards: BTreeMap::new(),
            out: Vec::new(),
            defs: BTreeMap::new(),
            absolute: BTreeMap::new(),
            commons: BTreeMap::new(),
            imports,
            import_symbols,
            used_imports: BTreeSet::new(),
            thunks: BTreeMap::new(),
            iat: BTreeMap::new(),
            entry: entry.to_string(),
            subsystem,
        }
    }

    fn pull_archive_members(&mut self) {
        if self.pool.is_empty() {
            return;
        }
        let mut wanted: BTreeSet<String> = BTreeSet::new();
        wanted.insert(self.entry.clone());
        for obj in &self.objects {
            collect_undefined(obj, &mut wanted);
        }
        loop {
            let mut pulled_any = false;
            for index in 0..self.pool.len() {
                if self.pulled[index] {
                    continue;
                }
                let needed = self.pool[index].symbols.iter().any(|sym| {
                    if sym.name.is_empty() || !wanted.contains(&sym.name) {
                        return false;
                    }
                    if sym.section > 0 {
                        return true;
                    }
                    // A weak external with an auxiliary tag names a fallback
                    // symbol in the same member, so the member provides it.
                    sym.storage == SYM_CLASS_WEAK_EXTERNAL
                        && sym
                            .aux
                            .first()
                            .map(|aux| u32::from_le_bytes([aux[0], aux[1], aux[2], aux[3]]))
                            .unwrap_or(0)
                            != 0
                });
                if !needed {
                    continue;
                }
                self.pulled[index] = true;
                pulled_any = true;
                collect_undefined(&self.pool[index], &mut wanted);
            }
            if !pulled_any {
                break;
            }
        }
        let mut index = 0;
        while index < self.pool.len() {
            if self.pulled[index] {
                let object = self.pool.remove(index);
                self.pulled.remove(index);
                self.objects.push(object);
            } else {
                index += 1;
            }
        }
    }

    fn raw_symbol(&self, oi: usize, raw: usize) -> Result<usize, LinkError> {
        self.objects
            .get(oi)
            .and_then(|object| object.symbol_index.get(raw))
            .copied()
            .filter(|&index| index != usize::MAX)
            .ok_or_else(|| {
                let label = self
                    .objects
                    .get(oi)
                    .map(|o| o.label.as_str())
                    .unwrap_or("?");
                let count = self
                    .objects
                    .get(oi)
                    .map(|o| o.symbol_index.len())
                    .unwrap_or(0);
                unsupported(format!(
                    "debug: relocation symbol {raw} of {count} in {label}"
                ))
            })
    }

    fn dedupe_comdat(&mut self) -> Result<(), LinkError> {
        let mut seen: BTreeMap<String, (usize, usize)> = BTreeMap::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                let chars = self.objects[oi].sections[si].chars;
                if chars & SCN_LNK_COMDAT == 0 {
                    continue;
                }
                let name = self.objects[oi].sections[si].name.clone();
                let mut signature = None;
                for sym in &self.objects[oi].symbols {
                    if sym.section == (si as i32 + 1)
                        && sym.storage == SYM_CLASS_STATIC
                        && !sym.aux.is_empty()
                    {
                        let selection = sym.aux[0][14];
                        // Only ANY, SAME_SIZE, and EXACT_MATCH carry a
                        // duplicate-discard rule. NODUPLICATES must surface a
                        // real duplicate, and a section symbol (named after
                        // its own section) is not a COMDAT key at all.
                        if !(2..=4).contains(&selection) || sym.name == name {
                            continue;
                        }
                        signature = Some(sym.name.clone());
                        break;
                    }
                }
                let Some(signature) = signature else {
                    continue;
                };
                if let Some(&(koi, ksi)) = seen.get(&signature) {
                    self.kept[oi][si] = false;
                    self.discards.insert((oi, si), (koi, ksi));
                } else {
                    seen.insert(signature, (oi, si));
                }
            }
        }
        Ok(())
    }

    fn merge_sections(&mut self) -> Result<(), LinkError> {
        let order = [".text", ".rdata", ".data", ".bss", ".pdata", ".xdata"];
        let mut names: Vec<String> = Vec::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                if !self.kept[oi][si] {
                    continue;
                }
                let section = &self.objects[oi].sections[si];
                let label = &self.objects[oi].label;
                if section.name == ".idata" || section.name == ".reloc" {
                    return Err(unsupported(format!(
                        "{label}: input section {} conflicts with the PE linker output",
                        section.name
                    )));
                }
                if section.data.len() as u32 > 0 || section.vsize > 0 {
                    let name = output_section_name(&section.name)?;
                    if !names.contains(&name) {
                        names.push(name);
                    }
                }
            }
        }
        names.sort_by(|a, b| {
            let rank = |name: &str| order.iter().position(|o| o == &name).unwrap_or(order.len());
            rank(a).cmp(&rank(b)).then_with(|| a.cmp(b))
        });
        for name in &names {
            self.ensure_section(name);
        }
        let mut placements: Vec<Vec<Option<Placement>>> = self
            .objects
            .iter()
            .map(|obj| vec![None; obj.sections.len()])
            .collect();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                if !self.kept[oi][si] {
                    continue;
                }
                let (name, align, chars, data_len, bss) = {
                    let section = &self.objects[oi].sections[si];
                    let bss = section.chars & SCN_CNT_UNINITIALIZED_DATA != 0
                        && section.data.iter().all(|&b| b == 0);
                    (
                        output_section_name(&section.name)?,
                        section.align,
                        section.chars,
                        (section.data.len() as u64).max(section.vsize as u64),
                        bss,
                    )
                };
                if data_len == 0 {
                    continue;
                }
                let out = self.ensure_section(&name);
                let out_align = self.out[out].align.max(align);
                self.out[out].align = out_align;
                let off = if bss {
                    let off = align_up(self.out[out].memsz, align as u64);
                    self.out[out].memsz = off + data_len;
                    off
                } else {
                    let off = align_up(self.out[out].data.len() as u64, align as u64);
                    if off > self.out[out].data.len() as u64 {
                        self.out[out].data.resize(off as usize, 0);
                    }
                    self.out[out]
                        .data
                        .extend_from_slice(&self.objects[oi].sections[si].data);
                    self.out[out].memsz = self.out[out].data.len() as u64;
                    off
                };
                let mask = SCN_CNT_CODE
                    | SCN_CNT_INITIALIZED_DATA
                    | SCN_CNT_UNINITIALIZED_DATA
                    | SCN_MEM_EXECUTE
                    | SCN_MEM_READ
                    | SCN_MEM_WRITE
                    | SCN_MEM_DISCARDABLE;
                self.out[out].chars |= chars & mask;
                self.out[out].chars = canonical_chars(&self.out[out].name, self.out[out].chars);
                placements[oi][si] = Some(Placement { out, off });
            }
        }
        self.placement = placements;
        Ok(())
    }

    fn ensure_section(&mut self, name: &str) -> usize {
        if let Some(index) = self.out.iter().position(|section| section.name == name) {
            return index;
        }
        let (chars, align) = match name {
            ".text" => (SCN_CNT_CODE | SCN_MEM_EXECUTE | SCN_MEM_READ, 16),
            ".rdata" => (SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ, 16),
            ".data" => (SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ | SCN_MEM_WRITE, 16),
            ".bss" => (
                SCN_CNT_UNINITIALIZED_DATA | SCN_MEM_READ | SCN_MEM_WRITE,
                16,
            ),
            ".pdata" | ".xdata" => (SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ, 4),
            _ => (SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ, 16),
        };
        self.out.push(OutSection {
            name: name.to_string(),
            chars,
            align,
            data: Vec::new(),
            memsz: 0,
            rva: 0,
            raw_off: 0,
            raw_size: 0,
        });
        self.out.len() - 1
    }

    fn allocate_commons(&mut self) -> Result<(), LinkError> {
        let mut commons: Vec<(usize, usize, String, u64)> = Vec::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].symbols.len() {
                let symbol = &self.objects[oi].symbols[si];
                if symbol.section == 0
                    && symbol.value > 0
                    && symbol.storage == SYM_CLASS_EXTERNAL
                    && !symbol.name.is_empty()
                {
                    commons.push((oi, si, symbol.name.clone(), symbol.value as u64));
                }
            }
        }
        commons.sort_by(|a, b| a.2.cmp(&b.2).then(a.0.cmp(&b.0)).then(a.1.cmp(&b.1)));
        for (oi, si, _name, size) in commons {
            if self.defs.contains_key(&self.objects[oi].symbols[si].name) {
                continue;
            }
            let bss = self.ensure_section(".bss");
            let align = size.next_power_of_two().min(16).max(1);
            let off = align_up(self.out[bss].memsz, align);
            self.out[bss].memsz = off + size;
            self.commons.insert((oi, si), (bss, off));
        }
        Ok(())
    }

    fn build_defs(&mut self) -> Result<(), LinkError> {
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].symbols.len() {
                let symbol = &self.objects[oi].symbols[si];
                if symbol.name.is_empty() {
                    continue;
                }
                if symbol.storage == SYM_CLASS_SECTION
                    || (symbol.storage == SYM_CLASS_STATIC && !symbol.aux.is_empty())
                {
                    continue;
                }
                if symbol.section == -1 {
                    self.absolute
                        .entry(symbol.name.clone())
                        .or_insert(symbol.value as u64);
                    continue;
                }
                if symbol.section == 0 && symbol.storage == SYM_CLASS_WEAK_EXTERNAL {
                    let tag = symbol
                        .aux
                        .first()
                        .map(|aux| u32::from_le_bytes([aux[0], aux[1], aux[2], aux[3]]))
                        .unwrap_or(0);
                    if tag != 0
                        && let Ok(target) = self.raw_symbol(oi, tag as usize)
                    {
                        let fallback = &self.objects[oi].symbols[target];
                        let fallback_section = fallback.section.max(1) as usize - 1;
                        if fallback.section > 0
                            && fallback_section < self.kept[oi].len()
                            && self.kept[oi][fallback_section]
                        {
                            let name = symbol.name.clone();
                            self.defs
                                .entry(name)
                                .or_insert(SymRef { obj: oi, sym: target });
                        }
                    }
                    continue;
                }
                if symbol.section <= 0 {
                    continue;
                }
                let section = (symbol.section - 1) as usize;
                if section >= self.kept[oi].len() || !self.kept[oi][section] {
                    continue;
                }
                if let Some(existing) = self.defs.get(&symbol.name) {
                    if existing.obj == oi && existing.sym == si {
                        continue;
                    }
                    if symbol.storage == SYM_CLASS_WEAK_EXTERNAL {
                        continue;
                    }
                    return Err(unsupported(format!(
                        "duplicate symbol `{}` in {} and {}",
                        symbol.name, self.objects[existing.obj].label, self.objects[oi].label
                    )));
                }
                self.defs
                    .insert(symbol.name.clone(), SymRef { obj: oi, sym: si });
            }
        }
        Ok(())
    }

    fn section_target(&self, oi: usize, si: usize, value: u32) -> Result<Target, LinkError> {
        if si >= self.kept[oi].len() {
            return Err(unsupported("symbol names a section outside the object"));
        }
        if !self.kept[oi][si] {
            if let Some(&(koi, ksi)) = self.discards.get(&(oi, si))
                && let Some(place) = self.placement[koi][ksi]
            {
                return Ok(Target::Rva {
                    rva: self.out[place.out].rva + place.off + value as u64,
                    section: Some(place.out),
                    section_off: place.off + value as u64,
                });
            }
            return Ok(Target::Rva {
                rva: 0,
                section: None,
                section_off: 0,
            });
        }
        let place = match self.placement[oi][si] {
            Some(place) => place,
            None => {
                let section = &self.objects[oi].sections[si];
                // Empty objects (zero-length statics, empty string literals)
                // occupy no image bytes, so they get no output section and no
                // real address. Nothing can read through them.
                if section.data.is_empty() && section.vsize == 0 {
                    return Ok(Target::Rva {
                        rva: 0,
                        section: None,
                        section_off: 0,
                    });
                }
                return Err(unsupported(format!(
                    "{}: section {} has no placement",
                    self.objects[oi].label, si
                )));
            }
        };
        Ok(Target::Rva {
            rva: self.out[place.out].rva + place.off + value as u64,
            section: Some(place.out),
            section_off: place.off + value as u64,
        })
    }

    fn import_target(&self, name: &str) -> Option<Target> {
        let index = *self.import_symbols.get(name)?;
        if !self.used_imports.contains(&index) {
            return None;
        }
        let slot = *self.iat.get(&index)?;
        if !name.starts_with("__imp_")
            && self.imports[index].code
            && let Some(rva) = self.thunks.get(&index)
        {
            return Some(Target::Rva {
                rva: *rva,
                section: None,
                section_off: 0,
            });
        }
        Some(Target::Rva {
            rva: slot,
            section: None,
            section_off: 0,
        })
    }

    fn resolve_symbol(&self, oi: usize, si: usize, depth: usize) -> Result<Target, LinkError> {
        if depth > 16 {
            return Err(unsupported("weak external chain is too deep"));
        }
        let symbol = self
            .objects
            .get(oi)
            .and_then(|obj| obj.symbols.get(si))
            .ok_or_else(|| unsupported("relocation names a missing symbol"))?;
        match symbol.section {
            -1 => Ok(Target::Absolute(symbol.value as u64)),
            -2 => Ok(Target::Absolute(0)),
            0 => {
                if symbol.storage == SYM_CLASS_WEAK_EXTERNAL {
                    let tag = symbol
                        .aux
                        .first()
                        .map(|aux| u32::from_le_bytes([aux[0], aux[1], aux[2], aux[3]]))
                        .unwrap_or(0);
                    if tag == 0 {
                        return Ok(Target::Absolute(0));
                    }
                    let tag = self.raw_symbol(oi, tag as usize)?;
                    return self.resolve_symbol(oi, tag, depth + 1);
                }
                if symbol.value > 0 {
                    if let Some(&(out, off)) = self.commons.get(&(oi, si)) {
                        return Ok(Target::Rva {
                            rva: self.out[out].rva + off,
                            section: Some(out),
                            section_off: off,
                        });
                    }
                }
                if let Some(&existing) = self.defs.get(&symbol.name) {
                    if existing.obj != oi || existing.sym != si {
                        return self.resolve_symbol(existing.obj, existing.sym, depth + 1);
                    }
                }
                if let Some(value) = self.absolute.get(&symbol.name) {
                    return Ok(Target::Absolute(*value));
                }
                if let Some(target) = self.import_target(&symbol.name) {
                    return Ok(target);
                }
                if symbol.name == "__ImageBase" || symbol.name == "_image_base__" {
                    return Ok(Target::Rva {
                        rva: 0,
                        section: None,
                        section_off: 0,
                    });
                }
                Err(unsupported(format!("undefined symbol `{}`", symbol.name)))
            }
            section if section > 0 => {
                let si = (section - 1) as usize;
                self.section_target(oi, si, symbol.value)
            }
            _ => Err(unsupported("unsupported COFF symbol section")),
        }
    }

    fn collect_imports(&mut self) -> Result<(), LinkError> {
        let mut thunk_imports = BTreeSet::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                if !self.kept[oi][si] {
                    continue;
                }
                for reloc in &self.objects[oi].sections[si].relocs {
                    let raw = self.raw_symbol(oi, reloc.symbol as usize)?;
                    let symbol = self.objects[oi]
                        .symbols
                        .get(raw)
                        .ok_or_else(|| unsupported("relocation names a missing symbol"))?;
                    if symbol.section != 0 || symbol.value != 0 {
                        continue;
                    }
                    if symbol.storage == SYM_CLASS_WEAK_EXTERNAL {
                        let tag = symbol
                            .aux
                            .first()
                            .map(|aux| u32::from_le_bytes([aux[0], aux[1], aux[2], aux[3]]))
                            .unwrap_or(0);
                        if tag == 0 {
                            continue;
                        }
                        let tag = self.raw_symbol(oi, tag as usize)?;
                        let alias = &self.objects[oi].symbols[tag];
                        if let Some(&index) = self.import_symbols.get(&alias.name) {
                            self.used_imports.insert(index);
                        }
                        continue;
                    }
                    if self.defs.contains_key(&symbol.name)
                        || self.absolute.contains_key(&symbol.name)
                    {
                        continue;
                    }
                    let Some(&index) = self.import_symbols.get(&symbol.name) else {
                        continue;
                    };
                    self.used_imports.insert(index);
                    if !symbol.name.starts_with("__imp_") && self.imports[index].code {
                        thunk_imports.insert(index);
                    }
                }
            }
        }
        // Resolve references that only appear as weak aliases or symbols with
        // non-zero addends in the common table.
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].symbols.len() {
                let symbol = &self.objects[oi].symbols[si];
                if symbol.section == 0
                    && symbol.storage != SYM_CLASS_WEAK_EXTERNAL
                    && symbol.value == 0
                    && !symbol.name.is_empty()
                    && !self.defs.contains_key(&symbol.name)
                    && !self.absolute.contains_key(&symbol.name)
                    && let Some(&index) = self.import_symbols.get(&symbol.name)
                {
                    self.used_imports.insert(index);
                }
            }
        }
        if !self.used_imports.is_empty() {
            let text = self.ensure_section(".text");
            let base = align_up(self.out[text].data.len() as u64, 16);
            self.out[text].data.resize(base as usize, 0);
            for (n, index) in thunk_imports.iter().enumerate() {
                self.thunks.insert(*index, 0);
                let off = base + n as u64 * 6;
                self.out[text].data.resize((off + 6) as usize, 0);
                self.out[text].memsz = self.out[text].data.len() as u64;
            }
            let mut thunk_at = base;
            for index in thunk_imports {
                self.thunks.insert(index, thunk_at);
                thunk_at += 6;
            }
        }
        Ok(())
    }

    fn assign_rvas(&mut self) -> Result<(), LinkError> {
        let mut rva = SECTION_ALIGN;
        for section in &mut self.out {
            let align = SECTION_ALIGN;
            rva = align_up(rva, align);
            section.rva = rva;
            rva += align_up(section.memsz.max(section.data.len() as u64), 1);
        }
        Ok(())
    }

    fn build_idata(&mut self) -> Result<(u64, u64, u64, u64), LinkError> {
        if self.used_imports.is_empty() {
            return Ok((0, 0, 0, 0));
        }
        let mut grouped: BTreeMap<String, Vec<usize>> = BTreeMap::new();
        for &index in &self.used_imports {
            grouped
                .entry(self.imports[index].dll.clone())
                .or_default()
                .push(index);
        }
        for entries in grouped.values_mut() {
            entries.sort_by(|a, b| self.imports[*a].export.cmp(&self.imports[*b].export));
        }
        let groups: Vec<(String, Vec<usize>)> = grouped.into_iter().collect();
        let dir_size = (groups.len() as u64 + 1) * 20;
        let mut cursor = dir_size;
        let mut ilt_off = Vec::new();
        let mut iat_off = Vec::new();
        for (_, entries) in &groups {
            ilt_off.push(cursor);
            cursor += (entries.len() as u64 + 1) * 8;
            iat_off.push(cursor);
            cursor += (entries.len() as u64 + 1) * 8;
        }
        let mut hint_off: BTreeMap<usize, u64> = BTreeMap::new();
        for (_, entries) in &groups {
            for &index in entries {
                let import = &self.imports[index];
                if import.ordinal.is_none() {
                    hint_off.insert(index, cursor);
                    cursor += 2 + import.export.len() as u64 + 1;
                    if cursor % 2 == 1 {
                        cursor += 1;
                    }
                }
            }
        }
        let mut dll_off: BTreeMap<String, u64> = BTreeMap::new();
        for (dll, _) in &groups {
            dll_off.insert(dll.clone(), cursor);
            cursor += dll.len() as u64 + 1;
        }
        let size = align_up(cursor, 4);
        let idata = self.ensure_section(".idata");
        self.out[idata].chars = SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ | SCN_MEM_WRITE;
        self.out[idata].memsz = size;
        self.out[idata].data = vec![0u8; size as usize];
        self.assign_rvas()?;
        let base = self.out[idata].rva;
        let mut bytes = vec![0u8; size as usize];
        let mut dir = 0usize;
        for (g, (dll, entries)) in groups.iter().enumerate() {
            let ilt = base + ilt_off[g];
            let iat = base + iat_off[g];
            write_u32(&mut bytes, dir, ilt as u32);
            write_u32(&mut bytes, dir + 12, (base + dll_off[dll]) as u32);
            write_u32(&mut bytes, dir + 16, iat as u32);
            dir += 20;
            let mut slot = 0u64;
            for &index in entries {
                let import = &self.imports[index];
                self.iat.insert(index, base + iat_off[g] + slot * 8);
                let entry = if let Some(ordinal) = import.ordinal {
                    0x8000_0000_0000_0000 | ordinal as u64
                } else {
                    base + hint_off[&index]
                };
                let at = (ilt_off[g] + slot * 8) as usize;
                write_u64(&mut bytes, at, entry);
                let at = (iat_off[g] + slot * 8) as usize;
                write_u64(&mut bytes, at, entry);
                slot += 1;
            }
            let at = (ilt_off[g] + slot * 8) as usize;
            write_u64(&mut bytes, at, 0);
            let at = (iat_off[g] + slot * 8) as usize;
            write_u64(&mut bytes, at, 0);
        }
        for (g, (_, entries)) in groups.iter().enumerate() {
            for &index in entries {
                let import = &self.imports[index];
                if let Some(off) = hint_off.get(&index) {
                    write_u16(&mut bytes, *off as usize, import.hint);
                    let name = import.export.as_bytes();
                    let at = *off as usize + 2;
                    bytes[at..at + name.len()].copy_from_slice(name);
                }
            }
            let _ = g;
        }
        for (dll, off) in &dll_off {
            let at = *off as usize;
            let name = dll.as_bytes();
            bytes[at..at + name.len()].copy_from_slice(name);
        }
        self.out[idata].data = bytes;
        let first_iat = iat_off.first().map(|off| base + off).unwrap_or(0);
        let total_iat: u64 = groups
            .iter()
            .map(|(_, entries)| (entries.len() as u64 + 1) * 8)
            .sum();
        Ok((base, dir_size, first_iat, total_iat))
    }

    fn fill_thunks(&mut self) -> Result<(), LinkError> {
        let text = self
            .out
            .iter()
            .position(|section| section.name == ".text")
            .ok_or_else(|| unsupported("import thunks need a .text section"))?;
        for (index, off) in self.thunks.clone() {
            let slot = self
                .iat
                .get(&index)
                .ok_or_else(|| unsupported("import thunk has no IAT slot"))?;
            let at = (self.out[text].rva + off) as i64;
            let disp = (*slot as i64) - (at + 6);
            if disp != (disp as i32) as i64 {
                return Err(unsupported("import thunk displacement overflows 32 bits"));
            }
            let data = &mut self.out[text].data;
            let start = off as usize;
            data[start] = 0xff;
            data[start + 1] = 0x25;
            data[start + 2..start + 6].copy_from_slice(&(disp as i32).to_le_bytes());
        }
        Ok(())
    }

    fn apply_relocations(&mut self) -> Result<Vec<(u64, u16)>, LinkError> {
        let mut base_relocs = Vec::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                if !self.kept[oi][si] {
                    continue;
                }
                let place = match self.placement[oi][si] {
                    Some(place) => place,
                    None => continue,
                };
                let relocs = &self.objects[oi].sections[si].relocs;
                for reloc in relocs {
                    let p = self.out[place.out].rva + place.off + reloc.offset as u64;
                    let raw = self.raw_symbol(oi, reloc.symbol as usize)?;
                    let target = self.resolve_symbol(oi, raw, 0)?;
                    let site = (place.off + reloc.offset as u64) as usize;
                    let out = &mut self.out[place.out];
                    let data = &mut out.data;
                    let need = match reloc.kind {
                        1 => 8,
                        10 => 2,
                        12 => 1,
                        _ => 4,
                    };
                    if site + need > data.len() {
                        return Err(unsupported(format!(
                            "{}: relocation at {:#x} is out of range",
                            self.objects[oi].label, reloc.offset
                        )));
                    }
                    match reloc.kind {
                        0 => {}
                        1 => {
                            let addend = read_u64(data, site)? as i64;
                            let value = IMAGE_BASE as i64 + target.rva() as i64 + addend;
                            write_u64(data, site, value as u64);
                            base_relocs.push((p, 10));
                        }
                        2 => {
                            let addend = read_u32(data, site)? as i64;
                            let value = IMAGE_BASE as i64 + target.rva() as i64 + addend;
                            if value != (value as i32) as i64 {
                                return Err(unsupported("ADDR32 absolute address exceeds 32 bits"));
                            }
                            write_u32(data, site, value as u32);
                            base_relocs.push((p, 3));
                        }
                        3 => {
                            let addend = read_u32(data, site)? as i64;
                            let value = target.rva() as i64 + addend;
                            if value != (value as i32) as i64 {
                                return Err(unsupported("ADDR32NB address exceeds 32 bits"));
                            }
                            write_u32(data, site, value as u32);
                        }
                        4..=9 => {
                            let extra = if reloc.kind == 4 {
                                0
                            } else {
                                (reloc.kind - 4) as i64
                            };
                            let addend = read_u32(data, site)? as i32 as i64;
                            let value = target.rva() as i64 + addend - (p as i64 + 4 + extra);
                            if value != (value as i32) as i64 {
                                return Err(unsupported("REL32 displacement exceeds 32 bits"));
                            }
                            write_u32(data, site, value as i32 as u32);
                        }
                        10 => {
                            let section = target.section().ok_or_else(|| {
                                unsupported("SECTION relocation names an absolute symbol")
                            })?;
                            write_u16(data, site, (section + 1) as u16);
                        }
                        11 | 12 => {
                            let value = match target {
                                Target::Rva { section_off, .. } => section_off,
                                Target::Absolute(value) => value,
                            };
                            if reloc.kind == 11 {
                                if value > u32::MAX as u64 {
                                    return Err(unsupported("SECREL offset exceeds 32 bits"));
                                }
                                write_u32(data, site, value as u32);
                            } else {
                                data[site] = (value & 0x7f) as u8;
                            }
                        }
                        other => {
                            return Err(unsupported(format!(
                                "{}: unsupported x86_64 COFF relocation {other}",
                                self.objects[oi].label
                            )));
                        }
                    }
                }
            }
        }
        Ok(base_relocs)
    }

    fn sort_pdata(&mut self) {
        let Some(section) = self.out.iter_mut().find(|section| section.name == ".pdata") else {
            return;
        };
        if section.data.len() % 12 != 0 {
            return;
        }
        let mut entries: Vec<[u8; 12]> = section
            .data
            .chunks_exact(12)
            .map(|chunk| {
                let mut entry = [0u8; 12];
                entry.copy_from_slice(chunk);
                entry
            })
            .collect();
        entries.sort_by_key(|entry| u32::from_le_bytes([entry[0], entry[1], entry[2], entry[3]]));
        for (at, entry) in entries.iter().enumerate() {
            section.data[at * 12..at * 12 + 12].copy_from_slice(entry);
        }
    }

    fn add_reloc_section(&mut self, mut relocs: Vec<(u64, u16)>) -> Result<(u64, u64), LinkError> {
        if relocs.is_empty() {
            return Ok((0, 0));
        }
        relocs.sort();
        let mut bytes: Vec<u8> = Vec::new();
        let mut i = 0;
        while i < relocs.len() {
            let page = relocs[i].0 & !0xfff;
            let mut entries: Vec<(u64, u16)> = Vec::new();
            while i < relocs.len() && relocs[i].0 & !0xfff == page {
                entries.push((relocs[i].0 & 0xfff, relocs[i].1));
                i += 1;
            }
            let block = 8 + entries.len() * 2;
            let block = block.div_ceil(4) * 4;
            let at = bytes.len();
            bytes.resize(at + block, 0);
            write_u32(&mut bytes, at, page as u32);
            write_u32(&mut bytes, at + 4, block as u32);
            for (n, (offset, kind)) in entries.iter().enumerate() {
                write_u16(
                    &mut bytes,
                    at + 8 + n * 2,
                    (kind << 12) | (*offset as u16 & 0xfff),
                );
            }
        }
        let idata = self.ensure_section(".reloc");
        self.out[idata].chars = SCN_CNT_INITIALIZED_DATA | SCN_MEM_READ | SCN_MEM_DISCARDABLE;
        self.out[idata].data = bytes;
        self.out[idata].memsz = self.out[idata].data.len() as u64;
        self.assign_rvas()?;
        Ok((self.out[idata].rva, self.out[idata].memsz))
    }

    fn entry_rva(&self) -> Result<u64, LinkError> {
        let mut candidates = vec![self.entry.as_str()];
        if self.entry == "mainCRTStartup" {
            // MinGW CRTs name the C entry `main` and some hand-written stubs
            // call it `start`; a target with its own runtime (UEFI's
            // `efi_main`) must resolve exactly the symbol it asked for.
            candidates.extend(["main", "start"]);
        }
        for name in candidates {
            if let Some(reference) = self.defs.get(name) {
                let target = self.section_target(
                    reference.obj,
                    (self.objects[reference.obj].symbols[reference.sym].section - 1) as usize,
                    self.objects[reference.obj].symbols[reference.sym].value,
                )?;
                return Ok(target.rva());
            }
        }
        Err(unsupported(format!(
            "entry symbol `{}` is not defined",
            self.entry
        )))
    }

    fn write_pe(
        &mut self,
        pdata: (u64, u64),
        idata: (u64, u64, u64, u64),
        reloc: (u64, u64),
        entry: u64,
        base_relocs: usize,
        output: &Path,
    ) -> Result<PeStats, LinkError> {
        let section_count = self.out.len();
        let name_table = {
            let mut table: Vec<u8> = Vec::new();
            let mut offsets: Vec<Option<u32>> = Vec::with_capacity(section_count);
            for section in &self.out {
                if section.name.len() <= 8 {
                    offsets.push(None);
                } else {
                    offsets.push(Some(table.len() as u32));
                    table.extend_from_slice(section.name.as_bytes());
                    table.push(0);
                }
            }
            (table, offsets)
        };
        let section_table = 0x80 + 4 + 20 + 240;
        let header_end = section_table + section_count * 40 + name_table.0.len();
        let headers_size = align_up(header_end as u64, FILE_ALIGN);
        let mut raw = headers_size;
        for section in &mut self.out {
            let size = section.data.len() as u64;
            section.raw_size = if size == 0 {
                0
            } else {
                align_up(size, FILE_ALIGN)
            };
            section.raw_off = if size == 0 { 0 } else { raw };
            if size > 0 {
                raw += section.raw_size;
            }
        }
        let image_end = self
            .out
            .iter()
            .map(|section| section.rva + section.memsz.max(section.data.len() as u64))
            .max()
            .unwrap_or(SECTION_ALIGN);
        let image_size = align_up(image_end, SECTION_ALIGN);
        let size_of_code: u64 = self
            .out
            .iter()
            .filter(|section| section.chars & SCN_CNT_CODE != 0)
            .map(|section| section.raw_size)
            .sum();
        let size_of_init: u64 = self
            .out
            .iter()
            .filter(|section| {
                section.chars & SCN_CNT_CODE == 0 && section.chars & SCN_CNT_UNINITIALIZED_DATA == 0
            })
            .map(|section| section.raw_size)
            .sum();
        let size_of_bss: u64 = self
            .out
            .iter()
            .filter(|section| section.chars & SCN_CNT_UNINITIALIZED_DATA != 0)
            .map(|section| section.memsz)
            .sum();
        let mut file = vec![0u8; raw as usize];
        file[0] = b'M';
        file[1] = b'Z';
        write_u32(&mut file, 0x3c, 0x80);
        let stub = [
            0x0e, 0x1f, 0xba, 0x0e, 0x00, 0xb4, 0x09, 0xcd, 0x21, 0xb8, 0x01, 0x4c, 0xcd, 0x21,
        ];
        file[0x40..0x40 + stub.len()].copy_from_slice(&stub);
        file[0x80..0x84].copy_from_slice(b"PE\0\0");
        let coff = 0x84;
        write_u16(&mut file, coff, MACHINE_AMD64);
        write_u16(&mut file, coff + 2, section_count as u16);
        write_u32(&mut file, coff + 4, 0);
        write_u32(&mut file, coff + 8, 0);
        write_u32(&mut file, coff + 12, 0);
        write_u16(&mut file, coff + 16, 240);
        write_u16(&mut file, coff + 18, 0x0022);
        let opt = coff + 20;
        write_u16(&mut file, opt, 0x20b);
        file[opt + 2] = 0;
        file[opt + 3] = 0;
        write_u32(&mut file, opt + 4, size_of_code as u32);
        write_u32(&mut file, opt + 8, size_of_init as u32);
        write_u32(&mut file, opt + 12, size_of_bss as u32);
        write_u32(&mut file, opt + 16, entry as u32);
        let text = self.out.iter().find(|section| section.name == ".text");
        write_u32(&mut file, opt + 20, text.map(|s| s.rva as u32).unwrap_or(0));
        write_u64(&mut file, opt + 24, IMAGE_BASE);
        write_u32(&mut file, opt + 32, SECTION_ALIGN as u32);
        write_u32(&mut file, opt + 36, FILE_ALIGN as u32);
        write_u16(&mut file, opt + 40, 6);
        write_u16(&mut file, opt + 42, 0);
        write_u16(&mut file, opt + 44, 0);
        write_u16(&mut file, opt + 46, 0);
        write_u16(&mut file, opt + 48, 6);
        write_u16(&mut file, opt + 50, 0);
        write_u32(&mut file, opt + 52, 0);
        write_u32(&mut file, opt + 56, image_size as u32);
        write_u32(&mut file, opt + 60, headers_size as u32);
        write_u32(&mut file, opt + 64, 0);
        write_u16(&mut file, opt + 68, self.subsystem);
        write_u16(&mut file, opt + 70, 0x0100);
        write_u64(&mut file, opt + 72, 0x10_0000);
        write_u64(&mut file, opt + 80, 0x1000);
        write_u64(&mut file, opt + 88, 0x10_0000);
        write_u64(&mut file, opt + 96, 0x1000);
        write_u32(&mut file, opt + 104, 0);
        write_u32(&mut file, opt + 108, 16);
        let dirs = opt + 112;
        if idata.0 != 0 {
            write_u32(&mut file, dirs + 8, idata.0 as u32);
            write_u32(&mut file, dirs + 12, idata.1 as u32);
        }
        if pdata.0 != 0 {
            write_u32(&mut file, dirs + 24, pdata.0 as u32);
            write_u32(&mut file, dirs + 28, pdata.1 as u32);
        }
        if reloc.0 != 0 {
            write_u32(&mut file, dirs + 40, reloc.0 as u32);
            write_u32(&mut file, dirs + 44, reloc.1 as u32);
        }
        let mut at = section_table;
        for (index, section) in self.out.iter().enumerate() {
            let mut name = [0u8; 8];
            if let Some(offset) = name_table.1[index] {
                let text = format!("/{offset}");
                let bytes = text.as_bytes();
                let n = bytes.len().min(8);
                name[..n].copy_from_slice(&bytes[..n]);
            } else {
                let bytes = section.name.as_bytes();
                let n = bytes.len().min(8);
                name[..n].copy_from_slice(&bytes[..n]);
            }
            file[at..at + 8].copy_from_slice(&name);
            write_u32(
                &mut file,
                at + 8,
                section.memsz.max(section.data.len() as u64) as u32,
            );
            write_u32(&mut file, at + 12, section.rva as u32);
            write_u32(&mut file, at + 16, section.raw_size as u32);
            write_u32(&mut file, at + 20, section.raw_off as u32);
            write_u32(&mut file, at + 24, 0);
            write_u32(&mut file, at + 28, 0);
            write_u16(&mut file, at + 32, 0);
            write_u16(&mut file, at + 34, 0);
            write_u32(&mut file, at + 36, section.chars);
            at += 40;
        }
        if !name_table.0.is_empty() {
            let start = section_table + section_count * 40;
            file[start..start + name_table.0.len()].copy_from_slice(&name_table.0);
        }
        for section in &self.out {
            if section.raw_size == 0 {
                continue;
            }
            let start = section.raw_off as usize;
            let end = start + section.data.len();
            file[start..end].copy_from_slice(&section.data);
        }
        std::fs::write(output, &file)
            .map_err(|e| LinkError::Native(format!("cannot write {}: {e}", output.display())))?;
        Ok(PeStats {
            image_size,
            imports: self.used_imports.len(),
            base_relocations: base_relocs,
            sections: section_count,
        })
    }

    fn drop_metadata(&mut self) {
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                if is_metadata_section(&self.objects[oi].sections[si].name) {
                    self.kept[oi][si] = false;
                }
            }
        }
    }

    fn run(mut self, output: &Path) -> Result<PeStats, LinkError> {
        self.pull_archive_members();
        self.kept = self
            .objects
            .iter()
            .map(|obj| vec![true; obj.sections.len()])
            .collect();
        self.placement = self
            .objects
            .iter()
            .map(|obj| vec![None; obj.sections.len()])
            .collect();
        self.drop_metadata();
        self.dedupe_comdat()?;
        self.merge_sections()?;
        self.allocate_commons()?;
        self.build_defs()?;
        self.collect_imports()?;
        self.assign_rvas()?;
        let idata = self.build_idata()?;
        self.fill_thunks()?;
        let relocs = self.apply_relocations()?;
        self.sort_pdata();
        let base_reloc_count = relocs.len();
        let reloc = self.add_reloc_section(relocs)?;
        let entry = self.entry_rva()?;
        let pdata = self
            .out
            .iter()
            .find(|section| section.name == ".pdata")
            .map(|section| (section.rva, section.memsz))
            .unwrap_or((0, 0));
        self.write_pe(pdata, idata, reloc, entry, base_reloc_count, output)
    }
}

impl Target {
    fn section(self) -> Option<usize> {
        match self {
            Target::Rva { section, .. } => section,
            Target::Absolute(_) => None,
        }
    }
}

pub fn link(
    inputs: &[PeInput],
    output: &Path,
    entry: &str,
    subsystem: u16,
) -> Result<PeStats, LinkError> {
    let mut objects = Vec::new();
    let mut pool = Vec::new();
    let mut imports = Vec::new();
    for input in inputs {
        let (label, bytes) = match input {
            PeInput::Object(bytes) => ("<object>".to_string(), bytes.clone()),
            PeInput::ObjectPath(path) => (
                path.display().to_string(),
                std::fs::read(path).map_err(|e| {
                    LinkError::Native(format!("cannot read {}: {e}", path.display()))
                })?,
            ),
            PeInput::Library(bytes) => ("<library>".to_string(), bytes.clone()),
            PeInput::LibraryPath(path) => (
                path.display().to_string(),
                std::fs::read(path).map_err(|e| {
                    LinkError::Native(format!("cannot read {}: {e}", path.display()))
                })?,
            ),
        };
        match input {
            PeInput::Object(_) | PeInput::ObjectPath(_) => {
                objects.push(parse_object(&label, &bytes)?);
            }
            PeInput::Library(_) | PeInput::LibraryPath(_) => {
                let library = parse_library(&label, &bytes)?;
                imports.extend(library.imports);
                pool.extend(library.objects);
            }
        }
    }
    Linker::new(objects, pool, imports, entry, subsystem).run(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn import_header(sig1: u16, sig2: u16, data: &[u8], kind: u16) -> Vec<u8> {
        let mut bytes = vec![0u8; 20 + data.len()];
        write_u16(&mut bytes, 0, sig1);
        write_u16(&mut bytes, 2, sig2);
        write_u16(&mut bytes, 4, 0);
        write_u16(&mut bytes, 6, MACHINE_AMD64);
        write_u32(&mut bytes, 8, 0);
        write_u32(&mut bytes, 12, data.len() as u32);
        write_u16(&mut bytes, 16, 0);
        write_u16(&mut bytes, 18, kind);
        bytes[20..].copy_from_slice(data);
        bytes
    }

    #[test]
    fn short_import_member_parses() {
        let data = b"ExitProcess\0kernel32.dll\0";
        // type 0 (code), name type 1 (name)
        let bytes = import_header(0, 0xffff, data, 1 << 2);
        let import = parse_import_member(&bytes).unwrap().unwrap();
        assert_eq!(import.dll, "kernel32.dll");
        assert_eq!(import.export, "ExitProcess");
        assert!(import.code);
    }

    #[test]
    fn long_import_member_parses() {
        let data = b"__imp_ExitProcess\0kernel32.dll\0ExitProcess\0";
        let bytes = import_header(0xffff, 0, data, 1 << 2);
        let import = parse_import_member(&bytes).unwrap().unwrap();
        assert_eq!(import.dll, "kernel32.dll");
        assert_eq!(import.export, "ExitProcess");
    }

    #[test]
    fn non_import_members_are_ignored() {
        let bytes = import_header(0x1234, 0x5678, b"", 0);
        assert!(parse_import_member(&bytes).unwrap().is_none());
    }
}
