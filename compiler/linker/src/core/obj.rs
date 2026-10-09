use crate::LinkError;

pub const SHN_UNDEF: u32 = 0;
pub const SHN_ABS: u32 = 0xfff1;
pub const SHN_COMMON: u32 = 0xfff2;
pub const SHN_XINDEX: u32 = 0xffff;

pub const SHT_NULL: u32 = 0;
pub const SHT_PROGBITS: u32 = 1;
pub const SHT_SYMTAB: u32 = 2;
pub const SHT_STRTAB: u32 = 3;
pub const SHT_RELA: u32 = 4;
pub const SHT_NOBITS: u32 = 8;
pub const SHT_GROUP: u32 = 17;
pub const SHT_GNU_VERSYM: u32 = 0x6fffffff;

pub const SHF_ALLOC: u64 = 0x2;
pub const SHF_EXECINSTR: u64 = 0x4;
pub const SHF_WRITE: u64 = 0x1;
pub const SHF_TLS: u64 = 0x400;
pub const SHF_COMPRESSED: u64 = 0x800;

pub const STB_LOCAL: u8 = 0;
pub const STB_GLOBAL: u8 = 1;
pub const STB_WEAK: u8 = 2;

pub const STT_NOTYPE: u8 = 0;
pub const STT_OBJECT: u8 = 1;
pub const STT_FUNC: u8 = 2;
pub const STT_SECTION: u8 = 3;
pub const STT_FILE: u8 = 4;
pub const STT_TLS: u8 = 6;
pub const STT_GNU_IFUNC: u8 = 10;

pub const STV_HIDDEN: u8 = 2;

pub const GRP_COMDAT: u32 = 1;

#[derive(Clone, Debug)]
pub struct Section {
    pub name: u32,
    pub kind: u32,
    pub flags: u64,
    pub link: u32,
    pub info: u32,
    pub align: u64,
    pub off: u64,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct Symbol {
    pub name: u32,
    pub bind: u8,
    pub kind: u8,
    pub vis: u8,
    pub shndx: u32,
    pub value: u64,
    pub size: u64,
}

#[derive(Clone, Debug)]
pub struct Rela {
    pub section: u32,
    pub offset: u64,
    pub kind: u32,
    pub sym: u32,
    pub addend: i64,
}

#[derive(Clone, Debug)]
pub struct Group {
    pub signature: u32,
    pub members: Vec<u32>,
}

pub struct Object<'a> {
    pub data: &'a [u8],
    pub machine: u16,
    pub sections: Vec<Section>,
    pub symbols: Vec<Symbol>,
    pub relas: Vec<Rela>,
    pub groups: Vec<Group>,
    pub symtab_bases: Vec<(usize, usize)>,
    shstrtab: (u64, u64),
    symstr: Option<(u64, u64)>,
}

fn u16_at(d: &[u8], o: usize) -> Option<u16> {
    d.get(o..o + 2).and_then(|b| b.try_into().ok()).map(u16::from_le_bytes)
}

fn u32_at(d: &[u8], o: usize) -> Option<u32> {
    d.get(o..o + 4).and_then(|b| b.try_into().ok()).map(u32::from_le_bytes)
}

fn u64_at(d: &[u8], o: usize) -> Option<u64> {
    d.get(o..o + 8).and_then(|b| b.try_into().ok()).map(u64::from_le_bytes)
}

fn cstr(d: &[u8], base: u64, off: u32) -> Result<&str, LinkError> {
    let start = base
        .checked_add(off as u64)
        .and_then(|v| usize::try_from(v).ok())
        .ok_or_else(|| LinkError::Native("bad string offset".to_string()))?;
    if start > d.len() {
        return Err(LinkError::Native("string offset out of range".to_string()));
    }
    let end = d[start..].iter().position(|&b| b == 0).map(|p| start + p).unwrap_or(d.len());
    std::str::from_utf8(&d[start..end]).map_err(|_| LinkError::Native("non-utf8 name in object".to_string()))
}

