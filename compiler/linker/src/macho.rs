use crate::LinkError;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

const MH_MAGIC_64: u32 = 0xfeed_facf;
const MH_OBJECT: u32 = 1;
const MH_EXECUTE: u32 = 2;
const CPU_TYPE_X86_64: u32 = 0x0100_0007;
const CPU_TYPE_ARM64: u32 = 0x0100_000c;
const CPU_SUBTYPE_X86_64_ALL: u32 = 0x8000_0003;
const CPU_SUBTYPE_ARM64_ALL: u32 = 0;

const LC_SEGMENT_64: u32 = 0x19;
const LC_SYMTAB: u32 = 0x2;
const LC_DYSYMTAB: u32 = 0xb;
const LC_LOAD_DYLINKER: u32 = 0xe;
const LC_LOAD_DYLIB: u32 = 0xc;
const LC_MAIN: u32 = 0x8000_0028;
const LC_BUILD_VERSION: u32 = 0x32;
const LC_DYLD_INFO_ONLY: u32 = 0x8000_0022;
const LC_CODE_SIGNATURE: u32 = 0x1d;

const S_ZEROFILL: u32 = 0x1;
const S_THREAD_LOCAL_REGULAR: u32 = 0x11;
const S_THREAD_LOCAL_ZEROFILL: u32 = 0x12;
const S_THREAD_LOCAL_VARIABLES: u32 = 0x13;
const S_ATTR_PURE_INSTRUCTIONS: u32 = 0x8000_0000;
const S_ATTR_SOME_INSTRUCTIONS: u32 = 0x0000_0400;

const N_STAB: u8 = 0xe0;
const N_TYPE: u8 = 0x0e;
const N_EXT: u8 = 0x01;
const N_UNDF: u8 = 0x0;
const N_ABS: u8 = 0x2;
const N_SECT: u8 = 0xe;
const N_WEAK_REF: u16 = 0x0040;
const N_WEAK_DEF: u16 = 0x0080;

const BASE_ADDR: u64 = 0x1_0000_0000;
const PAGEZERO_SIZE: u64 = 0x1_0000_0000;

