use super::{ApplyCtx, Plan, SymClass, Target};
use crate::LinkError;

pub const EM_AARCH64: u16 = 183;

pub const R_NONE: u32 = 0;
pub const R_ABS64: u32 = 257;
pub const R_ABS32: u32 = 258;
pub const R_ABS16: u32 = 259;
pub const R_PREL64: u32 = 260;
pub const R_PREL32: u32 = 261;
pub const R_PREL16: u32 = 262;
pub const R_ADR_PREL_LO21: u32 = 274;
pub const R_ADR_PREL_PG_HI21: u32 = 275;
pub const R_ADR_PREL_PG_HI21_NC: u32 = 276;
pub const R_ADD_ABS_LO12_NC: u32 = 277;
pub const R_LDST8_ABS_LO12_NC: u32 = 278;
pub const R_TSTBR14: u32 = 279;
pub const R_CONDBR19: u32 = 280;
pub const R_JUMP26: u32 = 282;
pub const R_CALL26: u32 = 283;
pub const R_LDST16_ABS_LO12_NC: u32 = 284;
pub const R_LDST32_ABS_LO12_NC: u32 = 285;
pub const R_LDST64_ABS_LO12_NC: u32 = 286;
pub const R_ADR_GOT_PAGE: u32 = 311;
pub const R_LD64_GOT_LO12_NC: u32 = 312;
pub const R_TLSGD_ADR_PAGE21: u32 = 513;
pub const R_TLSGD_ADD_LO12_NC: u32 = 514;
pub const R_TLSIE_ADR_GOT_PAGE: u32 = 541;
pub const R_TLSIE_LD64_GOT_LO12_NC: u32 = 542;
pub const R_TLSLE_ADD_TPREL_HI12: u32 = 549;
pub const R_TLSLE_ADD_TPREL_LO12: u32 = 550;
pub const R_TLSLE_ADD_TPREL_LO12_NC: u32 = 551;
pub const R_TLSLE_LDST8_TPREL_LO12: u32 = 552;
pub const R_TLSLE_LDST8_TPREL_LO12_NC: u32 = 553;
pub const R_TLSLE_LDST16_TPREL_LO12: u32 = 554;
pub const R_TLSLE_LDST16_TPREL_LO12_NC: u32 = 555;
pub const R_TLSLE_LDST32_TPREL_LO12: u32 = 556;
pub const R_TLSLE_LDST32_TPREL_LO12_NC: u32 = 557;
pub const R_TLSLE_LDST64_TPREL_LO12: u32 = 558;
pub const R_TLSLE_LDST64_TPREL_LO12_NC: u32 = 559;

pub const R_COPY: u32 = 1024;
pub const R_GLOB_DAT: u32 = 1025;
pub const R_JUMP_SLOT: u32 = 1026;
pub const R_RELATIVE: u32 = 1027;
pub const R_IRELATIVE: u32 = 1032;