impl<'a> Object<'a> {
    pub fn parse(data: &'a [u8], label: &str) -> Result<Object<'a>, LinkError> {
        let err = |m: &str| LinkError::Native(format!("{label}: {m}"));
        if data.len() < 64 || &data[0..4] != b"\x7fELF" {
            return Err(err("not an ELF file"));
        }
        if data[4] != 2 || data[5] != 1 {
            return Err(err("not a 64-bit little-endian ELF file"));
        }
        if u16_at(data, 16).ok_or_else(|| err("truncated ELF header"))? != 1 {
            return Err(err("not a relocatable object"));
        }
        let machine = u16_at(data, 18).ok_or_else(|| err("truncated ELF header"))?;
        let shoff = u64_at(data, 40).ok_or_else(|| err("truncated ELF header"))? as usize;
        let shentsize = u16_at(data, 58).ok_or_else(|| err("truncated ELF header"))? as usize;
        let mut shnum = u16_at(data, 60).ok_or_else(|| err("truncated ELF header"))? as usize;
        let mut shstrndx = u16_at(data, 62).ok_or_else(|| err("truncated ELF header"))? as usize;
        if shoff == 0 || shentsize < 64 {
            return Err(err("object has no section headers"));
        }
        if shnum == 0 {
            shnum = u64_at(data, shoff + 32).ok_or_else(|| err("truncated section headers"))? as usize;
            if shnum == 0 {
                return Err(err("object has no sections"));
            }
        }
        if shnum > 100_000 {
            return Err(err("implausible section count"));
        }
        let mut sections = Vec::with_capacity(shnum.min(8192));
        for i in 0..shnum {
            let o = shoff.checked_add(i * shentsize).ok_or_else(|| err("section headers out of range"))?;
            if o + 64 > data.len() {
                return Err(err("section headers out of range"));
            }
            sections.push(Section {
                name: u32_at(data, o).ok_or_else(|| err("truncated section header"))?,
                kind: u32_at(data, o + 4).ok_or_else(|| err("truncated section header"))?,
                flags: u64_at(data, o + 8).ok_or_else(|| err("truncated section header"))?,
                link: u32_at(data, o + 40).ok_or_else(|| err("truncated section header"))?,
                info: u32_at(data, o + 44).ok_or_else(|| err("truncated section header"))?,
                align: u64_at(data, o + 48).ok_or_else(|| err("truncated section header"))?,
                off: u64_at(data, o + 24).ok_or_else(|| err("truncated section header"))?,
                size: u64_at(data, o + 32).ok_or_else(|| err("truncated section header"))?,
            });
        }
        if shstrndx == SHN_XINDEX as usize {
            shstrndx = sections.first().map(|s| s.info as usize).unwrap_or(usize::MAX);
        }
        let shstrtab_sec = sections.get(shstrndx).ok_or_else(|| err("bad section name table index"))?;
        let shstrtab = (shstrtab_sec.off, shstrtab_sec.size);
        let mut symbols = Vec::new();
        let mut relas = Vec::new();
        let mut groups = Vec::new();
        let mut symtab_bases: Vec<(usize, usize)> = Vec::new();
        let mut symstr: Option<(u64, u64)> = None;
        for (i, s) in sections.iter().enumerate() {
            if s.kind != SHT_SYMTAB {
                continue;
            }
            if s.size % 24 != 0 {
                return Err(err("bad symbol table size"));
            }
            let base = s.off as usize;
            let n = s.size as usize / 24;
            if base.checked_add(s.size as usize).is_none_or(|e| e > data.len()) {
                return Err(err("symbol table out of range"));
            }
            symtab_bases.push((i, symbols.len()));
            let strsec = sections.get(s.link as usize).ok_or_else(|| err("bad symtab string link"))?;
            if symstr.is_none() {
                symstr = Some((strsec.off, strsec.size));
            }
            for k in 0..n {
                let o = base + k * 24;
                let shndx = u16_at(data, o + 6).ok_or_else(|| err("truncated symbol"))? as u32;
                if shndx == SHN_XINDEX {
                    return Err(err("extended section indices unsupported"));
                }
                symbols.push(Symbol {
                    name: u32_at(data, o).ok_or_else(|| err("truncated symbol"))?,
                    bind: data[o + 4] >> 4,
                    kind: data[o + 4] & 15,
                    vis: data[o + 5] & 3,
                    shndx,
                    value: u64_at(data, o + 8).ok_or_else(|| err("truncated symbol"))?,
                    size: u64_at(data, o + 16).ok_or_else(|| err("truncated symbol"))?,
                });
            }
        }
        for s in sections.iter() {
            match s.kind {
                SHT_RELA => {
                    if s.size % 24 != 0 {
                        return Err(err("bad relocation section size"));
                    }
                    let base = s.off as usize;
                    let n = s.size as usize / 24;
                    if base.checked_add(s.size as usize).is_none_or(|e| e > data.len()) {
                        return Err(err("relocation section out of range"));
                    }
                    let symbase = match symtab_bases.iter().find(|&&(sec, _)| sec == s.link as usize).map(|&(_, b)| b) {
                        Some(b) => b,
                        None if s.size == 0 => continue,
                        None => return Err(err("relocation section with bad symtab link")),
                    };
                    for k in 0..n {
                        let o = base + k * 24;
                        let r_offset = u64_at(data, o).ok_or_else(|| err("truncated relocation"))?;
                        let r_info = u64_at(data, o + 8).ok_or_else(|| err("truncated relocation"))?;
                        let r_addend = u64_at(data, o + 16).ok_or_else(|| err("truncated relocation"))? as i64;
                        let sym = (r_info >> 32) as u32 as usize;
                        relas.push(Rela {
                            section: s.info,
                            offset: r_offset,
                            kind: (r_info & 0xffff_ffff) as u32,
                            sym: symbase.checked_add(sym).ok_or_else(|| err("bad relocation symbol"))? as u32,
                            addend: r_addend,
                        });
                    }
                }
                SHT_GROUP => {
                    if s.size < 4 || s.size % 4 != 0 {
                        return Err(err("bad group section size"));
                    }
                    let base = s.off as usize;
                    if base.checked_add(s.size as usize).is_none_or(|e| e > data.len()) {
                        return Err(err("group section out of range"));
                    }
                    if u32_at(data, base).ok_or_else(|| err("truncated group"))? != GRP_COMDAT {
                        return Err(err("unsupported non-COMDAT section group"));
                    }
                    let mut members = Vec::new();
                    for k in 1..s.size as usize / 4 {
                        members.push(u32_at(data, base + k * 4).ok_or_else(|| err("truncated group"))?);
                    }
                    groups.push(Group { signature: s.info, members });
                }
                _ => {}
            }
        }
        for s in sections.iter() {
            if s.kind != SHT_NOBITS && s.size > 0 {
                let end = s.off.checked_add(s.size).ok_or_else(|| err("bad section range"))? as usize;
                if end > data.len() {
                    return Err(err("section data out of range"));
                }
            }
        }
        Ok(Object { data, machine, sections, symbols, relas, groups, symtab_bases, shstrtab, symstr })
    }

    pub fn section_name(&self, idx: usize) -> Result<&str, LinkError> {
        let s = self.sections.get(idx).ok_or_else(|| LinkError::Native("bad section index".to_string()))?;
        cstr(self.data, self.shstrtab.0, s.name)
    }

    pub fn symbol_name(&self, idx: usize) -> Result<&str, LinkError> {
        let sym = self.symbols.get(idx).ok_or_else(|| LinkError::Native("bad symbol index".to_string()))?;
        let symstr = self.symstr.ok_or_else(|| LinkError::Native("object has no string table".to_string()))?;
        cstr(self.data, symstr.0, sym.name)
    }

    pub fn section_bytes(&self, idx: usize) -> Result<&'a [u8], LinkError> {
        let s = self.sections.get(idx).ok_or_else(|| LinkError::Native("bad section index".to_string()))?;
        if s.kind == SHT_NOBITS || s.size == 0 {
            return Ok(&[]);
        }
        let (o, n) = (s.off as usize, s.size as usize);
        self.data.get(o..o + n).ok_or_else(|| LinkError::Native("section data out of range".to_string()))
    }
}