#[derive(Debug)]
pub enum MachInput {
    Object(Vec<u8>),
    ObjectPath(PathBuf),
    Library(Vec<u8>),
    LibraryPath(PathBuf),
    Tbd(String, String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MachStats {
    pub image_size: u64,
    pub sections: usize,
    pub imports: usize,
    pub symbols: usize,
    pub code_signature_size: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Arch {
    Arm64,
    X86_64,
}

impl Arch {
    fn cputype(self) -> u32 {
        match self {
            Arch::Arm64 => CPU_TYPE_ARM64,
            Arch::X86_64 => CPU_TYPE_X86_64,
        }
    }

    fn cpusubtype(self) -> u32 {
        match self {
            Arch::Arm64 => CPU_SUBTYPE_ARM64_ALL,
            Arch::X86_64 => CPU_SUBTYPE_X86_64_ALL,
        }
    }

    fn page_size(self) -> u64 {
        match self {
            Arch::Arm64 => 0x4000,
            Arch::X86_64 => 0x1000,
        }
    }

    fn min_version(self) -> u32 {
        match self {
            Arch::Arm64 => 0x000b_0000,
            Arch::X86_64 => 0x000a_0f00,
        }
    }

    fn stub_size(self) -> u64 {
        match self {
            Arch::Arm64 => 12,
            Arch::X86_64 => 6,
        }
    }
}

fn section_type(flags: u32) -> u32 {
    flags & 0xff
}

fn is_thread_data(flags: u32) -> bool {
    matches!(
        section_type(flags),
        S_THREAD_LOCAL_REGULAR | S_THREAD_LOCAL_ZEROFILL
    )
}

fn is_thread_vars(flags: u32) -> bool {
    section_type(flags) == S_THREAD_LOCAL_VARIABLES
}

fn unsupported(message: impl Into<String>) -> LinkError {
    LinkError::Unsupported(message.into())
}

fn read_u32(bytes: &[u8], at: usize) -> Result<u32, LinkError> {
    let raw = bytes
        .get(at..at + 4)
        .ok_or_else(|| unsupported("truncated Mach-O field"))?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn read_u64(bytes: &[u8], at: usize) -> Result<u64, LinkError> {
    let raw = bytes
        .get(at..at + 8)
        .ok_or_else(|| unsupported("truncated Mach-O field"))?;
    Ok(u64::from_le_bytes([
        raw[0], raw[1], raw[2], raw[3], raw[4], raw[5], raw[6], raw[7],
    ]))
}

fn read_i32(bytes: &[u8], at: usize) -> Result<i32, LinkError> {
    Ok(read_u32(bytes, at)? as i32)
}

fn cstr(bytes: &[u8], at: usize) -> Result<String, LinkError> {
    let rest = bytes
        .get(at..)
        .ok_or_else(|| unsupported("string out of range"))?;
    let end = rest.iter().position(|&b| b == 0).unwrap_or(rest.len());
    Ok(String::from_utf8_lossy(&rest[..end]).into_owned())
}

fn put_u32(dst: &mut [u8], at: usize, value: u32) {
    dst[at..at + 4].copy_from_slice(&value.to_le_bytes());
}

fn align_up(value: u64, align: u64) -> u64 {
    if align <= 1 {
        return value;
    }
    value.div_ceil(align) * align
}

fn name16(buf: &[u8]) -> String {
    let text = String::from_utf8_lossy(buf);
    text.trim_end_matches(['\0', ' ']).to_string()
}

#[derive(Debug)]
struct Reloc {
    address: u32,
    symbol: u32,
    length: u8,
    external: bool,
    kind: u8,
}

#[derive(Debug)]
struct Section {
    seg: String,
    name: String,
    addr: u64,
    size: u64,
    data: Vec<u8>,
    align: u32,
    flags: u32,
    relocs: Vec<Reloc>,
}

#[derive(Debug)]
struct Symbol {
    name: String,
    n_type: u8,
    n_sect: u8,
    n_desc: u16,
    n_value: u64,
}

#[derive(Debug)]
struct Object {
    label: String,
    arch: Arch,
    sections: Vec<Section>,
    symbols: Vec<Symbol>,
}

fn parse_object(label: &str, bytes: &[u8]) -> Result<Object, LinkError> {
    if bytes.len() < 32 {
        return Err(unsupported(format!("{label}: truncated Mach-O header")));
    }
    let magic = read_u32(bytes, 0)?;
    if magic != MH_MAGIC_64 {
        return Err(unsupported(format!("{label}: not a 64-bit Mach-O object")));
    }
    let cputype = read_u32(bytes, 4)?;
    let filetype = read_u32(bytes, 12)?;
    let ncmds = read_u32(bytes, 16)? as usize;
    let sizeofcmds = read_u32(bytes, 20)? as usize;
    if filetype != MH_OBJECT {
        return Err(unsupported(format!("{label}: not a Mach-O object file")));
    }
    let arch = match cputype {
        CPU_TYPE_ARM64 => Arch::Arm64,
        CPU_TYPE_X86_64 => Arch::X86_64,
        other => {
            return Err(unsupported(format!(
                "{label}: unsupported Mach-O cputype {other:#x}"
            )));
        }
    };
    let commands_end = 32usize
        .checked_add(sizeofcmds)
        .ok_or_else(|| unsupported(format!("{label}: load command size overflow")))?;
    if commands_end > bytes.len() {
        return Err(unsupported(format!("{label}: load commands out of range")));
    }
    let mut sections = Vec::new();
    let mut symbol_offset = 0usize;
    let mut symbol_count = 0usize;
    let mut string_offset = 0usize;
    let mut string_size = 0usize;
    let mut at = 32usize;
    for _ in 0..ncmds {
        let cmd = read_u32(bytes, at)?;
        let cmdsize = read_u32(bytes, at + 4)? as usize;
        if cmdsize < 8 || at + cmdsize > bytes.len() {
            return Err(unsupported(format!("{label}: bad load command")));
        }
        match cmd {
            LC_SEGMENT_64 => {
                let nsects = read_u32(bytes, at + 64)? as usize;
                let needed =
                    72usize
                        .checked_add(nsects.checked_mul(80).ok_or_else(|| {
                            unsupported(format!("{label}: section count overflow"))
                        })?)
                        .ok_or_else(|| unsupported(format!("{label}: section count overflow")))?;
                if needed > cmdsize {
                    return Err(unsupported(format!(
                        "{label}: segment command is shorter than its section table"
                    )));
                }
                let mut sect_at = at + 72;
                for _ in 0..nsects {
                    let sectname = name16(&bytes[sect_at..sect_at + 16]);
                    let segname = name16(&bytes[sect_at + 16..sect_at + 32]);
                    let addr = read_u64(bytes, sect_at + 32)?;
                    let size = read_u64(bytes, sect_at + 40)?;
                    let offset = read_u32(bytes, sect_at + 48)? as usize;
                    let align = read_u32(bytes, sect_at + 52)?;
                    let reloff = read_u32(bytes, sect_at + 56)? as usize;
                    let nreloc = read_u32(bytes, sect_at + 60)? as usize;
                    let flags = read_u32(bytes, sect_at + 64)?;
                    let is_zerofill =
                        matches!(section_type(flags), S_ZEROFILL | S_THREAD_LOCAL_ZEROFILL);
                    let data = if is_zerofill || size == 0 {
                        Vec::new()
                    } else {
                        bytes
                            .get(offset..offset + size as usize)
                            .ok_or_else(|| {
                                unsupported(format!("{label}: section data out of range"))
                            })?
                            .to_vec()
                    };
                    let mut relocs = Vec::new();
                    for r in 0..nreloc {
                        let r_at = reloff
                            .checked_add(r.checked_mul(8).ok_or_else(|| {
                                unsupported(format!("{label}: relocation overflow"))
                            })?)
                            .ok_or_else(|| unsupported(format!("{label}: relocation overflow")))?;
                        let address = read_i32(bytes, r_at)?;
                        if address < 0 {
                            return Err(unsupported(format!(
                                "{label}: scattered Mach-O relocations are not supported"
                            )));
                        }
                        let info = read_u32(bytes, r_at + 4)?;
                        let symbol = info & 0x00ff_ffff;
                        let length = ((info >> 25) & 3) as u8;
                        let external = (info >> 27) & 1 != 0;
                        let kind = ((info >> 28) & 0xf) as u8;
                        relocs.push(Reloc {
                            address: address as u32,
                            symbol,
                            length,
                            external,
                            kind,
                        });
                    }
                    sections.push(Section {
                        seg: segname,
                        name: sectname,
                        addr,
                        size,
                        data,
                        align,
                        flags,
                        relocs,
                    });
                    sect_at += 80;
                }
            }
            LC_SYMTAB => {
                symbol_offset = read_u32(bytes, at + 8)? as usize;
                symbol_count = read_u32(bytes, at + 12)? as usize;
                string_offset = read_u32(bytes, at + 16)? as usize;
                string_size = read_u32(bytes, at + 20)? as usize;
            }
            _ => {}
        }
        at += cmdsize;
    }
    let strings_end = string_offset
        .checked_add(string_size)
        .ok_or_else(|| unsupported(format!("{label}: string table overflow")))?;
    let strings = bytes.get(string_offset..strings_end).unwrap_or_default();
    let mut symbols = Vec::with_capacity(symbol_count);
    for i in 0..symbol_count {
        let at = symbol_offset
            .checked_add(
                i.checked_mul(16)
                    .ok_or_else(|| unsupported(format!("{label}: symbol table overflow")))?,
            )
            .ok_or_else(|| unsupported(format!("{label}: symbol table overflow")))?;
        let n_strx = read_u32(bytes, at)? as usize;
        let n_type = bytes
            .get(at + 4)
            .copied()
            .ok_or_else(|| unsupported(format!("{label}: symbol out of range")))?;
        let n_sect = bytes.get(at + 5).copied().unwrap_or(0);
        let n_desc = u16::from_le_bytes([
            bytes.get(at + 6).copied().unwrap_or(0),
            bytes.get(at + 7).copied().unwrap_or(0),
        ]);
        let n_value = read_u64(bytes, at + 8)?;
        let name = if n_strx < strings.len() {
            cstr(strings, n_strx)?
        } else {
            String::new()
        };
        symbols.push(Symbol {
            name,
            n_type,
            n_sect,
            n_desc,
            n_value,
        });
    }
    Ok(Object {
        label: label.to_string(),
        arch,
        sections,
        symbols,
    })
}

fn archive_members(bytes: &[u8], label: &str) -> Result<Vec<Vec<u8>>, LinkError> {
    if !bytes.starts_with(b"!<arch>\n") {
        return Err(unsupported(format!("{label}: not an archive")));
    }
    let mut out = Vec::new();
    let mut at = 8usize;
    while at + 60 <= bytes.len() {
        let header = &bytes[at..at + 60];
        let raw_name = name16(&header[0..16]);
        let size_text = String::from_utf8_lossy(&header[48..58]).trim().to_string();
        let size = size_text
            .parse::<usize>()
            .map_err(|_| unsupported(format!("{label}: bad archive member size")))?;
        let data_at = at + 60;
        let mut data = bytes
            .get(data_at..data_at + size)
            .ok_or_else(|| unsupported(format!("{label}: archive member out of range")))?
            .to_vec();
        let name = if let Some(length) = raw_name
            .strip_prefix("#1/")
            .and_then(|n| n.trim().parse::<usize>().ok())
        {
            let end = length.min(data.len());
            let name = String::from_utf8_lossy(&data[..end])
                .trim_end_matches('\0')
                .to_string();
            data.drain(..end);
            name
        } else {
            raw_name
        };
        if !name.starts_with("__.") && !name.starts_with('/') {
            out.push(data);
        }
        at = data_at + size;
        if at % 2 == 1 {
            at += 1;
        }
    }
    Ok(out)
}

#[derive(Debug, Clone)]
struct Tbd {
    install_name: String,
    symbols: BTreeSet<String>,
}

fn clean_tbd_item(item: &str) -> String {
    item.trim()
        .trim_matches('"')
        .trim_matches('\'')
        .trim_end_matches(',')
        .trim()
        .to_string()
}

fn parse_symbol_body(body: &str, symbols: &mut BTreeSet<String>) {
    if let Some((_, rest)) = body.split_once('[') {
        let list = rest.split(']').next().unwrap_or(rest);
        for item in list.split(',') {
            let item = clean_tbd_item(item);
            if !item.is_empty() {
                symbols.insert(item);
            }
        }
    }
}

fn parse_tbd(label: &str, text: &str) -> Result<Tbd, LinkError> {
    let mut install_name = None;
    let mut symbols = BTreeSet::new();
    let mut lines = text.lines().peekable();
    while let Some(raw) = lines.next() {
        let line = raw.trim();
        if line.is_empty() || line.starts_with('#') || line == "..." || line.starts_with("---") {
            continue;
        }
        if let Some(value) = line.strip_prefix("install-name:") {
            install_name = Some(clean_tbd_item(value));
            continue;
        }
        let list_body = line
            .strip_prefix("symbols:")
            .or_else(|| line.strip_prefix("reexported-symbols:"));
        if let Some(mut body) = list_body.map(str::to_string) {
            if body.contains('[') {
                while body.matches('[').count() > body.matches(']').count() {
                    let Some(next) = lines.next() else {
                        break;
                    };
                    body.push(' ');
                    body.push_str(next);
                }
                parse_symbol_body(&body, &mut symbols);
            } else {
                while let Some(next) = lines.peek() {
                    let next = next.trim();
                    let Some(item) = next.strip_prefix("- ") else {
                        break;
                    };
                    if item.contains(':') || item.starts_with('[') {
                        break;
                    }
                    lines.next();
                    let item = clean_tbd_item(item);
                    if !item.is_empty() {
                        symbols.insert(item);
                    }
                }
            }
            continue;
        }
        if line.starts_with("targets:") || line.starts_with("tbd-version:") {
            continue;
        }
    }
    let install_name = install_name
        .filter(|name| !name.is_empty())
        .ok_or_else(|| unsupported(format!("{label}: .tbd stub has no install-name")))?;
    if symbols.is_empty() {
        return Err(unsupported(format!(
            "{label}: .tbd stub exports no symbols"
        )));
    }
    Ok(Tbd {
        install_name,
        symbols,
    })
}

#[derive(Clone, Copy)]
struct Placement {
    out: usize,
    off: u64,
}

struct OutSection {
    seg: String,
    name: String,
    align: u32,
    data: Vec<u8>,
    memsz: u64,
    zerofill: bool,
    flags: u32,
    addr: u64,
    file_offset: u64,
}

struct OutSegment {
    name: String,
    vmaddr: u64,
    vmsize: u64,
    fileoff: u64,
    filesize: u64,
    maxprot: u32,
    initprot: u32,
    sections: Vec<usize>,
}

struct Import {
    got: u64,
    stub: u64,
    code: bool,
    dylib: u8,
}

struct BindSite {
    name: String,
    dylib: u8,
    segment: usize,
    offset: u64,
}

struct Linker {
    arch: Arch,
    objects: Vec<Object>,
    tbd: Vec<Tbd>,
    placement: Vec<Vec<Option<Placement>>>,
    out: Vec<OutSection>,
    defs: BTreeMap<String, (usize, usize)>,
    commons: BTreeMap<(usize, usize), (usize, u64)>,
    undefined: BTreeSet<String>,
    imports: BTreeMap<String, Import>,
    bind_sites: Vec<BindSite>,
    thread_data_base: u64,
    entry: String,
    segments: Vec<OutSegment>,
    bind_offset: u64,
    bind_size: u64,
    rebase_offset: u64,
    rebase_size: u64,
    symbol_offset: u64,
    symbol_count: u32,
    string_offset: u64,
    string_size: u64,
    signature_offset: u64,
    signature_size: u64,
    local_count: u32,
    extdef_count: u32,
    undef_count: u32,
    symbol_bytes: Vec<u8>,
    string_bytes: Vec<u8>,
    bind_bytes: Vec<u8>,
    rebase_bytes: Vec<u8>,
}

impl Linker {
    fn new(objects: Vec<Object>, tbd: Vec<Tbd>, entry: &str) -> Result<Linker, LinkError> {
        let arch = objects
            .first()
            .map(|object| object.arch)
            .ok_or_else(|| unsupported("no Mach-O objects to link"))?;
        if objects.iter().any(|object| object.arch != arch) {
            return Err(unsupported("mixed Mach-O architectures"));
        }
        let placement = objects
            .iter()
            .map(|object| vec![None; object.sections.len()])
            .collect();
        Ok(Linker {
            arch,
            objects,
            tbd,
            placement,
            out: Vec::new(),
            defs: BTreeMap::new(),
            commons: BTreeMap::new(),
            undefined: BTreeSet::new(),
            imports: BTreeMap::new(),
            bind_sites: Vec::new(),
            thread_data_base: 0,
            entry: entry.to_string(),
            segments: Vec::new(),
            bind_offset: 0,
            bind_size: 0,
            rebase_offset: 0,
            rebase_size: 0,
            symbol_offset: 0,
            symbol_count: 0,
            string_offset: 0,
            string_size: 0,
            signature_offset: 0,
            signature_size: 0,
            local_count: 0,
            extdef_count: 0,
            undef_count: 0,
            symbol_bytes: Vec::new(),
            string_bytes: Vec::new(),
            bind_bytes: Vec::new(),
            rebase_bytes: Vec::new(),
        })
    }

    fn output_name(section: &Section) -> (String, String) {
        if section.seg == "__LD" {
            return ("__DATA".to_string(), section.name.clone());
        }
        (section.seg.clone(), section.name.clone())
    }

    fn ensure_section(&mut self, seg: &str, name: &str, align: u32, flags: u32) -> usize {
        if let Some(index) = self
            .out
            .iter()
            .position(|section| section.seg == seg && section.name == name)
        {
            return index;
        }
        self.out.push(OutSection {
            seg: seg.to_string(),
            name: name.to_string(),
            align,
            data: Vec::new(),
            memsz: 0,
            zerofill: matches!(section_type(flags), S_ZEROFILL | S_THREAD_LOCAL_ZEROFILL),
            flags,
            addr: 0,
            file_offset: 0,
        });
        self.out.len() - 1
    }

    fn merge_sections(&mut self) -> Result<(), LinkError> {
        let mut placements: Vec<Vec<Option<Placement>>> = self
            .objects
            .iter()
            .map(|object| vec![None; object.sections.len()])
            .collect();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                let (seg, name, size, align_pow, flags, data, zerofill) = {
                    let section = &self.objects[oi].sections[si];
                    let (seg, name) = Self::output_name(section);
                    (
                        seg,
                        name,
                        section.size,
                        section.align,
                        section.flags,
                        section.data.clone(),
                        matches!(
                            section_type(section.flags),
                            S_ZEROFILL | S_THREAD_LOCAL_ZEROFILL
                        ),
                    )
                };
                if size == 0 {
                    continue;
                }
                let align_pow = if name == "__thread_vars" {
                    align_pow.max(3).min(15)
                } else {
                    align_pow.min(15)
                };
                let align = 1u64 << align_pow;
                let index = self.ensure_section(&seg, &name, align_pow, flags);
                let out = &mut self.out[index];
                out.align = out.align.max(align_pow);
                let off = if out.zerofill || zerofill {
                    out.zerofill = true;
                    let off = align_up(out.memsz, align);
                    out.memsz = off + size;
                    off
                } else {
                    let off = align_up(out.data.len() as u64, align);
                    if off > out.data.len() as u64 {
                        out.data.resize(off as usize, 0);
                    }
                    out.data.extend_from_slice(&data);
                    out.memsz = out.data.len() as u64;
                    off
                };
                out.flags |=
                    flags & (S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS | S_ZEROFILL);
                placements[oi][si] = Some(Placement { out: index, off });
            }
        }
        self.placement = placements;
        Ok(())
    }

    fn collect_symbols(&mut self) -> Result<(), LinkError> {
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].symbols.len() {
                let symbol = &self.objects[oi].symbols[si];
                if symbol.n_type & N_STAB != 0 {
                    continue;
                }
                if symbol.name.is_empty() {
                    continue;
                }
                let kind = symbol.n_type & N_TYPE;
                let external = symbol.n_type & N_EXT != 0;
                let weak_ref = symbol.n_desc & N_WEAK_REF != 0;
                let weak_def = symbol.n_desc & N_WEAK_DEF != 0;
                let desc = symbol.n_desc;
                let value = symbol.n_value;
                let name = symbol.name.clone();
                if kind == N_UNDF {
                    if value > 0 {
                        // common symbol
                        let size = value;
                        let bss_pow = ((desc >> 8) & 0x0f).min(15) as u32;
                        let bss = self.ensure_section("__DATA", "__bss", 3, S_ZEROFILL);
                        self.out[bss].align = self.out[bss].align.max(bss_pow);
                        let align = 1u64 << bss_pow;
                        let off = align_up(self.out[bss].memsz, align);
                        self.out[bss].memsz = off + size;
                        self.commons.insert((oi, si), (bss, off));
                    } else if external && !weak_ref {
                        self.undefined.insert(name);
                    }
                    continue;
                }
                if kind == N_SECT {
                    if !external {
                        continue;
                    }
                    if let Some(existing) = self.defs.get(&name) {
                        if existing.0 == oi && existing.1 == si {
                            continue;
                        }
                        if weak_def {
                            continue;
                        }
                        let previous = &self.objects[existing.0].symbols[existing.1];
                        if previous.n_desc & N_WEAK_DEF != 0 {
                            self.defs.insert(name, (oi, si));
                            continue;
                        }
                        return Err(unsupported(format!(
                            "duplicate symbol `{}` in {} and {}",
                            name, self.objects[existing.0].label, self.objects[oi].label
                        )));
                    }
                    self.defs.insert(name, (oi, si));
                }
            }
        }
        self.undefined.retain(|name| !self.defs.contains_key(name));
        Ok(())
    }