pub fn reloc_name(kind: u32) -> &'static str {
    match kind {
        R_NONE => "R_AARCH64_NONE",
        R_ABS64 => "R_AARCH64_ABS64",
        R_ABS32 => "R_AARCH64_ABS32",
        R_ABS16 => "R_AARCH64_ABS16",
        R_PREL64 => "R_AARCH64_PREL64",
        R_PREL32 => "R_AARCH64_PREL32",
        R_PREL16 => "R_AARCH64_PREL16",
        R_ADR_PREL_LO21 => "R_AARCH64_ADR_PREL_LO21",
        R_ADR_PREL_PG_HI21 => "R_AARCH64_ADR_PREL_PG_HI21",
        R_ADR_PREL_PG_HI21_NC => "R_AARCH64_ADR_PREL_PG_HI21_NC",
        R_ADD_ABS_LO12_NC => "R_AARCH64_ADD_ABS_LO12_NC",
        R_LDST8_ABS_LO12_NC => "R_AARCH64_LDST8_ABS_LO12_NC",
        R_TSTBR14 => "R_AARCH64_TSTBR14",
        R_CONDBR19 => "R_AARCH64_CONDBR19",
        R_JUMP26 => "R_AARCH64_JUMP26",
        R_CALL26 => "R_AARCH64_CALL26",
        R_LDST16_ABS_LO12_NC => "R_AARCH64_LDST16_ABS_LO12_NC",
        R_LDST32_ABS_LO12_NC => "R_AARCH64_LDST32_ABS_LO12_NC",
        R_LDST64_ABS_LO12_NC => "R_AARCH64_LDST64_ABS_LO12_NC",
        R_ADR_GOT_PAGE => "R_AARCH64_ADR_GOT_PAGE",
        R_LD64_GOT_LO12_NC => "R_AARCH64_LD64_GOT_LO12_NC",
        R_TLSGD_ADR_PAGE21 => "R_AARCH64_TLSGD_ADR_PAGE21",
        R_TLSGD_ADD_LO12_NC => "R_AARCH64_TLSGD_ADD_LO12_NC",
        R_TLSIE_ADR_GOT_PAGE => "R_AARCH64_TLSIE_ADR_GOTTPREL_PAGE21",
        R_TLSIE_LD64_GOT_LO12_NC => "R_AARCH64_TLSIE_LD64_GOTTPREL_LO12_NC",
        R_TLSLE_ADD_TPREL_HI12 => "R_AARCH64_TLSLE_ADD_TPREL_HI12",
        R_TLSLE_ADD_TPREL_LO12 => "R_AARCH64_TLSLE_ADD_TPREL_LO12",
        R_TLSLE_ADD_TPREL_LO12_NC => "R_AARCH64_TLSLE_ADD_TPREL_LO12_NC",
        R_TLSLE_LDST8_TPREL_LO12 => "R_AARCH64_TLSLE_LDST8_TPREL_LO12",
        R_TLSLE_LDST8_TPREL_LO12_NC => "R_AARCH64_TLSLE_LDST8_TPREL_LO12_NC",
        R_TLSLE_LDST16_TPREL_LO12 => "R_AARCH64_TLSLE_LDST16_TPREL_LO12",
        R_TLSLE_LDST16_TPREL_LO12_NC => "R_AARCH64_TLSLE_LDST16_TPREL_LO12_NC",
        R_TLSLE_LDST32_TPREL_LO12 => "R_AARCH64_TLSLE_LDST32_TPREL_LO12",
        R_TLSLE_LDST32_TPREL_LO12_NC => "R_AARCH64_TLSLE_LDST32_TPREL_LO12_NC",
        R_TLSLE_LDST64_TPREL_LO12 => "R_AARCH64_TLSLE_LDST64_TPREL_LO12",
        R_TLSLE_LDST64_TPREL_LO12_NC => "R_AARCH64_TLSLE_LDST64_TPREL_LO12_NC",
        _ => "R_AARCH64_UNKNOWN",
    }
}

pub struct Aarch64 {
    pub static_link: bool,
}

impl Aarch64 {
    fn unsupported(&self, kind: u32, sym: SymClass) -> LinkError {
        LinkError::Native(format!(
            "unsupported relocation {} ({kind}) for {sym:?}: object uses a code pattern the native linker cannot emit yet",
            reloc_name(kind)
        ))
    }

