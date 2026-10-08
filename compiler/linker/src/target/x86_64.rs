use super::{ApplyCtx, Plan, SymClass, Target};
use crate::LinkError;

pub const EM_X86_64: u16 = 62;

pub const R_NONE: u32 = 0;
pub const R_64: u32 = 1;
pub const R_PC32: u32 = 2;
pub const R_PLT32: u32 = 4;
pub const R_GOTPCREL: u32 = 9;
pub const R_32: u32 = 10;
pub const R_32S: u32 = 11;
pub const R_TLSGD: u32 = 19;
pub const R_TLSLD: u32 = 20;
pub const R_DTPOFF32: u32 = 21;
pub const R_GOTTPOFF: u32 = 22;
pub const R_TPOFF32: u32 = 23;
pub const R_PC64: u32 = 24;
pub const R_GOTPCRELX: u32 = 41;
pub const R_REX_GOTPCRELX: u32 = 42;

pub fn reloc_name(kind: u32) -> &'static str {
    match kind {
        R_NONE => "R_X86_64_NONE",
        R_64 => "R_X86_64_64",
        R_PC32 => "R_X86_64_PC32",
        R_PLT32 => "R_X86_64_PLT32",
        R_GOTPCREL => "R_X86_64_GOTPCREL",
        R_32 => "R_X86_64_32",
        R_32S => "R_X86_64_32S",
        R_TLSGD => "R_X86_64_TLSGD",
        R_TLSLD => "R_X86_64_TLSLD",
        R_DTPOFF32 => "R_X86_64_DTPOFF32",
        R_GOTTPOFF => "R_X86_64_GOTTPOFF",
        R_TPOFF32 => "R_X86_64_TPOFF32",
        R_PC64 => "R_X86_64_PC64",
        R_GOTPCRELX => "R_X86_64_GOTPCRELX",
        R_REX_GOTPCRELX => "R_X86_64_REX_GOTPCRELX",
        _ => "R_X86_64_UNKNOWN",
    }
}

pub struct X86_64 {
    pub static_link: bool,
}

impl X86_64 {
    fn unsupported(&self, kind: u32, sym: SymClass) -> LinkError {
        LinkError::Native(format!(
            "unsupported relocation {} ({kind}) for {sym:?}: object uses a code pattern the native linker cannot emit yet",
            reloc_name(kind)
        ))
    }