    fn find_export(&self, name: &str) -> Option<(u8, &Tbd)> {
        self.tbd
            .iter()
            .enumerate()
            .find(|(_, tbd)| tbd.symbols.contains(name))
            .map(|(index, tbd)| ((index + 1) as u8, tbd))
    }

    fn classify_imports(&mut self) -> Result<(), LinkError> {
        let mut code_symbols: BTreeSet<String> = BTreeSet::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                if self.placement[oi][si].is_none() {
                    continue;
                }
                for reloc in &self.objects[oi].sections[si].relocs {
                    if !reloc.external {
                        continue;
                    }
                    let symbol = self.objects[oi]
                        .symbols
                        .get(reloc.symbol as usize)
                        .ok_or_else(|| unsupported("relocation names a missing symbol"))?;
                    if !self.undefined.contains(&symbol.name) {
                        continue;
                    }
                    let branch = match self.arch {
                        Arch::Arm64 => reloc.kind == 2,
                        Arch::X86_64 => reloc.kind == 2,
                    };
                    if branch {
                        code_symbols.insert(symbol.name.clone());
                    }
                }
            }
        }
        let mut names: Vec<String> = self.undefined.iter().cloned().collect();
        names.sort();
        for name in names {
            let Some((dylib, _)) = self.find_export(&name) else {
                return Err(unsupported(format!(
                    "undefined symbol `{name}` is not exported by any .tbd stub"
                )));
            };
            let code = code_symbols.contains(&name);
            self.imports.insert(
                name.clone(),
                Import {
                    got: 0,
                    stub: 0,
                    code,
                    dylib,
                },
            );
        }
        if !self.imports.is_empty() {
            let stubs = self.ensure_section(
                "__TEXT",
                "__stubs",
                2,
                S_ATTR_PURE_INSTRUCTIONS | S_ATTR_SOME_INSTRUCTIONS,
            );
            let stub_size = self.arch.stub_size();
            self.out[stubs].data = vec![0u8; self.imports.len() as usize * stub_size as usize];
            self.out[stubs].memsz = self.out[stubs].data.len() as u64;
            let got = self.ensure_section("__DATA", "__got", 3, 0);
            self.out[got].data = vec![0u8; self.imports.len() * 8];
            self.out[got].memsz = self.out[got].data.len() as u64;
        }
        Ok(())
    }

    fn order_sections(&self) -> Vec<Vec<usize>> {
        let order = [
            ("__TEXT", &["__text", "__eh_frame", "__stubs"][..]),
            ("__DATA_CONST", &["__const", "__cstring", "__objc_*"][..]),
            (
                "__DATA",
                &["__compact_unwind", "__data", "__got", "__bss"][..],
            ),
        ];
        let mut segments: Vec<(String, Vec<usize>)> = Vec::new();
        let mut names: Vec<String> = Vec::new();
        for section in &self.out {
            if !names.contains(&section.seg) {
                names.push(section.seg.clone());
            }
        }
        names.sort_by_key(|name| match name.as_str() {
            "__TEXT" => 0,
            "__DATA_CONST" => 1,
            "__DATA" => 2,
            _ => 3,
        });
        for name in names {
            let mut indexes: Vec<usize> = self
                .out
                .iter()
                .enumerate()
                .filter(|(_, section)| section.seg == name)
                .map(|(index, _)| index)
                .collect();
            let preferred = order
                .iter()
                .find(|(seg, _)| *seg == name)
                .map(|(_, names)| *names)
                .unwrap_or(&[]);
            indexes.sort_by_key(|&index| {
                let rank = preferred
                    .iter()
                    .position(|n| *n == self.out[index].name)
                    .unwrap_or(preferred.len());
                (self.out[index].zerofill, rank)
            });
            segments.push((name, indexes));
        }
        let mut result: Vec<Vec<usize>> = Vec::new();
        for (_, indexes) in segments {
            result.push(indexes);
        }
        result
    }

    fn dylib_names(&self) -> Vec<String> {
        if self.tbd.is_empty() {
            return vec!["/usr/lib/libSystem.B.dylib".to_string()];
        }
        self.tbd
            .iter()
            .map(|tbd| tbd.install_name.clone())
            .collect()
    }

    fn layout(&mut self) -> Result<(), LinkError> {
        let ordered = self.order_sections();
        let font = self.arch;
        let dylib_names = self.dylib_names();
        let has_const = ordered
            .iter()
            .any(|indexes| indexes.iter().any(|&i| self.out[i].seg == "__DATA_CONST"));
        let mut segment_names: Vec<&str> = vec!["__PAGEZERO", "__TEXT"];
        if has_const {
            segment_names.push("__DATA_CONST");
        }
        segment_names.push("__DATA");
        segment_names.push("__LINKEDIT");
        let mut sizeofcmds = 0u64;
        for name in &segment_names {
            let nsects = ordered
                .iter()
                .enumerate()
                .filter(|(i, _)| {
                    let seg = match *name {
                        "__PAGEZERO" | "__LINKEDIT" => false,
                        other => ordered[*i].iter().any(|&s| self.out[s].seg == other),
                    };
                    seg
                })
                .map(|(_, indexes)| indexes.len())
                .sum::<usize>();
            sizeofcmds += 72 + 80 * nsects as u64;
        }
        sizeofcmds += 24; // LC_MAIN
        sizeofcmds += align_up(12 + "/usr/lib/dyld".len() as u64 + 1, 8);
        for dylib_name in &dylib_names {
            sizeofcmds += align_up(24 + dylib_name.len() as u64 + 1, 8);
        }
        sizeofcmds += 24; // LC_BUILD_VERSION
        sizeofcmds += 48; // LC_DYLD_INFO_ONLY
        sizeofcmds += 24; // LC_SYMTAB
        sizeofcmds += 80; // LC_DYSYMTAB
        sizeofcmds += 16; // LC_CODE_SIGNATURE
        let header_size = align_up(32 + sizeofcmds, 16);
        if header_size > font.page_size() {
            return Err(unsupported("Mach-O load commands exceed one page"));
        }
        let cursor = header_size;
        let mut segments = Vec::new();
        segments.push(OutSegment {
            name: "__PAGEZERO".to_string(),
            vmaddr: 0,
            vmsize: PAGEZERO_SIZE,
            fileoff: 0,
            filesize: 0,
            maxprot: 0,
            initprot: 0,
            sections: Vec::new(),
        });
        let mut file_cursor = 0u64;
        let mut vm_cursor = BASE_ADDR;
        for name in &segment_names {
            if *name == "__PAGEZERO" {
                continue;
            }
            let sections: Vec<usize> = ordered
                .iter()
                .enumerate()
                .filter(|(i, _)| ordered[*i].iter().any(|&s| self.out[s].seg == *name))
                .flat_map(|(_, indexes)| indexes.iter().copied())
                .collect();
            let is_linkedit = *name == "__LINKEDIT";
            if is_linkedit {
                let seg = OutSegment {
                    name: name.to_string(),
                    vmaddr: align_up(vm_cursor, font.page_size()),
                    vmsize: 0,
                    fileoff: align_up(file_cursor, font.page_size()),
                    filesize: 0,
                    maxprot: 7,
                    initprot: 1,
                    sections: Vec::new(),
                };
                segments.push(seg);
                continue;
            }
            let start_file = if *name == "__TEXT" {
                0
            } else {
                align_up(file_cursor, font.page_size())
            };
            let start_vm = if *name == "__TEXT" {
                BASE_ADDR
            } else {
                align_up(vm_cursor, font.page_size())
            };
            let mut local_cursor = if *name == "__TEXT" { cursor } else { 0 };
            let mut file_end = local_cursor;
            for &section_index in &sections {
                let section = &mut self.out[section_index];
                let align = 1u64 << section.align.min(15);
                let off = align_up(local_cursor, align);
                section.addr = start_vm + off;
                if section.zerofill {
                    section.file_offset = 0;
                } else {
                    section.file_offset = start_file + off;
                    file_end = off + section.data.len() as u64;
                }
                local_cursor = off + section.memsz.max(section.data.len() as u64);
            }
            let filesize = if *name == "__TEXT" {
                align_up(file_end, 8)
            } else {
                align_up(file_end, font.page_size())
            };
            let vmsize = align_up(local_cursor, font.page_size());
            let (maxprot, initprot) = match *name {
                "__TEXT" => (7, 5),
                "__DATA_CONST" => (7, 1),
                _ => (7, 3),
            };
            let seg = OutSegment {
                name: name.to_string(),
                vmaddr: start_vm,
                vmsize,
                fileoff: start_file,
                filesize,
                maxprot,
                initprot,
                sections,
            };
            file_cursor = start_file + filesize;
            vm_cursor = start_vm + vmsize;
            segments.push(seg);
        }
        self.thread_data_base = self
            .out
            .iter()
            .filter(|section| is_thread_data(section.flags))
            .map(|section| section.addr)
            .min()
            .unwrap_or(0);
        self.segments = segments;
        Ok(())
    }

    fn assign_imports(&mut self) -> Result<(), LinkError> {
        if self.imports.is_empty() {
            return Ok(());
        }
        let stubs = self
            .out
            .iter()
            .position(|section| section.name == "__stubs")
            .ok_or_else(|| unsupported("missing __stubs section"))?;
        let got = self
            .out
            .iter()
            .position(|section| section.name == "__got")
            .ok_or_else(|| unsupported("missing __got section"))?;
        let mut names: Vec<String> = self.imports.keys().cloned().collect();
        names.sort();
        let code_names: Vec<String> = names
            .iter()
            .filter(|name| self.imports[*name].code)
            .cloned()
            .collect();
        for (index, name) in names.iter().enumerate() {
            let got_addr = self.out[got].addr + index as u64 * 8;
            let stub_index = code_names.iter().position(|n| n == name).unwrap_or(0);
            let stub_addr = self.out[stubs].addr + stub_index as u64 * self.arch.stub_size();
            if let Some(import) = self.imports.get_mut(name) {
                import.got = got_addr;
                import.stub = stub_addr;
            }
        }
        let stubs = self
            .out
            .iter()
            .position(|section| section.name == "__stubs")
            .ok_or_else(|| unsupported("missing __stubs section"))?;
        let mut data = std::mem::take(&mut self.out[stubs].data);
        for (name, import) in &self.imports {
            if !import.code {
                continue;
            }
            let stub_index = names
                .iter()
                .filter(|name| self.imports[*name].code)
                .position(|n| n == name)
                .unwrap_or(0);
            let field = stub_index * self.arch.stub_size() as usize;
            match self.arch {
                Arch::Arm64 => {
                    let adrp = adrp_word(import.stub, import.got);
                    let ldr = ldr64_word(import.got);
                    data[field..field + 4].copy_from_slice(&adrp.to_le_bytes());
                    data[field + 4..field + 8].copy_from_slice(&ldr.to_le_bytes());
                    data[field + 8..field + 12].copy_from_slice(&0xd61f_0200u32.to_le_bytes());
                }
                Arch::X86_64 => {
                    let disp = import.got as i64 - (import.stub as i64 + 6);
                    data[field] = 0xff;
                    data[field + 1] = 0x25;
                    data[field + 2..field + 6].copy_from_slice(&(disp as i32).to_le_bytes());
                }
            }
        }
        self.out[stubs].data = data;
        Ok(())
    }

    fn symbol_target(&self, oi: usize, symbol: usize) -> Result<(u64, Option<usize>), LinkError> {
        let symbol_index = symbol;
        if let Some(&(out, off)) = self.commons.get(&(oi, symbol_index)) {
            return Ok((self.out[out].addr + off, Some(out)));
        }
        let symbol = self.objects[oi]
            .symbols
            .get(symbol)
            .ok_or_else(|| unsupported("relocation names a missing symbol"))?;
        let kind = symbol.n_type & N_TYPE;
        if kind == N_SECT {
            let si = symbol.n_sect as usize - 1;
            let placement = self.placement[oi]
                .get(si)
                .and_then(|p| *p)
                .ok_or_else(|| unsupported(format!("symbol `{}` has no placement", symbol.name)))?;
            let object_section = self.objects[oi].sections.get(si).ok_or_else(|| {
                unsupported(format!("symbol `{}` names a missing section", symbol.name))
            })?;
            let within = symbol.n_value.wrapping_sub(object_section.addr);
            return Ok((
                self.out[placement.out].addr + placement.off + within,
                Some(placement.out),
            ));
        }
        if kind == N_ABS {
            return Ok((symbol.n_value, None));
        }
        if kind == N_UNDF {
            if let Some(&(doi, dsi)) = self.defs.get(&symbol.name)
                && (doi != oi || dsi != symbol_index)
            {
                return self.symbol_target(doi, dsi);
            }
            if let Some(import) = self.imports.get(&symbol.name) {
                return Ok((import.got, None));
            }
            return Err(unsupported(format!("undefined symbol `{}`", symbol.name)));
        }
        Err(unsupported("unsupported Mach-O symbol type"))
    }

    fn resolve_reloc_target(
        &self,
        oi: usize,
        reloc: &Reloc,
    ) -> Result<(u64, Option<usize>), LinkError> {
        if reloc.external {
            let symbol = self.objects[oi]
                .symbols
                .get(reloc.symbol as usize)
                .ok_or_else(|| unsupported("relocation names a missing symbol"))?;
            if self.undefined.contains(&symbol.name)
                && let Some(import) = self.imports.get(&symbol.name)
            {
                if import.code && reloc.kind == 2 {
                    return Ok((import.stub, None));
                }
                return Ok((import.got, None));
            }
            return self.symbol_target(oi, reloc.symbol as usize);
        }
        let ordinal = reloc.symbol as usize;
        let target_index = ordinal
            .checked_sub(1)
            .filter(|&index| index < self.objects[oi].sections.len())
            .ok_or_else(|| {
                unsupported(format!(
                    "{}: local relocation names section {} (sections {}, reloc at {:#x} kind {}, external {})",
                    self.objects[oi].label,
                    ordinal,
                    self.objects[oi].sections.len(),
                    reloc.address,
                    reloc.kind,
                    reloc.external
                ))
            })?;
        let placement = self.placement[oi]
            .get(target_index)
            .and_then(|p| *p)
            .ok_or_else(|| unsupported("local relocation names an unplaced section"))?;
        Ok((
            self.out[placement.out].addr + placement.off,
            Some(placement.out),
        ))
    }

    fn rebase_site(&self, out_section: usize, at: u64) -> Option<(usize, u64)> {
        self.segments
            .iter()
            .enumerate()
            .find(|(_, segment)| segment.sections.contains(&out_section))
            .and_then(|(index, segment)| {
                if segment.name == "__TEXT" || segment.name == "__LINKEDIT" {
                    return None;
                }
                Some((index, self.out[out_section].addr - segment.vmaddr + at))
            })
    }

    fn reloc_symbol_defined(&self, oi: usize, reloc: &Reloc) -> bool {
        if !reloc.external {
            return true;
        }
        let Some(symbol) = self.objects[oi].symbols.get(reloc.symbol as usize) else {
            return false;
        };
        matches!(symbol.n_type & N_TYPE, N_SECT | N_ABS)
            || self.defs.contains_key(&symbol.name)
            || self.commons.contains_key(&(oi, reloc.symbol as usize))
    }

    fn apply_relocations(&mut self) -> Result<Vec<(usize, u64)>, LinkError> {
        let mut rebases = Vec::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].sections.len() {
                let Some(place) = self.placement[oi][si] else {
                    continue;
                };
                let relocs = &self.objects[oi].sections[si].relocs;
                let mut index = 0usize;
                let mut pending_addend = 0i64;
                while index < relocs.len() {
                    let reloc = &relocs[index];
                    if matches!(self.arch, Arch::Arm64) && reloc.kind == 10 {
                        let raw = reloc.symbol & 0x00ff_ffff;
                        pending_addend = if raw & 0x0080_0000 != 0 {
                            (raw | 0xff00_0000) as i32 as i64
                        } else {
                            raw as i64
                        };
                        index += 1;
                        continue;
                    }
                    let subtractor_kind = match self.arch {
                        Arch::Arm64 => reloc.kind == 1,
                        Arch::X86_64 => reloc.kind == 5,
                    };
                    if subtractor_kind {
                        let next = relocs.get(index + 1).ok_or_else(|| {
                            unsupported("SUBTRACTOR relocation has no paired UNSIGNED relocation")
                        })?;
                        let subtractor = self.resolve_reloc_target(oi, reloc)?;
                        let unsigned = self.resolve_reloc_target(oi, next)?;
                        let at = (place.off + next.address as u64) as usize;
                        let width = 1usize << next.length;
                        let mut data = std::mem::take(&mut self.out[place.out].data);
                        let addend = read_width(&data, at, width)? as i64;
                        let value = (unsigned.0 as i64)
                            .wrapping_add(addend)
                            .wrapping_sub(subtractor.0 as i64)
                            .wrapping_sub(pending_addend);
                        pending_addend = 0;
                        write_width(&mut data, at, width, value as u64)?;
                        self.out[place.out].data = data;
                        index += 2;
                        continue;
                    }
                    let at = (place.off + reloc.address as u64) as usize;
                    let (mut target, target_section) = self.resolve_reloc_target(oi, reloc)?;
                    target = target.wrapping_add(pending_addend as u64);
                    pending_addend = 0;
                    let defined = self.reloc_symbol_defined(oi, reloc);
                    let p = self.out[place.out].addr + place.off + reloc.address as u64;
                    let mut data = std::mem::take(&mut self.out[place.out].data);
                    match self.arch {
                        Arch::Arm64 => self
                            .apply_arm64(reloc, &mut data, at, p, target, defined)
                            .map_err(|e| {
                            unsupported(format!(
                                "{}: section {} relocation {:#x} kind {}: {e}",
                                self.objects[oi].label, si, reloc.address, reloc.kind
                            ))
                        })?,
                        Arch::X86_64 => self
                            .apply_x86(reloc, &mut data, at, p, target, defined)
                            .map_err(|e| {
                                unsupported(format!(
                                    "{}: section {} relocation {:#x} kind {}: {e}",
                                    self.objects[oi].label, si, reloc.address, reloc.kind
                                ))
                            })?,
                    }
                    self.out[place.out].data = data;
                    let width = 1usize << reloc.length;
                    let thread_offset = width == 8
                        && reloc.kind == 0
                        && is_thread_vars(self.out[place.out].flags)
                        && target_section
                            .is_some_and(|section| is_thread_data(self.out[section].flags));
                    if thread_offset {
                        let value = target.wrapping_sub(self.thread_data_base);
                        self.out[place.out].data[at..at + 8].copy_from_slice(&value.to_le_bytes());
                        index += 1;
                        continue;
                    }
                    let import_bind = if width == 8 && reloc.kind == 0 && reloc.external && !defined
                    {
                        let symbol = &self.objects[oi].symbols[reloc.symbol as usize];
                        if self.undefined.contains(&symbol.name) {
                            self.imports
                                .get(&symbol.name)
                                .map(|import| (symbol.name.clone(), import.dylib))
                        } else {
                            None
                        }
                    } else {
                        None
                    };
                    if let Some((name, dylib)) = import_bind {
                        if let Some((segment, offset)) =
                            self.rebase_site(place.out, place.off + reloc.address as u64)
                        {
                            self.out[place.out].data[at..at + 8].fill(0);
                            self.bind_sites.push(BindSite {
                                name,
                                dylib,
                                segment,
                                offset,
                            });
                        }
                    } else if width == 8
                        && matches!(reloc.kind, 0 | 4)
                        && let Some(site) =
                            self.rebase_site(place.out, place.off + reloc.address as u64)
                    {
                        rebases.push(site);
                    }
                    index += 1;
                }
            }
        }
        rebases.sort();
        rebases.dedup();
        self.bind_sites
            .sort_by(|a, b| (a.segment, a.offset).cmp(&(b.segment, b.offset)));
        self.bind_sites
            .dedup_by(|a, b| a.segment == b.segment && a.offset == b.offset);
        Ok(rebases)
    }

    fn apply_arm64(
        &self,
        reloc: &Reloc,
        data: &mut [u8],
        at: usize,
        p: u64,
        target: u64,
        defined: bool,
    ) -> Result<(), LinkError> {
        match reloc.kind {
            0 => {
                let width = 1usize << reloc.length;
                let addend = read_width(data, at, width)?;
                write_width(data, at, width, target.wrapping_add(addend))?;
            }
            2 => {
                let word = read_insn(data, at)?;
                let delta = target as i64 - p as i64;
                if delta & 3 != 0 || !(-(1 << 27)..(1 << 27)).contains(&delta) {
                    return Err(unsupported("arm64 branch target out of range"));
                }
                let imm = ((delta >> 2) as u32) & 0x03ff_ffff;
                write_insn(data, at, (word & 0xfc00_0000) | imm)?;
            }
            3 | 5 | 8 => {
                let word = read_insn(data, at)?;
                let delta = ((target & !0xfff) as i64).wrapping_sub((p & !0xfff) as i64) >> 12;
                if !(-(1 << 20)..(1 << 20)).contains(&delta) {
                    return Err(unsupported("arm64 adrp target out of range"));
                }
                let imm = (delta as u32) & 0x1f_ffff;
                let encoded = (word & 0x9f00_001f) | ((imm & 3) << 29) | ((imm >> 2) << 5);
                write_insn(data, at, encoded)?;
            }
            4 | 6 | 9 => {
                let word = read_insn(data, at)?;
                let offset = (target & 0xfff) as u32;
                if reloc.kind == 6 && defined {
                    let rd = word & 0x1f;
                    let rn = (word >> 5) & 0x1f;
                    let add = 0x9100_0000 | (rn << 5) | rd | (offset << 10);
                    write_insn(data, at, add)?;
                } else if (word & 0x9f00_0000) == 0x9100_0000 || (word & 0x1f00_0000) == 0x1100_0000
                {
                    write_insn(data, at, (word & 0xffc0_03ff) | (offset << 10))?;
                } else {
                    let size = (word >> 30) & 3;
                    if offset & ((1 << size) - 1) != 0 {
                        return Err(unsupported("arm64 page offset alignment mismatch"));
                    }
                    let scaled = (offset >> size) & 0xfff;
                    write_insn(data, at, (word & 0xffc0_03ff) | (scaled << 10))?;
                }
            }
            7 => {
                let delta = target as i64 - p as i64;
                if delta != (delta as i32) as i64 {
                    return Err(unsupported("arm64 POINTER_TO_GOT out of range"));
                }
                write_width(data, at, 4, delta as u32 as u64)?;
            }
            10 => {}
            other => {
                return Err(unsupported(format!(
                    "unsupported arm64 Mach-O relocation {other}"
                )));
            }
        }
        Ok(())
    }

    fn apply_x86(
        &self,
        reloc: &Reloc,
        data: &mut [u8],
        at: usize,
        p: u64,
        target: u64,
        defined: bool,
    ) -> Result<(), LinkError> {
        match reloc.kind {
            0 => {
                let width = 1usize << reloc.length;
                let addend = read_width(data, at, width)?;
                write_width(data, at, width, target.wrapping_add(addend))?;
            }
            1 | 2 | 6 | 7 | 8 | 9 => {
                let extra = match reloc.kind {
                    6 => 1,
                    7 => 2,
                    8 => 4,
                    _ => 0,
                };
                let addend = read_u32(data, at)? as i32 as i64;
                let value = target as i64 + addend - (p as i64 + 4 + extra);
                if value != (value as i32) as i64 {
                    return Err(unsupported("x86_64 pcrel relocation out of range"));
                }
                write_width(data, at, 4, value as u32 as u64)?;
            }
            3 => {
                let addend = read_u32(data, at)? as i32 as i64;
                if defined && at >= 2 && data[at - 2] == 0x8b {
                    data[at - 2] = 0x8d;
                }
                let value = target as i64 + addend - (p as i64 + 4);
                if value != (value as i32) as i64 {
                    return Err(unsupported("x86_64 GOT_LOAD out of range"));
                }
                write_width(data, at, 4, value as u32 as u64)?;
            }
            4 => {
                write_width(data, at, 8, target)?;
            }
            other => {
                return Err(unsupported(format!(
                    "unsupported x86_64 Mach-O relocation {other}"
                )));
            }
        }
        Ok(())
    }

    fn entry_address(&self) -> Result<u64, LinkError> {
        for name in [self.entry.as_str(), "_start", "start", "_main", "main"] {
            if let Some(&(oi, si)) = self.defs.get(name) {
                let symbol = &self.objects[oi].symbols[si];
                let section_index = symbol.n_sect as usize - 1;
                let placement = self.placement[oi]
                    .get(section_index)
                    .and_then(|p| *p)
                    .ok_or_else(|| unsupported("entry symbol has no placement"))?;
                let object_section = self.objects[oi]
                    .sections
                    .get(section_index)
                    .ok_or_else(|| unsupported("entry symbol names a missing section"))?;
                let within = symbol.n_value.wrapping_sub(object_section.addr);
                return Ok(self.out[placement.out].addr + placement.off + within);
            }
        }
        Err(unsupported(format!(
            "entry symbol `{}` is not defined",
            self.entry
        )))
    }

    fn build_tables(&mut self, rebases: &[(usize, u64)]) -> Result<(), LinkError> {
        let mut section_ordinal: BTreeMap<usize, u8> = BTreeMap::new();
        let mut ordinal = 1u8;
        for segment in &self.segments {
            for &section in &segment.sections {
                section_ordinal.insert(section, ordinal);
                ordinal = ordinal.saturating_add(1);
            }
        }
        let mut locals: Vec<(String, u8, u8, u16, u64)> = Vec::new();
        let mut extdefs: Vec<(String, u8, u8, u16, u64)> = Vec::new();
        for oi in 0..self.objects.len() {
            for si in 0..self.objects[oi].symbols.len() {
                let symbol = &self.objects[oi].symbols[si];
                if symbol.n_type & N_STAB != 0 || symbol.name.is_empty() {
                    continue;
                }
                let kind = symbol.n_type & N_TYPE;
                let external = symbol.n_type & N_EXT != 0;
                if kind == N_SECT || kind == N_UNDF {
                    let is_common = self.commons.contains_key(&(oi, si));
                    if kind == N_UNDF && !is_common {
                        continue;
                    }
                    if !is_common
                        && self
                            .defs
                            .get(&symbol.name)
                            .is_some_and(|&(doi, dsi)| doi != oi || dsi != si)
                    {
                        continue;
                    }
                    let target = self.symbol_target(oi, si)?;
                    let out_section = target
                        .1
                        .ok_or_else(|| unsupported("defined symbol has no section"))?;
                    let n_sect = *section_ordinal
                        .get(&out_section)
                        .ok_or_else(|| unsupported("defined symbol section missing"))?;
                    let n_type = if external { N_SECT | N_EXT } else { N_SECT };
                    let entry = (symbol.name.clone(), n_type, n_sect, symbol.n_desc, target.0);
                    if external {
                        extdefs.push(entry);
                    } else {
                        locals.push(entry);
                    }
                } else if kind == N_ABS {
                    let n_type = if external { N_ABS | N_EXT } else { N_ABS };
                    let entry = (
                        symbol.name.clone(),
                        n_type,
                        0,
                        symbol.n_desc,
                        symbol.n_value,
                    );
                    if external {
                        extdefs.push(entry);
                    } else {
                        locals.push(entry);
                    }
                }
            }
        }
        locals.sort_by(|a, b| a.0.cmp(&b.0));
        extdefs.sort_by(|a, b| a.0.cmp(&b.0));
        let mut undefs: Vec<String> = self.undefined.iter().cloned().collect();
        undefs.sort();
        let local_count = locals.len() as u32;
        let extdef_count = extdefs.len() as u32;
        let undef_count = undefs.len() as u32;
        let mut symbols = locals;
        symbols.extend(extdefs);
        for name in &undefs {
            symbols.push((name.clone(), N_UNDF | N_EXT, 0, 0, 0));
        }
        let mut strings = vec![0u8];
        let mut string_index: BTreeMap<String, u32> = BTreeMap::new();
        let mut symbol_bytes = Vec::with_capacity(symbols.len() * 16);
        for (name, n_type, n_sect, n_desc, value) in &symbols {
            let offset = *string_index.entry(name.clone()).or_insert_with(|| {
                let offset = strings.len() as u32;
                strings.extend_from_slice(name.as_bytes());
                strings.push(0);
                offset
            });
            symbol_bytes.extend_from_slice(&offset.to_le_bytes());
            symbol_bytes.push(*n_type);
            symbol_bytes.push(*n_sect);
            symbol_bytes.extend_from_slice(&n_desc.to_le_bytes());
            symbol_bytes.extend_from_slice(&value.to_le_bytes());
        }
        let mut bind = Vec::new();
        if !self.imports.is_empty() || !self.bind_sites.is_empty() {
            let data_segment = self
                .segments
                .iter()
                .position(|segment| segment.name == "__DATA")
                .ok_or_else(|| unsupported("missing __DATA segment"))?;
            bind.push(0x51u8);
            for (name, import) in &self.imports {
                bind.push(0x10 | (import.dylib & 0x0f));
                bind.push(0x40u8);
                bind.extend_from_slice(name.as_bytes());
                bind.push(0);
                bind.push(0x70 | (data_segment as u8 & 0x0f));
                push_uleb(&mut bind, import.got - self.segments[data_segment].vmaddr);
                bind.push(0x90u8);
            }
            for site in &self.bind_sites {
                bind.push(0x10 | (site.dylib & 0x0f));
                bind.push(0x40u8);
                bind.extend_from_slice(site.name.as_bytes());
                bind.push(0);
                bind.push(0x70 | (site.segment as u8 & 0x0f));
                push_uleb(&mut bind, site.offset);
                bind.push(0x90u8);
            }
            bind.push(0x00u8);
        }
        let mut rebase = Vec::new();
        if !rebases.is_empty() {
            rebase.push(0x11u8);
            for (segment, offset) in rebases {
                rebase.push(0x20 | (*segment as u8 & 0x0f));
                push_uleb(&mut rebase, *offset);
                rebase.push(0x51u8);
            }
            rebase.push(0x00u8);
        }
        let linkedit = self
            .segments
            .iter()
            .position(|segment| segment.name == "__LINKEDIT")
            .ok_or_else(|| unsupported("missing __LINKEDIT segment"))?;
        let symoff = self.segments[linkedit].fileoff;
        let stroff = symoff + symbol_bytes.len() as u64;
        let bindoff = align_up(stroff + strings.len() as u64, 8);
        let rebaseoff = align_up(bindoff + bind.len() as u64, 8);
        let sigoff = align_up(rebaseoff + rebase.len() as u64, 16);
        let ident = b"rnx";
        let hash_slots = sigoff.div_ceil(4096);
        let signature_size = align_up(20 + 48 + ident.len() as u64 + 1 + hash_slots * 32, 16);
        let linkedit_size = sigoff - self.segments[linkedit].fileoff + signature_size;
        self.segments[linkedit].filesize = linkedit_size;
        self.segments[linkedit].vmsize = align_up(linkedit_size, self.arch.page_size());
        self.symbol_offset = symoff;
        self.symbol_count = symbols.len() as u32;
        self.string_offset = stroff;
        self.string_size = strings.len() as u64;
        self.bind_offset = bindoff;
        self.bind_size = bind.len() as u64;
        self.rebase_offset = rebaseoff;
        self.rebase_size = rebase.len() as u64;
        self.signature_offset = sigoff;
        self.signature_size = signature_size;
        self.local_count = local_count;
        self.extdef_count = extdef_count;
        self.undef_count = undef_count;
        self.symbol_bytes = symbol_bytes;
        self.string_bytes = strings;
        self.bind_bytes = bind;
        self.rebase_bytes = rebase;
        Ok(())
    }

    fn run(mut self, output: &Path) -> Result<MachStats, LinkError> {
        self.merge_sections()?;
        self.collect_symbols()?;
        self.classify_imports()?;
        self.layout()?;
        self.assign_imports()?;
        let rebases = self.apply_relocations()?;
        self.build_tables(&rebases)?;
        self.write(output)
    }

    fn write(&mut self, output: &Path) -> Result<MachStats, LinkError> {
        let linkedit = self
            .segments
            .iter()
            .position(|segment| segment.name == "__LINKEDIT")
            .ok_or_else(|| unsupported("missing __LINKEDIT segment"))?;
        let file_size = self.segments[linkedit].fileoff + self.segments[linkedit].filesize;
        let mut file = vec![0u8; file_size as usize];
        let entry = self.entry_address()?;
        let dylib_names = self.dylib_names();
        let mut ncmds = 0u32;
        let mut commands: Vec<u8> = Vec::new();
        for segment in &self.segments {
            let cmd_size = 72 + 80 * segment.sections.len() as u32;
            commands.extend_from_slice(&LC_SEGMENT_64.to_le_bytes());
            commands.extend_from_slice(&cmd_size.to_le_bytes());
            commands.extend_from_slice(&name_bytes(&segment.name));
            commands.extend_from_slice(&segment.vmaddr.to_le_bytes());
            commands.extend_from_slice(&segment.vmsize.to_le_bytes());
            commands.extend_from_slice(&segment.fileoff.to_le_bytes());
            commands.extend_from_slice(&segment.filesize.to_le_bytes());
            commands.extend_from_slice(&segment.maxprot.to_le_bytes());
            commands.extend_from_slice(&segment.initprot.to_le_bytes());
            commands.extend_from_slice(&(segment.sections.len() as u32).to_le_bytes());
            commands.extend_from_slice(&0u32.to_le_bytes());
            for &index in &segment.sections {
                let section = &self.out[index];
                commands.extend_from_slice(&name_bytes(&section.name));
                commands.extend_from_slice(&name_bytes(&section.seg));
                commands.extend_from_slice(&section.addr.to_le_bytes());
                commands
                    .extend_from_slice(&section.memsz.max(section.data.len() as u64).to_le_bytes());
                commands.extend_from_slice(&(section.file_offset as u32).to_le_bytes());
                commands.extend_from_slice(&section.align.to_le_bytes());
                commands.extend_from_slice(&0u32.to_le_bytes());
                commands.extend_from_slice(&0u32.to_le_bytes());
                commands.extend_from_slice(&section.flags.to_le_bytes());
                commands.extend_from_slice(&0u32.to_le_bytes());
                commands.extend_from_slice(&0u32.to_le_bytes());
                commands.extend_from_slice(&0u32.to_le_bytes());
            }
            ncmds += 1;
        }
        let text_vmaddr = self.segments[1].vmaddr;
        commands.extend_from_slice(&LC_MAIN.to_le_bytes());
        commands.extend_from_slice(&24u32.to_le_bytes());
        commands.extend_from_slice(&(entry - text_vmaddr).to_le_bytes());
        commands.extend_from_slice(&0u64.to_le_bytes());
        ncmds += 1;
        let dylinker = "/usr/lib/dyld";
        let mut dylinker_cmd = Vec::new();
        dylinker_cmd.extend_from_slice(&LC_LOAD_DYLINKER.to_le_bytes());
        let size = align_up(12 + dylinker.len() as u64 + 1, 8) as u32;
        dylinker_cmd.extend_from_slice(&size.to_le_bytes());
        dylinker_cmd.extend_from_slice(&12u32.to_le_bytes());
        dylinker_cmd.extend_from_slice(dylinker.as_bytes());
        dylinker_cmd.push(0);
        dylinker_cmd.resize(size as usize, 0);
        commands.extend_from_slice(&dylinker_cmd);
        ncmds += 1;
        for dylib_name in &dylib_names {
            let mut dylib_cmd = Vec::new();
            dylib_cmd.extend_from_slice(&LC_LOAD_DYLIB.to_le_bytes());
            let size = align_up(24 + dylib_name.len() as u64 + 1, 8) as u32;
            dylib_cmd.extend_from_slice(&size.to_le_bytes());
            dylib_cmd.extend_from_slice(&24u32.to_le_bytes());
            dylib_cmd.extend_from_slice(&0u32.to_le_bytes());
            dylib_cmd.extend_from_slice(&0u32.to_le_bytes());
            dylib_cmd.extend_from_slice(&0u32.to_le_bytes());
            dylib_cmd.extend_from_slice(dylib_name.as_bytes());
            dylib_cmd.push(0);
            dylib_cmd.resize(size as usize, 0);
            commands.extend_from_slice(&dylib_cmd);
            ncmds += 1;
        }
        commands.extend_from_slice(&LC_BUILD_VERSION.to_le_bytes());
        commands.extend_from_slice(&24u32.to_le_bytes());
        commands.extend_from_slice(&1u32.to_le_bytes());
        commands.extend_from_slice(&self.arch.min_version().to_le_bytes());
        commands.extend_from_slice(&self.arch.min_version().to_le_bytes());
        commands.extend_from_slice(&0u32.to_le_bytes());
        ncmds += 1;
        commands.extend_from_slice(&LC_DYLD_INFO_ONLY.to_le_bytes());
        commands.extend_from_slice(&48u32.to_le_bytes());
        commands.extend_from_slice(&(self.rebase_offset as u32).to_le_bytes());
        commands.extend_from_slice(&(self.rebase_size as u32).to_le_bytes());
        commands.extend_from_slice(&(self.bind_offset as u32).to_le_bytes());
        commands.extend_from_slice(&(self.bind_size as u32).to_le_bytes());
        for _ in 0..6 {
            commands.extend_from_slice(&0u32.to_le_bytes());
        }
        ncmds += 1;
        commands.extend_from_slice(&LC_SYMTAB.to_le_bytes());
        commands.extend_from_slice(&24u32.to_le_bytes());
        commands.extend_from_slice(&(self.symbol_offset as u32).to_le_bytes());
        commands.extend_from_slice(&self.symbol_count.to_le_bytes());
        commands.extend_from_slice(&(self.string_offset as u32).to_le_bytes());
        commands.extend_from_slice(&(self.string_size as u32).to_le_bytes());
        ncmds += 1;
        commands.extend_from_slice(&LC_DYSYMTAB.to_le_bytes());
        commands.extend_from_slice(&80u32.to_le_bytes());
        for value in [
            0u32,
            self.local_count,
            self.local_count,
            self.extdef_count,
            self.local_count + self.extdef_count,
            self.undef_count,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
            0,
        ] {
            commands.extend_from_slice(&value.to_le_bytes());
        }
        ncmds += 1;
        commands.extend_from_slice(&LC_CODE_SIGNATURE.to_le_bytes());
        commands.extend_from_slice(&16u32.to_le_bytes());
        commands.extend_from_slice(&(self.signature_offset as u32).to_le_bytes());
        commands.extend_from_slice(&(self.signature_size as u32).to_le_bytes());
        ncmds += 1;
        let first_text = self
            .segments
            .iter()
            .find(|segment| segment.name == "__TEXT")
            .and_then(|segment| segment.sections.first())
            .map(|&index| self.out[index].file_offset)
            .unwrap_or(u64::MAX);
        if 32 + commands.len() as u64 > first_text {
            return Err(unsupported("load commands overlap the first section"));
        }
        put_u32(&mut file, 0, MH_MAGIC_64);
        put_u32(&mut file, 4, self.arch.cputype());
        put_u32(&mut file, 8, self.arch.cpusubtype());
        put_u32(&mut file, 12, MH_EXECUTE);
        put_u32(&mut file, 16, ncmds);
        put_u32(&mut file, 20, commands.len() as u32);
        put_u32(&mut file, 24, 0x0020_0085);
        put_u32(&mut file, 28, 0);
        file[32..32 + commands.len()].copy_from_slice(&commands);
        for section in &self.out {
            if section.data.is_empty() {
                continue;
            }
            let start = section.file_offset as usize;
            let end = start + section.data.len();
            file[start..end].copy_from_slice(&section.data);
        }
        let sym_at = self.symbol_offset as usize;
        file[sym_at..sym_at + self.symbol_bytes.len()].copy_from_slice(&self.symbol_bytes);
        let str_at = self.string_offset as usize;
        file[str_at..str_at + self.string_bytes.len()].copy_from_slice(&self.string_bytes);
        let bind_at = self.bind_offset as usize;
        file[bind_at..bind_at + self.bind_bytes.len()].copy_from_slice(&self.bind_bytes);
        let rebase_at = self.rebase_offset as usize;
        file[rebase_at..rebase_at + self.rebase_bytes.len()].copy_from_slice(&self.rebase_bytes);
        let signature = adhoc_signature(&file, self.signature_offset as usize);
        let sig_at = self.signature_offset as usize;
        let copy = signature.len().min(file.len() - sig_at);
        file[sig_at..sig_at + copy].copy_from_slice(&signature[..copy]);
        std::fs::write(output, &file)
            .map_err(|e| LinkError::Native(format!("cannot write {}: {e}", output.display())))?;
        Ok(MachStats {
            image_size: file.len() as u64,
            sections: self.out.len(),
            imports: self.imports.len(),
            symbols: self.symbol_count as usize,
            code_signature_size: signature.len() as u64,
        })
    }
}