    fn plan_direct(&self, kind: u32, sym: SymClass) -> Result<Plan, LinkError> {
        match sym {
            SymClass::WeakUndef | SymClass::Local => match kind {
                R_NONE => Ok(Plan::Skip),
                R_ABS64
                | R_ABS32
                | R_ABS16
                | R_PREL64
                | R_PREL32
                | R_PREL16
                | R_ADR_PREL_LO21
                | R_ADR_PREL_PG_HI21
                | R_ADR_PREL_PG_HI21_NC
                | R_ADD_ABS_LO12_NC
                | R_LDST8_ABS_LO12_NC
                | R_LDST16_ABS_LO12_NC
                | R_LDST32_ABS_LO12_NC
                | R_LDST64_ABS_LO12_NC
                | R_TSTBR14
                | R_CONDBR19
                | R_JUMP26
                | R_CALL26 => Ok(Plan::Direct),
                R_ADR_GOT_PAGE | R_LD64_GOT_LO12_NC => Ok(Plan::Got),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::Ifunc => match kind {
                R_JUMP26 | R_CALL26 | R_ABS64 => Ok(Plan::IFuncPlt),
                R_ADR_GOT_PAGE | R_LD64_GOT_LO12_NC => Ok(Plan::IFuncGot),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::Tls => match kind {
                R_ABS64
                | R_ABS32
                | R_ABS16
                | R_PREL64
                | R_PREL32
                | R_PREL16
                | R_ADR_PREL_LO21
                | R_ADR_PREL_PG_HI21
                | R_ADR_PREL_PG_HI21_NC
                | R_ADD_ABS_LO12_NC
                | R_LDST8_ABS_LO12_NC
                | R_LDST16_ABS_LO12_NC
                | R_LDST32_ABS_LO12_NC
                | R_LDST64_ABS_LO12_NC => Ok(Plan::Direct),
                R_TLSIE_ADR_GOT_PAGE | R_TLSIE_LD64_GOT_LO12_NC => Ok(Plan::Got),
                R_TLSLE_ADD_TPREL_HI12
                | R_TLSLE_ADD_TPREL_LO12
                | R_TLSLE_ADD_TPREL_LO12_NC
                | R_TLSLE_LDST8_TPREL_LO12
                | R_TLSLE_LDST8_TPREL_LO12_NC
                | R_TLSLE_LDST16_TPREL_LO12
                | R_TLSLE_LDST16_TPREL_LO12_NC
                | R_TLSLE_LDST32_TPREL_LO12
                | R_TLSLE_LDST32_TPREL_LO12_NC
                | R_TLSLE_LDST64_TPREL_LO12
                | R_TLSLE_LDST64_TPREL_LO12_NC => Ok(Plan::Direct),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::DynamicFunc => match kind {
                R_JUMP26 | R_CALL26 => Ok(Plan::Plt),
                R_ABS64 => Ok(Plan::DynAbs),
                R_ADR_GOT_PAGE | R_LD64_GOT_LO12_NC => Ok(Plan::Got),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
            SymClass::DynamicData => match kind {
                R_ADR_GOT_PAGE | R_LD64_GOT_LO12_NC => Ok(Plan::Got),
                R_ABS64 => Ok(Plan::DynAbs),
                R_NONE => Ok(Plan::Skip),
                _ => Err(self.unsupported(kind, sym)),
            },
        }
    }
}

fn field_bytes<'b>(bytes: &'b mut [u8], field: usize, n: usize) -> Result<&'b mut [u8], LinkError> {
    bytes
        .get_mut(field..field + n)
        .ok_or_else(|| LinkError::Native("relocation field out of range".to_string()))
}

fn write_le(bytes: &mut [u8], value: u64) -> Result<(), LinkError> {
    let n = bytes.len();
    if n == 0 || n > 8 {
        return Err(LinkError::Native(
            "relocation field out of range".to_string(),
        ));
    }
    for (i, slot) in bytes.iter_mut().enumerate() {
        *slot = ((value >> (8 * i)) & 0xff) as u8;
    }
    Ok(())
}

fn read_instr(bytes: &[u8], field: usize) -> Result<u32, LinkError> {
    let raw = bytes
        .get(field..field + 4)
        .ok_or_else(|| LinkError::Native("relocation field out of range".to_string()))?;
    Ok(u32::from_le_bytes([raw[0], raw[1], raw[2], raw[3]]))
}

fn write_instr(bytes: &mut [u8], field: usize, word: u32) -> Result<(), LinkError> {
    field_bytes(bytes, field, 4)?.copy_from_slice(&word.to_le_bytes());
    Ok(())
}

fn range_error(kind: u32, value: i64, bits: u32) -> LinkError {
    LinkError::Native(format!(
        "{} overflow: value {value:#x} does not fit a signed {bits}-bit pc-relative field",
        reloc_name(kind)
    ))
}

fn page_delta(target: u64, place: u64) -> i64 {
    ((target & !0xfff) as i64).wrapping_sub((place & !0xfff) as i64) >> 12
}

fn encode_adrp(word: u32, imm: i64, check: bool, kind: u32) -> Result<u32, LinkError> {
    if check && !(-(1 << 20)..(1 << 20)).contains(&imm) {
        return Err(range_error(kind, imm, 21));
    }
    let imm = imm as u32 & 0x1f_ffff;
    Ok((word & 0x9f00_001f) | ((imm & 3) << 29) | ((imm >> 2) << 5))
}

fn encode_adr(word: u32, imm: i64, kind: u32) -> Result<u32, LinkError> {
    if !(-(1 << 20)..(1 << 20)).contains(&imm) {
        return Err(range_error(kind, imm, 21));
    }
    let imm = imm as u32 & 0x1f_ffff;
    Ok((word & 0x9f00_001f) | ((imm & 3) << 29) | ((imm >> 2) << 5))
}

fn encode_add_imm(word: u32, imm12: u32, shift: bool) -> u32 {
    (word & 0xffc0_03ff) | ((imm12 & 0xfff) << 10) | (u32::from(shift) << 22)
}

fn encode_ldst_imm(word: u32, offset: i64, shift: u32, kind: u32) -> Result<u32, LinkError> {
    if offset < 0 {
        return Err(LinkError::Native(format!(
            "{}: negative load/store offset {offset:#x}",
            reloc_name(kind)
        )));
    }
    if offset as u64 & ((1u64 << shift) - 1) != 0 {
        return Err(LinkError::Native(format!(
            "{}: offset {offset:#x} is not aligned to {} bytes",
            reloc_name(kind),
            1u64 << shift
        )));
    }
    let scaled = (offset as u64) >> shift;
    if scaled > 0xfff {
        return Err(LinkError::Native(format!(
            "{} overflow: offset {offset:#x} exceeds a 12-bit scaled field",
            reloc_name(kind)
        )));
    }
    Ok((word & 0xffc0_03ff) | ((scaled as u32) << 10))
}

fn encode_branch(word: u32, delta: i64, kind: u32) -> Result<u32, LinkError> {
    if delta & 3 != 0 {
        return Err(LinkError::Native(format!(
            "{}: offset {delta:#x} is not 4-byte aligned",
            reloc_name(kind)
        )));
    }
    if !(-(1 << 27)..(1 << 27)).contains(&delta) {
        return Err(range_error(kind, delta, 28));
    }
    Ok((word & 0xfc00_0000) | (((delta >> 2) as u32) & 0x03ff_ffff))
}

fn encode_cond_branch(word: u32, delta: i64) -> Result<u32, LinkError> {
    if delta & 3 != 0 {
        return Err(LinkError::Native(format!(
            "R_AARCH64_CONDBR19: offset {delta:#x} is not 4-byte aligned"
        )));
    }
    if !(-(1 << 20)..(1 << 20)).contains(&delta) {
        return Err(range_error(R_CONDBR19, delta, 21));
    }
    Ok((word & 0xff00_001f) | ((((delta >> 2) as u32) & 0x7_ffff) << 5))
}

fn encode_test_branch(word: u32, delta: i64) -> Result<u32, LinkError> {
    if delta & 3 != 0 {
        return Err(LinkError::Native(format!(
            "R_AARCH64_TSTBR14: offset {delta:#x} is not 4-byte aligned"
        )));
    }
    if !(-(1 << 15)..(1 << 15)).contains(&delta) {
        return Err(range_error(R_TSTBR14, delta, 16));
    }
    Ok((word & 0xfff8_001f) | ((((delta >> 2) as u32) & 0x3fff) << 5))
}

impl Target for Aarch64 {
    fn machine(&self) -> u16 {
        EM_AARCH64
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
        self.plan_direct(kind, sym)
    }

    fn apply(
        &self,
        kind: u32,
        plan: Plan,
        ctx: &ApplyCtx,
        bytes: &mut [u8],
        field: usize,
    ) -> Result<(), LinkError> {
        let s = ctx.sym_addr as i64;
        let p = ctx.place_addr as i64;
        let a = ctx.addend;
        if plan == Plan::Skip {
            return Ok(());
        }
        let indirect = matches!(plan, Plan::Plt | Plan::IFuncPlt);
        let branch_target = if indirect { ctx.slot_addr } else { s as u64 };
        let data_target = if matches!(plan, Plan::IFuncPlt) {
            ctx.slot_addr as i64
        } else {
            s
        };
        match kind {
            R_ABS64 => write_le(
                field_bytes(bytes, field, 8)?,
                (data_target.wrapping_add(a)) as u64,
            ),
            R_ABS32 => {
                let v = data_target.wrapping_add(a);
                if v < 0 || v > 0xffff_ffff {
                    return Err(LinkError::Native(format!(
                        "R_AARCH64_ABS32 overflow: {v:#x} exceeds 32 bits"
                    )));
                }
                write_le(field_bytes(bytes, field, 4)?, v as u64)
            }
            R_ABS16 => {
                let v = data_target.wrapping_add(a);
                if v < 0 || v > 0xffff {
                    return Err(LinkError::Native(format!(
                        "R_AARCH64_ABS16 overflow: {v:#x} exceeds 16 bits"
                    )));
                }
                write_le(field_bytes(bytes, field, 2)?, v as u64)
            }
            R_PREL64 => write_le(
                field_bytes(bytes, field, 8)?,
                s.wrapping_add(a).wrapping_sub(p) as u64,
            ),
            R_PREL32 => {
                let v = s.wrapping_add(a).wrapping_sub(p);
                if v != (v as i32) as i64 {
                    return Err(LinkError::Native(format!(
                        "R_AARCH64_PREL32 overflow: {v:#x} does not fit 32 bits"
                    )));
                }
                write_le(field_bytes(bytes, field, 4)?, v as u64)
            }
            R_PREL16 => {
                let v = s.wrapping_add(a).wrapping_sub(p);
                if v != (v as i16) as i64 {
                    return Err(LinkError::Native(format!(
                        "R_AARCH64_PREL16 overflow: {v:#x} does not fit 16 bits"
                    )));
                }
                write_le(field_bytes(bytes, field, 2)?, v as u64)
            }
            R_ADR_PREL_LO21 => {
                let word = read_instr(bytes, field)?;
                let imm = s.wrapping_add(a).wrapping_sub(p);
                write_instr(bytes, field, encode_adr(word, imm, R_ADR_PREL_LO21)?)
            }
            R_ADR_PREL_PG_HI21 | R_ADR_PREL_PG_HI21_NC => {
                let word = read_instr(bytes, field)?;
                let imm = page_delta((s.wrapping_add(a)) as u64, p as u64);
                write_instr(
                    bytes,
                    field,
                    encode_adrp(word, imm, kind == R_ADR_PREL_PG_HI21, kind)?,
                )
            }
            R_ADD_ABS_LO12_NC => {
                let word = read_instr(bytes, field)?;
                let imm = (s.wrapping_add(a)) as u32 & 0xfff;
                write_instr(bytes, field, encode_add_imm(word, imm, false))
            }
            R_LDST8_ABS_LO12_NC | R_LDST16_ABS_LO12_NC | R_LDST32_ABS_LO12_NC
            | R_LDST64_ABS_LO12_NC => {
                let word = read_instr(bytes, field)?;
                let shift = match kind {
                    R_LDST8_ABS_LO12_NC => 0,
                    R_LDST16_ABS_LO12_NC => 1,
                    R_LDST32_ABS_LO12_NC => 2,
                    _ => 3,
                };
                let offset = (s.wrapping_add(a)) as u64 & 0xfff;
                write_instr(
                    bytes,
                    field,
                    encode_ldst_imm(word, offset as i64, shift, kind)?,
                )
            }
            R_TSTBR14 => {
                let word = read_instr(bytes, field)?;
                let target = if indirect { ctx.slot_addr as i64 } else { s }
                    .wrapping_add(a)
                    .wrapping_sub(p);
                write_instr(bytes, field, encode_test_branch(word, target)?)
            }
            R_CONDBR19 => {
                let word = read_instr(bytes, field)?;
                let target = if indirect { ctx.slot_addr as i64 } else { s }
                    .wrapping_add(a)
                    .wrapping_sub(p);
                write_instr(bytes, field, encode_cond_branch(word, target)?)
            }
            R_JUMP26 | R_CALL26 => {
                let word = read_instr(bytes, field)?;
                let delta = (branch_target as i64).wrapping_add(a).wrapping_sub(p);
                write_instr(bytes, field, encode_branch(word, delta, kind)?)
            }
            R_ADR_GOT_PAGE | R_TLSIE_ADR_GOT_PAGE => {
                let word = read_instr(bytes, field)?;
                let imm = page_delta(ctx.slot_addr, p as u64);
                write_instr(bytes, field, encode_adrp(word, imm, true, kind)?)
            }
            R_LD64_GOT_LO12_NC | R_TLSIE_LD64_GOT_LO12_NC => {
                let word = read_instr(bytes, field)?;
                write_instr(
                    bytes,
                    field,
                    encode_ldst_imm(word, (ctx.slot_addr & 0xfff) as i64, 3, kind)?,
                )
            }
            R_TLSLE_ADD_TPREL_HI12 => {
                let word = read_instr(bytes, field)?;
                let imm = ((ctx.tpoff.wrapping_add(a)) as u32 >> 12) & 0xfff;
                write_instr(bytes, field, encode_add_imm(word, imm, true))
            }
            R_TLSLE_ADD_TPREL_LO12 | R_TLSLE_ADD_TPREL_LO12_NC => {
                let word = read_instr(bytes, field)?;
                let imm = (ctx.tpoff.wrapping_add(a)) as u32 & 0xfff;
                write_instr(bytes, field, encode_add_imm(word, imm, false))
            }
            R_TLSLE_LDST8_TPREL_LO12
            | R_TLSLE_LDST8_TPREL_LO12_NC
            | R_TLSLE_LDST16_TPREL_LO12
            | R_TLSLE_LDST16_TPREL_LO12_NC
            | R_TLSLE_LDST32_TPREL_LO12
            | R_TLSLE_LDST32_TPREL_LO12_NC
            | R_TLSLE_LDST64_TPREL_LO12
            | R_TLSLE_LDST64_TPREL_LO12_NC => {
                let word = read_instr(bytes, field)?;
                let shift = match kind {
                    R_TLSLE_LDST8_TPREL_LO12 | R_TLSLE_LDST8_TPREL_LO12_NC => 0,
                    R_TLSLE_LDST16_TPREL_LO12 | R_TLSLE_LDST16_TPREL_LO12_NC => 1,
                    R_TLSLE_LDST32_TPREL_LO12 | R_TLSLE_LDST32_TPREL_LO12_NC => 2,
                    _ => 3,
                };
                let offset = (ctx.tpoff.wrapping_add(a)) as u32 & 0xfff;
                write_instr(
                    bytes,
                    field,
                    encode_ldst_imm(word, offset as i64, shift, kind)?,
                )
            }
            _ => Err(self.unsupported(kind, SymClass::Local)),
        }
    }

    fn tls_tp_offset(&self, dtpoff: u64, _tls_size: u64) -> i64 {
        // AArch64 uses TLS Variant I: the thread pointer names the TCB and the
        // module block starts after it. glibc's TCB is 16 bytes.
        dtpoff as i64 + 16
    }

    fn abs_width(&self, kind: u32) -> Option<u8> {
        match kind {
            R_ABS64 => Some(8),
            R_ABS32 => Some(4),
            R_ABS16 => Some(2),
            _ => None,
        }
    }

    fn plt0_entry(&self, plt_addr: u64, got_plt_addr: u64) -> [u8; 16] {
        stub_bytes(plt_addr, got_plt_addr.wrapping_add(16), true)
    }

    fn plt_entry(&self, entry_addr: u64, slot_addr: u64, _index: u32, _plt0_addr: u64) -> [u8; 16] {
        stub_bytes(entry_addr, slot_addr, false)
    }

    fn iplt_entry(&self, entry_addr: u64, slot_addr: u64) -> [u8; 16] {
        self.plt_entry(entry_addr, slot_addr, 0, 0)
    }

    fn rel_jmpslot(&self) -> u32 {
        R_JUMP_SLOT
    }

    fn rel_globdat(&self) -> u32 {
        R_GLOB_DAT
    }

    fn rel_relative(&self) -> u32 {
        R_RELATIVE
    }

    fn rel_irelative(&self) -> u32 {
        R_IRELATIVE
    }

    fn rel_abs64(&self) -> u32 {
        R_ABS64
    }

    fn rel_copy(&self) -> u32 {
        R_COPY
    }
}

fn stub_bytes(entry_addr: u64, slot_addr: u64, header: bool) -> [u8; 16] {
    let imm = page_delta(slot_addr, entry_addr) as u32 & 0x1f_ffff;
    let adrp = (0x9000_0010 & 0x9f00_001f) | ((imm & 3) << 29) | ((imm >> 2) << 5);
    let lo12 = (slot_addr & 0xfff) as u32;
    let ldr = (0xf940_0211 & 0xffc0_03ff) | ((lo12 >> 3) << 10);
    let mut out = [0u8; 16];
    out[0..4].copy_from_slice(&adrp.to_le_bytes());
    out[4..8].copy_from_slice(&ldr.to_le_bytes());
    if header {
        out[8..12].copy_from_slice(&0xd61f_0220u32.to_le_bytes());
        out[12..16].copy_from_slice(&0xd503_201fu32.to_le_bytes());
    } else {
        let add = (0x9100_0210 & 0xffc0_03ff) | (lo12 << 10);
        out[8..12].copy_from_slice(&add.to_le_bytes());
        out[12..16].copy_from_slice(&0xd61f_0220u32.to_le_bytes());
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ctx(sym: u64, place: u64) -> ApplyCtx {
        ApplyCtx {
            sym_addr: sym,
            place_addr: place,
            slot_addr: 0,
            addend: 0,
            tpoff: 0,
            dtpoff: 0,
        }
    }

    #[test]
    fn adrp_add_encodes_a_page_and_lo12() {
        let mut bytes = 0x9000_0000u32.to_le_bytes();
        let target = 0x1234_5678u64;
        let place = 0x1234_0000u64;
        Aarch64 { static_link: true }
            .apply(
                R_ADR_PREL_PG_HI21,
                Plan::Direct,
                &ctx(target, place),
                &mut bytes,
                0,
            )
            .unwrap();
        let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        let immlo = (word >> 29) & 3;
        let immhi = (word >> 5) & 0x7ffff;
        let imm = ((immhi as i64) << 2 | immlo as i64) << 12;
        assert_eq!(imm, 0x5000);

        let mut add = 0x9100_0000u32.to_le_bytes();
        Aarch64 { static_link: true }
            .apply(
                R_ADD_ABS_LO12_NC,
                Plan::Direct,
                &ctx(target, place),
                &mut add,
                0,
            )
            .unwrap();
        let word = u32::from_le_bytes([add[0], add[1], add[2], add[3]]);
        assert_eq!((word >> 10) & 0xfff, 0x678);
    }

    #[test]
    fn branch_and_conditional_encodings_round_trip() {
        let arm = Aarch64 { static_link: true };
        for (kind, base, mask, shift, bits) in [
            (R_JUMP26, 0x1400_0000u32, 0x03ff_ffffu32, 2u32, 28),
            (R_CALL26, 0x9400_0000u32, 0x03ff_ffffu32, 2u32, 28),
        ] {
            let mut bytes = base.to_le_bytes();
            let place = 0x1000u64;
            let target = place + 0x40;
            arm.apply(kind, Plan::Direct, &ctx(target, place), &mut bytes, 0)
                .unwrap();
            let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
            assert_eq!((word & mask) << shift, 0x10 << shift, "imm for {kind}");
            let far = place + (1i64 << (bits - 1)) as u64;
            let mut bytes = base.to_le_bytes();
            assert!(
                arm.apply(kind, Plan::Direct, &ctx(far, place), &mut bytes, 0)
                    .is_err()
            );
        }
        let mut bytes = 0x5400_0000u32.to_le_bytes();
        arm_apply_cond(&mut bytes, 0x20, 0x1000);
        let word = u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]);
        assert_eq!((word >> 5) & 0x7_ffff, 8);
    }

    fn arm_apply_cond(bytes: &mut [u8], target_off: u64, place: u64) {
        Aarch64 { static_link: true }
            .apply(
                R_CONDBR19,
                Plan::Direct,
                &ctx(place + target_off, place),
                bytes,
                0,
            )
            .unwrap();
    }

    #[test]
    fn range_and_alignment_checks_are_explicit() {
        let arm = Aarch64 { static_link: true };
        let place = 0x1000u64;
        let mut bytes = 0x1400_0000u32.to_le_bytes();
        let err = arm
            .apply(
                R_JUMP26,
                Plan::Direct,
                &ctx(place + 2, place),
                &mut bytes,
                0,
            )
            .unwrap_err();
        assert!(err.to_string().contains("4-byte aligned"), "{err}");
        let err = arm
            .apply(
                R_TSTBR14,
                Plan::Direct,
                &ctx(place + (1 << 20), place),
                &mut bytes,
                0,
            )
            .unwrap_err();
        assert!(err.to_string().contains("overflow"), "{err}");
        let mut ldst = 0xf940_0000u32.to_le_bytes();
        let err = arm
            .apply(
                R_LDST64_ABS_LO12_NC,
                Plan::Direct,
                &ctx(place + 4, place),
                &mut ldst,
                0,
            )
            .unwrap_err();
        assert!(err.to_string().contains("aligned to 8 bytes"), "{err}");
    }

    #[test]
    fn general_dynamic_tls_is_rejected_not_miscompiled() {
        let err = Aarch64 { static_link: false }
            .plan(R_TLSGD_ADR_PAGE21, SymClass::Tls)
            .unwrap_err();
        assert!(err.to_string().contains("TLSGD_ADR_PAGE21"), "{err}");
    }

    #[test]
    fn dynamic_relocation_numbers_match_the_aarch64_abi() {
        let arm = Aarch64 { static_link: false };
        assert_eq!(arm.rel_jmpslot(), 1026);
        assert_eq!(arm.rel_globdat(), 1025);
        assert_eq!(arm.rel_relative(), 1027);
        assert_eq!(arm.rel_irelative(), 1032);
        assert_eq!(arm.rel_abs64(), 257);
        assert_eq!(arm.rel_copy(), 1024);
        assert_eq!(arm.plt_entry(0, 0x2000, 0, 0).len(), 16);
        assert_eq!(arm.iplt_entry(0, 0x2000).len(), 16);
    }

    #[test]
    fn plt_stub_targets_the_got_slot() {
        let out = Aarch64 { static_link: false }.plt_entry(0x1000, 0x3000, 0, 0);
        let adrp = u32::from_le_bytes([out[0], out[1], out[2], out[3]]);
        assert_eq!(adrp & 0x9f00_001f, 0x9000_0010);
        let ldr = u32::from_le_bytes([out[4], out[5], out[6], out[7]]);
        assert_eq!(ldr & 0xffc0_03ff, 0xf940_0211);
        let add = u32::from_le_bytes([out[8], out[9], out[10], out[11]]);
        assert_eq!(add & 0xffc0_03ff, 0x9100_0210);
        assert_eq!(&out[12..16], &0xd61f_0220u32.to_le_bytes());
    }
}