    fn plan_static(&self, kind: u32, sym: SymClass) -> Result<Plan, LinkError> {
        match sym {
            SymClass::WeakUndef => match kind {
                R_64 | R_PC32 | R_PLT32 | R_32 | R_32S | R_PC64 | R_TPOFF32 | R_DTPOFF32 => Ok(Plan::Direct),
                R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX => Ok(Plan::Got),
                R_GOTTPOFF => Ok(Plan::TpoffRelax),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::Ifunc => match kind {
                R_PC32 | R_PLT32 | R_PC64 | R_64 | R_32 | R_32S => Ok(Plan::IFuncPlt),
                R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX => Ok(Plan::IFuncGot),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::Tls => match kind {
                R_TLSGD => Ok(Plan::TlsGd),
                R_TLSLD => Ok(Plan::TlsLd),
                R_DTPOFF32 => Ok(Plan::Direct),
                R_GOTTPOFF => Ok(Plan::TpoffRelax),
                R_TPOFF32 => Ok(Plan::Direct),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            _ => match kind {
                R_NONE => Ok(Plan::Skip),
                R_64 | R_PC32 | R_PLT32 | R_32 | R_32S | R_PC64 => Ok(Plan::Direct),
                R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX => Ok(Plan::Got),
                _ => Err(self.unsupported(kind, sym)),
            },
        }
    }

    fn plan_dynamic(&self, kind: u32, sym: SymClass) -> Result<Plan, LinkError> {
        match sym {
            SymClass::WeakUndef => match kind {
                R_64 | R_PC32 | R_PLT32 | R_32 | R_32S | R_PC64 => Ok(Plan::Direct),
                R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX => Ok(Plan::Got),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::Ifunc => match kind {
                R_PC32 | R_PLT32 | R_64 => Ok(Plan::IFuncPlt),
                R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX => Ok(Plan::IFuncGot),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::Local => match kind {
                R_NONE => Ok(Plan::Skip),
                R_64 | R_PC32 | R_PLT32 | R_32 | R_32S | R_PC64 => Ok(Plan::Direct),
                R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX => Ok(Plan::Got),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::DynamicFunc => match kind {
                R_NONE => Ok(Plan::Skip),
                R_PLT32 | R_PC32 | R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX => Ok(Plan::Plt),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::DynamicData => match kind {
                R_NONE => Ok(Plan::Skip),
                R_GOTPCREL | R_GOTPCRELX | R_REX_GOTPCRELX | R_PC32 | R_64 => Ok(Plan::Got),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::Tls => match kind {
                R_TLSGD => Ok(Plan::TlsGd),
                R_TLSLD => Ok(Plan::TlsLd),
                R_DTPOFF32 => Ok(Plan::Direct),
                R_GOTTPOFF => Ok(Plan::TpoffRelax),
                R_TPOFF32 => Ok(Plan::Direct),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
        }
    }
}

fn write32(bytes: &mut [u8], v: i64) -> Result<(), LinkError> {
    if bytes.len() < 4 {
        return Err(LinkError::Native("relocation field out of range".to_string()));
    }
    if v != (v as i32) as i64 {
        return Err(LinkError::Native(format!("relocation overflow: value {v:#x} does not fit 32 bits")));
    }
    bytes[..4].copy_from_slice(&(v as i32).to_le_bytes());
    Ok(())
}

fn write64(bytes: &mut [u8], v: u64) -> Result<(), LinkError> {
    if bytes.len() < 8 {
        return Err(LinkError::Native("relocation field out of range".to_string()));
    }
    bytes[..8].copy_from_slice(&v.to_le_bytes());
    Ok(())
}

impl Target for X86_64 {
    fn machine(&self) -> u16 {
        EM_X86_64
    }

    fn page_size(&self) -> u64 {
        0x1000
    }

    fn is_static(&self) -> bool {
        self.static_link
    }

    fn global_offset_label(&self) -> &'static str {
        "_GLOBAL_OFFSET_TABLE_"
    }

    fn plan(&self, kind: u32, sym: SymClass) -> Result<Plan, LinkError> {
        if self.static_link { self.plan_static(kind, sym) } else { self.plan_dynamic(kind, sym) }
    }

    fn apply(&self, kind: u32, plan: Plan, ctx: &ApplyCtx, bytes: &mut [u8], field: usize) -> Result<(), LinkError> {
        let s = ctx.sym_addr as i64;
        let p = ctx.place_addr as i64;
        let a = ctx.addend;
        macro_rules! fb {
            ($n:expr) => {
                field_bytes(bytes, field, $n)?
            };
        }
        match (kind, plan) {
            (_, Plan::Skip) => Ok(()),
            (R_64, Plan::IFuncPlt) | (R_32, Plan::IFuncPlt) | (R_32S, Plan::IFuncPlt) => {
                write_addr(fb!(8), (ctx.slot_addr as i64).wrapping_add(a), kind)
            }
            (R_PC32, Plan::IFuncPlt) | (R_PLT32, Plan::IFuncPlt) | (R_PC64, Plan::IFuncPlt) => {
                if kind == R_PC64 {
                    write64(fb!(8), (ctx.slot_addr as i64).wrapping_sub(p).wrapping_add(a) as u64)
                } else {
                    write32(fb!(4), (ctx.slot_addr as i64).wrapping_sub(p).wrapping_add(a))
                }
            }
            (R_64, _) => write64(fb!(8), s.wrapping_add(a) as u64),
            (R_PC64, _) => write64(fb!(8), s.wrapping_add(a).wrapping_sub(p) as u64),
            (R_PC32, Plan::Plt) | (R_PLT32, Plan::Plt) => {
                write32(fb!(4), (ctx.slot_addr as i64).wrapping_sub(p).wrapping_add(a))
            }
            (R_PC32, _) | (R_PLT32, _) => write32(fb!(4), s.wrapping_sub(p).wrapping_add(a)),
            (R_32, _) => {
                let v = s.wrapping_add(a);
                if v < 0 || v > 0xffff_ffff {
                    return Err(LinkError::Native(format!("R_X86_64_32 overflow: {v:#x} exceeds 32 bits")));
                }
                write32(fb!(4), v)
            }
            (R_32S, _) => write32(fb!(4), s.wrapping_add(a)),
            (R_GOTPCREL, _) | (R_GOTPCRELX, _) | (R_REX_GOTPCRELX, _) => {
                write32(fb!(4), (ctx.slot_addr as i64).wrapping_add(a).wrapping_sub(p))
            }
            (R_DTPOFF32, _) => write32(fb!(4), ctx.tpoff.wrapping_add(a)),
            (R_TPOFF32, _) => write32(fb!(4), ctx.tpoff.wrapping_add(a)),
            (R_TLSGD, Plan::TlsGd) => {
                let base = field.checked_sub(4).ok_or_else(|| LinkError::Native("TLSGD underflow".to_string()))?;
                apply_tls_gd(bytes.get_mut(base..base + 16).ok_or_else(|| LinkError::Native("TLSGD sequence out of range".to_string()))?, ctx.tpoff as i32)
            }
            (R_TLSLD, Plan::TlsLd) => {
                let base = field.checked_sub(3).ok_or_else(|| LinkError::Native("TLSLD underflow".to_string()))?;
                apply_tls_ld(bytes.get_mut(base..base + 12).ok_or_else(|| LinkError::Native("TLSLD sequence out of range".to_string()))?)
            }
            (R_GOTTPOFF, Plan::TpoffRelax) => {
                let base = field.checked_sub(3).ok_or_else(|| LinkError::Native("GOTTPOFF underflow".to_string()))?;
                apply_gottpoff(bytes.get_mut(base..base + 7).ok_or_else(|| LinkError::Native("GOTTPOFF sequence out of range".to_string()))?, ctx.tpoff as i32)
            }
            _ => Err(self.unsupported(kind, SymClass::Local)),
        }
    }

    fn iplt_entry(&self, entry_addr: u64, slot_addr: u64) -> [u8; 16] {
        let mut out = [0x90u8; 16];
        out[0] = 0xff;
        out[1] = 0x25;
        out[2..6].copy_from_slice(&((slot_addr as i64).wrapping_sub(entry_addr as i64 + 6) as i32).to_le_bytes());
        out
    }

    fn rel_jmpslot(&self) -> u32 {
        7
    }

    fn rel_globdat(&self) -> u32 {
        6
    }

    fn rel_relative(&self) -> u32 {
        8
    }

    fn rel_irelative(&self) -> u32 {
        37
    }

    fn rel_copy(&self) -> u32 {
        5
    }

    fn plt_entry(&self, entry_addr: u64, slot_addr: u64, index: u32, plt0_addr: u64) -> [u8; 16] {
        let mut out = [0u8; 16];
        out[0] = 0xff;
        out[1] = 0x25;
        out[2..6].copy_from_slice(&((slot_addr as i64).wrapping_sub(entry_addr as i64 + 6) as i32).to_le_bytes());
        out[6] = 0x68;
        out[7..11].copy_from_slice(&index.to_le_bytes());
        out[11] = 0xe9;
        out[12..16].copy_from_slice(&((plt0_addr as i64).wrapping_sub(entry_addr as i64 + 16) as i32).to_le_bytes());
        out
    }
}

fn field_bytes<'b>(bytes: &'b mut [u8], field: usize, n: usize) -> Result<&'b mut [u8], LinkError> {
    bytes.get_mut(field..field + n).ok_or_else(|| LinkError::Native("relocation field out of range".to_string()))
}fn write_addr(bytes: &mut [u8], v: i64, kind: u32) -> Result<(), LinkError> {
    if kind == R_64 {
        write64(bytes, v as u64)
    } else {
        write32(bytes, v)
    }
}

fn apply_tls_gd(bytes: &mut [u8], tpoff: i32) -> Result<(), LinkError> {
    if bytes.len() < 16 {
        return Err(LinkError::Native("TLSGD sequence out of range".to_string()));
    }
    if bytes[0..4] != [0x66, 0x48, 0x8d, 0x3d] || bytes[8..12] != [0x66, 0x66, 0x48, 0xe8] {
        return Err(LinkError::Native(
            "unsupported TLSGD instruction form: only data16 lea plus data16 data16 call relax to local-exec".to_string(),
        ));
    }
    bytes[0..9].copy_from_slice(&[0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00]);
    bytes[9] = 0x48;
    bytes[10] = 0x8d;
    bytes[11] = 0x80;
    bytes[12..16].copy_from_slice(&tpoff.to_le_bytes());
    Ok(())
}

fn apply_tls_ld(bytes: &mut [u8]) -> Result<(), LinkError> {
    if bytes.len() < 12 {
        return Err(LinkError::Native("TLSLD sequence out of range".to_string()));
    }
    if bytes[0] != 0x48 || bytes[1] != 0x8d || bytes[2] != 0x3d || bytes[7] != 0xe8 {
        return Err(LinkError::Native(
            "unsupported TLSLD instruction form: only lea plus call relax to local-exec".to_string(),
        ));
    }
    bytes[0..9].copy_from_slice(&[0x64, 0x48, 0x8b, 0x04, 0x25, 0x00, 0x00, 0x00, 0x00]);
    bytes[9..12].copy_from_slice(&[0x0f, 0x1f, 0x00]);
    Ok(())
}

fn apply_gottpoff(bytes: &mut [u8], tpoff: i32) -> Result<(), LinkError> {    if bytes.len() < 7 {
        return Err(LinkError::Native("GOTTPOFF sequence out of range".to_string()));
    }
    let rex = bytes[0];
    if rex != 0x48 && rex != 0x4c || bytes[1] != 0x8b {
        return Err(LinkError::Native(
            "unsupported GOTTPOFF instruction form: only REX.W mov r64 from GOT relax to local-exec".to_string(),
        ));
    }
    let modrm = bytes[2];
    if modrm & 0xc7 != 0x05 {
        return Err(LinkError::Native("unsupported GOTTPOFF addressing form".to_string()));
    }
    let dst = (modrm >> 3) & 7;
    bytes[1] = 0xc7;
    bytes[2] = 0xc0 | dst;
    bytes[3..7].copy_from_slice(&tpoff.to_le_bytes());
    Ok(())
}

pub fn gotpcrelx_form(bytes: &[u8], field: usize) -> u8 {
    if field < 2 || field + 4 > bytes.len() {
        return 0;
    }
    if bytes[field - 2] == 0x8b && bytes[field - 1] & 0xc7 == 0x05 {
        return 1;
    }
    if bytes[field - 2] == 0xff && (bytes[field - 1] == 0x15 || bytes[field - 1] == 0x25) {
        return if bytes[field - 1] == 0x15 { 2 } else { 3 };
    }
    0
}

pub fn apply_relaxed(bytes: &mut [u8], field: usize, sym_addr: u64, place_addr: u64, addend: i64) -> Result<(), LinkError> {
    let form = gotpcrelx_form(bytes, field);
    if form == 0 {
        return Err(LinkError::Native("unsupported GOTPCRELX instruction form for relaxation".to_string()));
    }
    if form == 1 {
        bytes[field - 2] = 0x8d;
        let rel = (sym_addr as i64).wrapping_add(addend).wrapping_sub(place_addr as i64);
        return write32(field_bytes(bytes, field, 4)?, rel);
    }
    let next_ip = (place_addr as i64).wrapping_add(3);
    let rel = (sym_addr as i64).wrapping_add(addend).wrapping_add(4).wrapping_sub(next_ip);
    if rel != (rel as i32) as i64 {
        return Err(LinkError::Native(format!("relocation overflow: value {rel:#x} does not fit 32 bits")));
    }
    bytes[field - 2] = if form == 2 { 0xe8 } else { 0xe9 };
    let slot = field_bytes(bytes, field - 1, 4)?;
    slot.copy_from_slice(&(rel as i32).to_le_bytes());
    field_bytes(bytes, field + 3, 1)?[0] = 0x90;
    Ok(())
}