fn name_bytes(name: &str) -> [u8; 16] {
    let mut out = [0u8; 16];
    let bytes = name.as_bytes();
    let n = bytes.len().min(16);
    out[..n].copy_from_slice(&bytes[..n]);
    out
}

fn read_width(data: &[u8], at: usize, width: usize) -> Result<u64, LinkError> {
    let bytes = data
        .get(at..at + width)
        .ok_or_else(|| unsupported("relocation field out of range"))?;
    let mut value = 0u64;
    for (i, byte) in bytes.iter().enumerate() {
        value |= (*byte as u64) << (i * 8);
    }
    Ok(value)
}

fn write_width(data: &mut [u8], at: usize, width: usize, value: u64) -> Result<(), LinkError> {
    let bytes = data
        .get_mut(at..at + width)
        .ok_or_else(|| unsupported("relocation field out of range"))?;
    for (i, byte) in bytes.iter_mut().enumerate() {
        *byte = ((value >> (i * 8)) & 0xff) as u8;
    }
    Ok(())
}

fn read_insn(data: &[u8], at: usize) -> Result<u32, LinkError> {
    Ok(read_width(data, at, 4)? as u32)
}

fn write_insn(data: &mut [u8], at: usize, word: u32) -> Result<(), LinkError> {
    write_width(data, at, 4, word as u64)
}

fn adrp_word(place: u64, target: u64) -> u32 {
    let delta = ((target & !0xfff) as i64).wrapping_sub((place & !0xfff) as i64) >> 12;
    let imm = (delta as u32) & 0x1f_ffff;
    0x9000_0000 | ((imm & 3) << 29) | ((imm >> 2) << 5) | 16
}

fn ldr64_word(slot: u64) -> u32 {
    let imm = ((slot & 0xfff) >> 3) as u32;
    0xf940_0210 | (imm << 10)
}

fn push_uleb(out: &mut Vec<u8>, mut value: u64) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        out.push(byte);
        if value == 0 {
            break;
        }
    }
}

fn adhoc_signature(file: &[u8], signature_offset: usize) -> Vec<u8> {
    use sha2::{Digest, Sha256};
    let code_limit = signature_offset as u64;
    let slots = code_limit.div_ceil(4096) as usize;
    let ident = b"rnx";
    let header_size = 48usize;
    let ident_offset = header_size;
    let hash_offset = header_size + ident.len() + 1;
    let cd_size = hash_offset + slots * 32;
    let mut cd = vec![0u8; cd_size];
    cd[..4].copy_from_slice(&0xfade0c02u32.to_be_bytes());
    cd[4..8].copy_from_slice(&(cd_size as u32).to_be_bytes());
    cd[8..12].copy_from_slice(&0x20100u32.to_be_bytes());
    cd[12..16].copy_from_slice(&0x2u32.to_be_bytes());
    cd[16..20].copy_from_slice(&(hash_offset as u32).to_be_bytes());
    cd[20..24].copy_from_slice(&(ident_offset as u32).to_be_bytes());
    cd[24..28].copy_from_slice(&0u32.to_be_bytes());
    cd[28..32].copy_from_slice(&(slots as u32).to_be_bytes());
    cd[32..36].copy_from_slice(&(code_limit as u32).to_be_bytes());
    cd[36] = 32;
    cd[37] = 2;
    cd[38] = 0;
    cd[39] = 12;
    cd[40..44].copy_from_slice(&0u32.to_be_bytes());
    cd[44..48].copy_from_slice(&0u32.to_be_bytes());
    cd[ident_offset..ident_offset + ident.len()].copy_from_slice(ident);
    for slot in 0..slots {
        let start = slot * 4096;
        let end = (start + 4096).min(code_limit as usize);
        let digest = Sha256::digest(&file[start..end]);
        cd[hash_offset + slot * 32..hash_offset + slot * 32 + 32].copy_from_slice(&digest);
    }
    let blob_len = align_up(20 + cd_size as u64, 16) as usize;
    let mut blob = vec![0u8; blob_len];
    blob[..4].copy_from_slice(&0xfade0cc0u32.to_be_bytes());
    blob[4..8].copy_from_slice(&(blob_len as u32).to_be_bytes());
    blob[8..12].copy_from_slice(&1u32.to_be_bytes());
    blob[12..16].copy_from_slice(&0u32.to_be_bytes());
    blob[16..20].copy_from_slice(&20u32.to_be_bytes());
    blob[20..20 + cd.len()].copy_from_slice(&cd);
    blob
}

fn object_exports(object: &Object) -> Vec<String> {
    object
        .symbols
        .iter()
        .filter(|symbol| {
            let kind = symbol.n_type & N_TYPE;
            symbol.n_type & N_STAB == 0
                && symbol.n_type & N_EXT != 0
                && !symbol.name.is_empty()
                && match kind {
                    N_SECT => true,
                    N_UNDF => symbol.n_value > 0,
                    _ => false,
                }
        })
        .map(|symbol| symbol.name.clone())
        .collect()
}

fn object_undefined(object: &Object) -> Vec<String> {
    object
        .symbols
        .iter()
        .filter(|symbol| {
            symbol.n_type & N_STAB == 0
                && symbol.n_type & N_TYPE == N_UNDF
                && symbol.n_type & N_EXT != 0
                && symbol.n_value == 0
                && symbol.n_desc & N_WEAK_REF == 0
                && !symbol.name.is_empty()
        })
        .map(|symbol| symbol.name.clone())
        .collect()
}

struct ArchiveInput {
    members: Vec<Option<Object>>,
}

fn select_archive_members(
    objects: &mut Vec<Object>,
    archives: &mut [ArchiveInput],
) -> Result<(), LinkError> {
    let mut defined: BTreeSet<String> = BTreeSet::new();
    let mut needed: BTreeSet<String> = BTreeSet::new();
    for object in objects.iter() {
        defined.extend(object_exports(object));
        needed.extend(object_undefined(object));
    }
    needed.retain(|name| !defined.contains(name));
    loop {
        let mut added = false;
        for archive in archives.iter_mut() {
            for member in archive.members.iter_mut() {
                let Some(object) = member.as_ref() else {
                    continue;
                };
                let exports = object_exports(object);
                if !exports.iter().any(|name| needed.contains(name)) {
                    continue;
                }
                let Some(object) = member.take() else {
                    continue;
                };
                for name in &exports {
                    needed.remove(name);
                }
                defined.extend(exports);
                for name in object_undefined(&object) {
                    if !defined.contains(&name) {
                        needed.insert(name);
                    }
                }
                objects.push(object);
                added = true;
            }
        }
        if !added {
            break;
        }
    }
    Ok(())
}

pub fn link(inputs: &[MachInput], output: &Path, entry: &str) -> Result<MachStats, LinkError> {
    let mut objects = Vec::new();
    let mut archives: Vec<ArchiveInput> = Vec::new();
    let mut tbds = Vec::new();
    for input in inputs {
        match input {
            MachInput::Object(bytes) => objects.push(parse_object("<object>", bytes)?),
            MachInput::ObjectPath(path) => {
                let bytes = std::fs::read(path).map_err(|e| {
                    LinkError::Native(format!("cannot read {}: {e}", path.display()))
                })?;
                objects.push(parse_object(&path.display().to_string(), &bytes)?);
            }
            MachInput::Library(bytes) => {
                let members = archive_members(bytes, "<library>")?;
                let mut parsed = Vec::with_capacity(members.len());
                for (index, member) in members.into_iter().enumerate() {
                    if member.len() >= 4 && read_u32(&member, 0)? == MH_MAGIC_64 {
                        parsed.push(Some(parse_object(
                            &format!("<library>(member {index})"),
                            &member,
                        )?));
                    }
                }
                archives.push(ArchiveInput { members: parsed });
            }
            MachInput::LibraryPath(path) => {
                let bytes = std::fs::read(path).map_err(|e| {
                    LinkError::Native(format!("cannot read {}: {e}", path.display()))
                })?;
                let label = path.display().to_string();
                let members = archive_members(&bytes, &label)?;
                let mut parsed = Vec::with_capacity(members.len());
                for (index, member) in members.into_iter().enumerate() {
                    if member.len() >= 4 && read_u32(&member, 0)? == MH_MAGIC_64 {
                        parsed.push(Some(parse_object(
                            &format!("{label}(member {index})"),
                            &member,
                        )?));
                    }
                }
                archives.push(ArchiveInput { members: parsed });
            }
            MachInput::Tbd(text, label) => tbds.push(parse_tbd(label, text)?),
        }
    }
    select_archive_members(&mut objects, &mut archives)?;
    Linker::new(objects, tbds, entry)?.run(output)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tbd_reads_inline_and_block_symbols() {
        let text = r#"--- !tapi-tbd
tbd-version: 4
targets: [ arm64-macos ]
install-name: '/usr/lib/libSystem.B.dylib'
exports:
  - targets: [ arm64-macos ]
    symbols: [ _exit, _write,
               _close ]
    objc-classes: [ NotASymbol ]
  - targets: [ arm64-macos ]
    symbols:
      - _malloc
      - _free
reexported-symbols: [ _dummy ]
...
"#;
        let tbd = parse_tbd("test.tbd", text).unwrap();
        assert_eq!(tbd.install_name, "/usr/lib/libSystem.B.dylib");
        for name in ["_exit", "_write", "_close", "_malloc", "_free", "_dummy"] {
            assert!(tbd.symbols.contains(name), "missing {name}");
        }
        assert!(!tbd.symbols.contains("NotASymbol"));
    }

    #[test]
    fn tbd_without_install_name_is_rejected() {
        let text = "symbols: [ _exit ]\n";
        assert!(parse_tbd("test.tbd", text).is_err());
    }
}
