use crate::core::obj::{
    Object, Section, SHF_ALLOC, SHF_EXECINSTR, SHF_TLS, SHF_WRITE, SHN_ABS, SHN_COMMON, SHN_UNDEF, SHT_GROUP,
    SHT_GNU_VERSYM, SHT_NOBITS, SHT_NULL, SHT_RELA, SHT_STRTAB, SHT_SYMTAB, STB_GLOBAL, STB_WEAK, STT_FILE,
    STT_GNU_IFUNC, STT_SECTION, STT_TLS, STV_HIDDEN,
};
use crate::core::symdb::{Def, SymDb};
use crate::target::x86_64::{R_GOTPCRELX, R_REX_GOTPCRELX};
use crate::target::{Plan, SymClass, Target};
use crate::LinkError;
use std::collections::{BTreeMap, BTreeSet, VecDeque};

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum SecClass {
    Skip,
    Text,
    Rodata,
    EhFrame,
    ExceptTable,
    InitArray,
    FiniArray,
    Init,
    Fini,
    DataRelRo,
    Data,
    Bss,
    Tdata,
    Tbss,
    Note,
    NoteProperty,
    GnuStack,
    Debug,
}

pub fn classify(sec: &Section, name: &str, keep_debug: bool) -> SecClass {
    if sec.kind == SHT_NULL
        || sec.kind == SHT_SYMTAB
        || sec.kind == SHT_STRTAB
        || sec.kind == SHT_RELA
        || sec.kind == SHT_GROUP
        || sec.kind == SHT_GNU_VERSYM
    {
        return SecClass::Skip;
    }
    if name == ".note.GNU-stack" {
        return SecClass::GnuStack;
    }
    if name == ".note.gnu.property" {
        return SecClass::NoteProperty;
    }
    if name == ".note.stapsdt" {
        return SecClass::Skip;
    }
    if name.starts_with(".debug") {
        if keep_debug {
            return SecClass::Debug;
        }
        return SecClass::Skip;
    }
    if name.starts_with(".comment")
        || name.starts_with(".llvm")
        || name == ".sframe"
        || name == ".eh_frame_hdr"
    {
        return SecClass::Skip;
    }
    if name == ".eh_frame" {
        return SecClass::EhFrame;
    }
    if name.starts_with(".gcc_except_table") {
        return SecClass::ExceptTable;
    }
    if sec.kind == 14 || name.starts_with(".init_array") || name.starts_with(".ctors") || name.starts_with(".preinit_array") {
        return SecClass::InitArray;
    }
    if sec.kind == 15 || name.starts_with(".fini_array") || name.starts_with(".dtors") {
        return SecClass::FiniArray;
    }
    if name == ".init" {
        return SecClass::Init;
    }
    if name == ".fini" {
        return SecClass::Fini;
    }
    if sec.flags & SHF_TLS != 0 {
        if sec.kind == SHT_NOBITS {
            return SecClass::Tbss;
        }
        return SecClass::Tdata;
    }
    if sec.kind == 7 {
        return SecClass::Note;
    }
    if sec.flags & SHF_ALLOC == 0 {
        return SecClass::Skip;
    }
    if sec.flags & SHF_EXECINSTR != 0 {
        return SecClass::Text;
    }
    if sec.kind == SHT_NOBITS {
        return SecClass::Bss;
    }
    if sec.flags & SHF_WRITE != 0 {
        if name.starts_with(".data.rel.ro") {
            return SecClass::DataRelRo;
        }
        return SecClass::Data;
    }
    SecClass::Rodata
}

pub struct ObjData {
    pub label: String,
    pub command_line: bool,
    pub classes: Vec<SecClass>,
    pub local_target: Vec<Option<(u32, u64)>>,
}

#[derive(Clone, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum GotKey {
    Named(String),
    Local(usize, u32, u64),
}

#[derive(Clone, Debug)]
pub struct GotSlot {
    pub key: GotKey,
    pub ifunc: bool,
    pub dynamic: bool,
}

#[derive(Clone, Debug)]
pub struct PltSlot {
    pub name: String,
    pub ifunc: bool,
}

#[derive(Clone, Debug)]
pub struct TlsInfo {
    pub order: Vec<(usize, u32)>,
    pub dtpoff: BTreeMap<(usize, u32), u64>,
    pub size: u64,
    pub align: u64,
    pub tdata_end: u64,
}

pub struct CieInfo {
    pub aug_r: u8,
    pub has_lsda: bool,
    pub lsda_enc: u8,
}

pub struct FdeSpan {
    pub start: usize,
    pub end: usize,
    pub pc_off: usize,
    pub pc_size: usize,
    pub aug_off: usize,
    pub aug_len: usize,
    pub cie: CieInfo,
    pub cie_start: usize,
    pub cie_end: usize,
}

fn read_uleb(bytes: &[u8], pos: &mut usize) -> Option<u64> {
    let mut r = 0u64;
    let mut s = 0u32;
    loop {
        let b = *bytes.get(*pos)?;
        *pos += 1;
        r |= ((b & 0x7f) as u64) << s;
        s += 7;
        if b & 0x80 == 0 {
            break;
        }
        if s > 63 {
            return None;
        }
    }
    Some(r)
}

fn enc_size(enc: u8) -> usize {
    if enc == 0xff {
        return 0;
    }
    match enc & 0x0f {
        0 => 1,
        1 => 2,
        2 => 4,
        _ => 8,
    }
}

pub fn walk_eh_frame(bytes: &[u8]) -> Vec<FdeSpan> {
    let mut out = Vec::new();
    let mut off = 0usize;
    let mut cie = CieInfo { aug_r: 0x1b, has_lsda: false, lsda_enc: 0xff };
    let mut cie_start = 0usize;
    let mut cie_end = 0usize;
    while off + 8 <= bytes.len() {
        let len = u32::from_le_bytes(bytes[off..off + 4].try_into().unwrap()) as usize;
        if len == 0 {
            break;
        }
        let total = 4 + ((len + 3) & !3);
        if off + total > bytes.len() {
            break;
        }
        let id = u32::from_le_bytes(bytes[off + 4..off + 8].try_into().unwrap());
        if id == 0 {
            let mut info = CieInfo { aug_r: 0x1b, has_lsda: false, lsda_enc: 0xff };
            let mut k = off + 9;
            let mut aug = Vec::new();
            while k < off + 4 + len && bytes[k] != 0 {
                aug.push(bytes[k]);
                k += 1;
            }
            k += 1;
            if aug.first() == Some(&b'z') {
                let mut p = k;
                read_uleb(bytes, &mut p);
                if bytes.get(off + 8) == Some(&1) {
                    p += 1;
                } else {
                    read_uleb(bytes, &mut p);
                }
                let alen = read_uleb(bytes, &mut p).unwrap_or(0) as usize;
                let ad = bytes.get(p..p + alen).unwrap_or(&[]);
                let mut q = 0usize;
                if aug.contains(&b'L') && q < ad.len() {
                    info.has_lsda = true;
                    info.lsda_enc = ad[q];
                    q += 1;
                }
                if aug.contains(&b'P') && q < ad.len() {
                    let e = ad[q];
                    q += 1 + enc_size(e);
                }
                if aug.contains(&b'R') && q < ad.len() {
                    info.aug_r = ad[q];
                }
            }
            cie = info;
            cie_start = off;
            cie_end = off + total;
        } else {
            let pc_size = enc_size(cie.aug_r);
            if pc_size == 0 {
                break;
            }
            let aug_off = off + 8 + pc_size * 2;
            let mut aug_len = 0usize;
            if cie.has_lsda {
                let mut p = aug_off;
                if let Some(v) = read_uleb(bytes, &mut p) {
                    aug_len = p - aug_off + v as usize;
                }
            }
            out.push(FdeSpan { start: off, end: off + total, pc_off: off + 8, pc_size, aug_off, aug_len, cie: CieInfo { aug_r: cie.aug_r, has_lsda: cie.has_lsda, lsda_enc: cie.lsda_enc }, cie_start, cie_end });
        }
        off += total;
    }
    out
}

pub struct CieEntry {
    pub body: Vec<u8>,
    pub total: u64,
    pub live: bool,
    pub first: (usize, u32, usize),
}

pub struct FdeOut {
    pub start: usize,
    pub end: usize,
    pub cie: usize,
    pub out_off: u64,
}

pub struct EhSecPlan {
    pub fdes: Vec<FdeOut>,
    pub out_off: u64,
    pub out_size: u64,
}

pub struct EhPlan {
    pub cies: Vec<CieEntry>,
    pub cie_index: BTreeMap<Vec<u8>, usize>,
    pub sections: BTreeMap<(usize, u32), EhSecPlan>,
    pub dead_rel: BTreeSet<(usize, u32, u64)>,
    pub total: u64,
}

fn cie_masked_key(
    bytes: &[u8],
    start: usize,
    end: usize,
    obj: &Object,
    resolved: &[Option<TargetSym>],
    sec_relas: &[usize],
) -> Vec<u8> {
    let mut key = bytes[start..end].to_vec();
    let mut extra: Vec<(usize, u32, u64)> = Vec::new();
    for &i in sec_relas.iter() {
        let r = &obj.relas[i];
        let o = r.offset as usize;
        if o >= start && o < end {
            let w = reloc_field_width(r.kind);
            for i in 0..w {
                if o + i - start < key.len() {
                    key[o + i - start] = 0;
                }
            }
            let tid = match resolved.get(r.sym as usize).and_then(|o| o.as_ref()) {
                Some(t) => {
                    if !t.name.is_empty() {
                        fnv(t.name.as_bytes())
                    } else {
                        fnv(&[t.obj as u8, t.sec as u8])
                            ^ t.value.wrapping_mul(0x9e3779b97f4a7c15)
                    }
                }
                None => r.sym as u64,
            };
            extra.push((o - start, r.kind, tid));
        }
    }
    extra.sort();
    for (o, k, tid) in extra {
        key.extend_from_slice(&(o as u64).to_le_bytes());
        key.extend_from_slice(&k.to_le_bytes());
        key.extend_from_slice(&tid.to_le_bytes());
    }
    key
}

pub struct Graph {
    pub objs: Vec<ObjData>,
    pub kept: Vec<Vec<bool>>,
    pub rela_idx: Vec<Vec<Vec<usize>>>,
    pub got: Vec<GotSlot>,
    pub got_index: BTreeMap<GotKey, usize>,
    pub plt: Vec<PltSlot>,
    pub plt_index: BTreeMap<String, usize>,
    pub plt_got: BTreeMap<usize, usize>,
    pub tls: TlsInfo,
    pub commons: Vec<(String, u64, u64)>,
    pub common_addr: BTreeMap<String, u64>,
    pub entry: Option<(usize, u32, u64)>,
    pub consumed_call: BTreeSet<(usize, u32, u64)>,
    pub icf_redirect: BTreeMap<(usize, u32), (usize, u32)>,
    pub icf_align: BTreeMap<(usize, u32), u64>,
    pub eh: EhPlan,
    pub eh_cie_off: Vec<u64>,
    pub merged_off: BTreeMap<(usize, u32), u64>,
    pub used_names: BTreeSet<String>,
    pub resolved: Vec<Vec<Option<TargetSym>>>,
    pub relaxed: BTreeSet<(usize, u32, u64)>,
    pub needed_libs: BTreeSet<String>,
    pub exec_stack: bool,
}

pub struct DynSym {
    pub func: bool,
    pub lib: String,
}

pub fn build_object(
    obj: &Object,
    oi: usize,
    label: &str,
    command_line: bool,
    keep_debug: bool,
    db: &mut SymDb,
    winners: &mut BTreeMap<String, usize>,
) -> Result<(ObjData, Vec<String>, Vec<String>), LinkError> {
    let labels = |o: usize| if o == oi { label.to_string() } else { format!("object {o}") };
    let mut loser_secs: BTreeSet<u32> = BTreeSet::new();
    for grp in obj.groups.iter() {
        let signame = obj.symbol_name(grp.signature as usize)?.to_string();
        match winners.get(&signame) {
            Some(_) => {
                for &m in grp.members.iter() {
                    loser_secs.insert(m);
                }
            }
            None => {
                winners.insert(signame, oi);
            }
        }
    }
    let mut classes = Vec::with_capacity(obj.sections.len());
    for (si, sec) in obj.sections.iter().enumerate() {
        if loser_secs.contains(&(si as u32)) {
            classes.push(SecClass::Skip);
            continue;
        }
        let name = obj.section_name(si).unwrap_or("");
        classes.push(classify(sec, name, keep_debug));
    }
    let mut local_target: Vec<Option<(u32, u64)>> = vec![None; obj.symbols.len()];
    let mut defined: Vec<String> = Vec::new();
    let mut referenced: Vec<String> = Vec::new();
    for (sym_idx, sym) in obj.symbols.iter().enumerate() {
        if sym.bind == STB_GLOBAL || sym.bind == STB_WEAK {
            continue;
        }
        if sym.kind == STT_SECTION {
            local_target[sym_idx] = Some((sym.shndx, 0));
        } else if sym.kind != STT_FILE && sym.shndx != SHN_UNDEF && sym.shndx != SHN_ABS && sym.shndx != SHN_COMMON {
            local_target[sym_idx] = Some((sym.shndx, sym.value));
        }
    }
    for (sym_idx, sym) in obj.symbols.iter().enumerate() {
        if sym.bind != STB_GLOBAL && sym.bind != STB_WEAK {
            continue;
        }
        if sym.kind == STT_FILE {
            continue;
        }
        let name = obj.symbol_name(sym_idx)?.to_string();
        if loser_secs.contains(&sym.shndx) {
            continue;
        }
        if sym.shndx == SHN_UNDEF {
            db.note_ref(&name, oi, sym.bind == STB_GLOBAL, sym.vis == STV_HIDDEN, sym.kind);
            referenced.push(name);
        } else if sym.shndx == SHN_COMMON {
            if sym.bind == STB_GLOBAL {
                db.define_common(&name, sym.size, sym.value);
                defined.push(name);
            }
        } else if sym.shndx == SHN_ABS {
            db.define(
                &name,
                Def {
                    obj: oi,
                    sec: u32::MAX,
                    value: sym.value,
                    size: sym.size,
                    bind: sym.bind,
                    kind: sym.kind,
                    vis: sym.vis,
                    tls: sym.kind == STT_TLS,
                    ifunc: sym.kind == STT_GNU_IFUNC,
                    absolute: true,
                },
                label,
                &labels,
            )?;
            defined.push(name);
        } else {
            db.define(
                &name,
                Def {
                    obj: oi,
                    sec: sym.shndx,
                    value: sym.value,
                    size: sym.size,
                    bind: sym.bind,
                    kind: sym.kind,
                    vis: sym.vis,
                    tls: sym.kind == STT_TLS,
                    ifunc: sym.kind == STT_GNU_IFUNC,
                    absolute: false,
                },
                label,
                &labels,
            )?;
            defined.push(name);
        }
    }
    let mut tls_calls: BTreeSet<u64> = BTreeSet::new();
    for rela in obj.relas.iter() {
        if rela.kind != 19 && rela.kind != 20 {
            continue;
        }
        let delta = if rela.kind == 19 { 8u64 } else { 5u64 };
        if obj.relas.iter().any(|q| q.section == rela.section && q.offset == rela.offset + delta && q.kind == 4) {
            tls_calls.insert(rela.offset + delta);
        }
    }
    for rela in obj.relas.iter() {
        let si = rela.section as usize;
        if si >= classes.len() || classes[si] == SecClass::Skip {
            continue;
        }
        if tls_calls.contains(&rela.offset) {
            continue;
        }
        let sym = obj.symbols.get(rela.sym as usize).ok_or_else(|| LinkError::Native(format!("{label}: bad relocation symbol")))?;
        if sym.bind == STB_GLOBAL || sym.bind == STB_WEAK {
            let name = obj.symbol_name(rela.sym as usize)?.to_string();
            db.note_ref(&name, oi, sym.bind == STB_GLOBAL, sym.vis == STV_HIDDEN, sym.kind);
            referenced.push(name);
        }
    }
    Ok((ObjData { label: label.to_string(), command_line, classes, local_target }, defined, referenced))
}

#[derive(Clone)]
pub struct TargetSym {
    pub obj: usize,
    pub sec: u32,
    pub value: u64,
    pub ifunc: bool,
    pub tls: bool,
    pub dynamic: bool,
    pub dyn_func: bool,
    pub weak_zero: bool,
    pub common: bool,
    pub name: String,
}

pub fn resolve_sym(
    oi: usize,
    label: &str,
    obj: &Object,
    locals: &[Option<(u32, u64)>],
    sym_idx: usize,
    db: &SymDb,
    dyn_syms: &BTreeMap<String, DynSym>,
) -> Result<TargetSym, LinkError> {
    let sym = obj.symbols.get(sym_idx).ok_or_else(|| LinkError::Native(format!("{label}: bad relocation symbol")))?;
    if sym.bind != STB_GLOBAL && sym.bind != STB_WEAK {
        match locals.get(sym_idx).copied().flatten() {
            Some((sec, val)) => {
                return Ok(TargetSym {
                    obj: oi,
                    sec,
                    value: val,
                    ifunc: false,
                    tls: sym.kind == STT_TLS,
                    dynamic: false,
                    dyn_func: false,
                    weak_zero: false,
                    common: false,
                    name: String::new(),
                });
            }
            None => return Err(LinkError::Native(format!("{label}: unsupported local relocation target"))),
        }
    }
    let name = obj.symbol_name(sym_idx)?.to_string();
    if let Some(e) = db.get(&name) {
        if e.common_size > 0 && e.def.as_ref().is_none_or(|d| d.bind != STB_GLOBAL) {
            return Ok(TargetSym {
                obj: usize::MAX,
                sec: u32::MAX,
                value: 0,
                ifunc: false,
                tls: false,
                dynamic: false,
                dyn_func: false,
                weak_zero: false,
                common: true,
                name,
            });
        }
        if let Some(d) = e.def.as_ref() {
            return Ok(TargetSym {
                obj: d.obj,
                sec: d.sec,
                value: d.value,
                ifunc: d.ifunc,
                tls: d.tls,
                dynamic: false,
                dyn_func: false,
                weak_zero: false,
                common: false,
                name,
            });
        }
    }
    if let Some(ds) = dyn_syms.get(&name) {
        return Ok(TargetSym {
            obj: usize::MAX,
            sec: u32::MAX,
            value: 0,
            ifunc: false,
            tls: false,
            dynamic: true,
            dyn_func: ds.func,
            weak_zero: false,
            common: false,
            name,
        });
    }
    if sym.bind == STB_WEAK {
        return Ok(TargetSym {
            obj: oi,
            sec: u32::MAX,
            value: 0,
            ifunc: false,
            tls: false,
            dynamic: false,
            dyn_func: false,
            weak_zero: true,
            common: false,
            name,
        });
    }
    Err(LinkError::Native(format!("undefined symbol `{name}` referenced by {label}")))
}

pub fn sym_class(t: &TargetSym) -> SymClass {
    if t.weak_zero {
        SymClass::WeakUndef
    } else if t.ifunc {
        SymClass::Ifunc
    } else if t.tls {
        SymClass::Tls
    } else if t.dynamic {
        if t.dyn_func { SymClass::DynamicFunc } else { SymClass::DynamicData }
    } else {
        SymClass::Local
    }
}

pub fn rela_index(obj: &Object) -> Vec<Vec<usize>> {
    let mut idx: Vec<Vec<usize>> = vec![Vec::new(); obj.sections.len()];
    for (i, r) in obj.relas.iter().enumerate() {
        if (r.section as usize) < idx.len() {
            idx[r.section as usize].push(i);
        }
    }
    for v in idx.iter_mut() {
        v.sort_by_key(|&i| obj.relas[i].offset);
    }
    idx
}

pub fn mark(
    parsed: &[Object],
    objs: Vec<ObjData>,
    db: &SymDb,
    dyn_syms: &BTreeMap<String, DynSym>,
    entry_name: &str,
    keep_debug: bool,
) -> Result<Graph, LinkError> {
    let mut kept: Vec<Vec<bool>> = parsed.iter().map(|o| vec![false; o.sections.len()]).collect();
    let mut queue: VecDeque<(usize, u32)> = VecDeque::new();
    let push = |kept: &mut Vec<Vec<bool>>, queue: &mut VecDeque<(usize, u32)>, oi: usize, sec: u32| {
        let s = sec as usize;
        if s < kept[oi].len() && !kept[oi][s] {
            kept[oi][s] = true;
            queue.push_back((oi, sec));
        }
    };
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, _sec) in obj.sections.iter().enumerate() {
            match objs[oi].classes[si] {
                SecClass::Init | SecClass::InitArray | SecClass::Fini | SecClass::FiniArray | SecClass::Note => {
                    push(&mut kept, &mut queue, oi, si as u32);
                }
                SecClass::Debug if keep_debug => {
                    push(&mut kept, &mut queue, oi, si as u32);
                }
                _ => {}
            }
        }
    }
    match db.get(entry_name).and_then(|e| e.def.as_ref()) {
        Some(d) if d.obj != usize::MAX && d.sec != u32::MAX => push(&mut kept, &mut queue, d.obj, d.sec),
        _ => return Err(LinkError::Native(format!("entry symbol `{entry_name}` is undefined"))),
    }
    use rayon::prelude::*;
    let resolved: Vec<Vec<Option<TargetSym>>> = parsed
        .par_iter()
        .enumerate()
        .map(|(oi, obj)| {
            let mut row: Vec<Option<TargetSym>> = vec![None; obj.symbols.len()];
            let mut seen = vec![false; row.len()];
            for rela in obj.relas.iter() {
                let sym_idx = rela.sym as usize;
                if sym_idx < row.len() && !seen[sym_idx] {
                    seen[sym_idx] = true;
                    row[sym_idx] = resolve_sym(oi, &objs[oi].label, obj, &objs[oi].local_target, sym_idx, db, dyn_syms).ok();
                }
            }
            row
        })
        .collect();
    let ridx: Vec<Vec<Vec<usize>>> = parsed.iter().map(rela_index).collect();
    let eh_pc: BTreeMap<(usize, u32), Vec<(u64, u64)>> = parsed
        .par_iter()
        .enumerate()
        .flat_map(|(oi, obj)| {
            let mut out = Vec::new();
            for (si, c) in objs[oi].classes.iter().enumerate() {
                if *c != SecClass::EhFrame {
                    continue;
                }
                let bytes = obj.section_bytes(si).unwrap_or(&[]);
                let mut ranges: Vec<(u64, u64)> = walk_eh_frame(bytes)
                    .into_iter()
                    .map(|f| (f.pc_off as u64, f.pc_off as u64 + f.pc_size as u64))
                    .collect();
                ranges.sort();
                let mut merged: Vec<(u64, u64)> = Vec::with_capacity(ranges.len());
                for r in ranges {
                    if let Some(last) = merged.last_mut() {
                        if r.0 <= last.1 {
                            last.1 = last.1.max(r.1);
                            continue;
                        }
                    }
                    merged.push(r);
                }
                out.push(((oi, si as u32), merged));
            }
            out
        })
        .collect();
    let follow = |oi: usize, sec: u32, off: u64| -> bool {
        if objs[oi].classes.get(sec as usize).copied().unwrap_or(SecClass::Skip) != SecClass::EhFrame {
            return true;
        }
        !eh_pc.get(&(oi, sec)).is_some_and(|ranges| {
            let i = ranges.partition_point(|r| r.0 <= off);
            i > 0 && off < ranges[i - 1].1
        })
    };
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in objs[oi].classes.iter().enumerate() {
            if *c != SecClass::EhFrame {
                continue;
            }
            let targets: Vec<(usize, u32)> = ridx[oi][si]
                .iter()
                .map(|&i| &obj.relas[i])
                .filter(|r| follow(oi, si as u32, r.offset))
                .filter_map(|r| resolved[oi].get(r.sym as usize).and_then(|o| o.as_ref()))
                .filter(|t| !t.dynamic && !t.weak_zero && !t.common && t.obj != usize::MAX && t.sec != u32::MAX)
                .filter(|t| (t.sec as usize) < kept[t.obj].len() && objs[t.obj].classes[t.sec as usize] != SecClass::Skip)
                .map(|t| (t.obj, t.sec))
                .collect();
            for (to, ts) in targets {
                push(&mut kept, &mut queue, to, ts);
            }
        }
    }
    while let Some((oi, sec)) = queue.pop_front() {
        if (sec as usize) >= ridx[oi].len() {
            continue;
        }
        if objs[oi].classes.get(sec as usize).copied().unwrap_or(SecClass::Skip) == SecClass::Debug {
            continue;
        }
        for &ri in ridx[oi][sec as usize].iter() {
            let rela = &parsed[oi].relas[ri];
            if !follow(oi, sec, rela.offset) {
                continue;
            }
            let t = match resolved[oi].get(rela.sym as usize).and_then(|o| o.as_ref()) {
                Some(t) => t,
                None => continue,
            };
            if t.dynamic || t.weak_zero || t.common || t.obj == usize::MAX {
                continue;
            }
            if t.sec == u32::MAX {
                continue;
            }
            let s = t.sec as usize;
            if s < kept[t.obj].len() && objs[t.obj].classes[s] != SecClass::Skip {
                push(&mut kept, &mut queue, t.obj, t.sec);
            }
        }
    }
    for oi in 0..parsed.len() {
        let has_text = objs[oi].classes.iter().zip(kept[oi].iter()).any(|(&c, &k)| k && c == SecClass::Text);
        if has_text {
            for (si, c) in objs[oi].classes.iter().enumerate() {
                if *c == SecClass::EhFrame {
                    kept[oi][si] = true;
                }
            }
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        let mut extra: Vec<(usize, u32)> = Vec::new();
        for (oi, obj) in parsed.iter().enumerate() {
            for g in obj.groups.iter() {
                let any = g.members.iter().any(|&m| kept[oi].get(m as usize).copied().unwrap_or(false));
                if any {
                    for &m in g.members.iter() {
                        let s = m as usize;
                        if s < kept[oi].len()
                            && !kept[oi][s]
                            && objs[oi].classes[s] != SecClass::Skip
                            && parsed[oi].sections[s].kind != SHT_RELA
                        {
                            extra.push((oi, m));
                        }
                    }
                }
            }
        }
        for (oi, sec) in extra {
            if !kept[oi][sec as usize] {
                kept[oi][sec as usize] = true;
                queue.push_back((oi, sec));
                changed = true;
            }
        }
        while let Some((oi, sec)) = queue.pop_front() {
            if (sec as usize) >= ridx[oi].len() {
                continue;
            }
            if objs[oi].classes.get(sec as usize).copied().unwrap_or(SecClass::Skip) == SecClass::Debug {
                continue;
            }
            for &ri in ridx[oi][sec as usize].iter() {
                let rela = &parsed[oi].relas[ri];
                if !follow(oi, sec, rela.offset) {
                    continue;
                }
                let t = match resolved[oi].get(rela.sym as usize).and_then(|o| o.as_ref()) {
                    Some(t) => t,
                    None => continue,
                };
                if t.dynamic || t.weak_zero || t.common || t.obj == usize::MAX || t.sec == u32::MAX {
                    continue;
                }
                let s = t.sec as usize;
                if s < kept[t.obj].len() && objs[t.obj].classes[s] != SecClass::Skip && !kept[t.obj][s] {
                    kept[t.obj][s] = true;
                    queue.push_back((t.obj, t.sec));
                    changed = true;
                }
            }
        }
    }
    let entry = match db.get(entry_name).and_then(|e| e.def.as_ref()) {
        Some(d) if d.obj != usize::MAX && d.sec != u32::MAX => Some((d.obj, d.sec, d.value)),
        _ => return Err(LinkError::Native(format!("entry symbol `{entry_name}` is undefined"))),
    };
    let g = Graph {
        objs,
        kept,
        got: Vec::new(),
        got_index: BTreeMap::new(),
        plt: Vec::new(),
        plt_index: BTreeMap::new(),
        plt_got: BTreeMap::new(),
        tls: TlsInfo { order: Vec::new(), dtpoff: BTreeMap::new(), size: 0, align: 1, tdata_end: 0 },
        commons: Vec::new(),
        common_addr: BTreeMap::new(),
        entry,
        consumed_call: BTreeSet::new(),
        icf_redirect: BTreeMap::new(),
        icf_align: BTreeMap::new(),
        eh: EhPlan { cies: Vec::new(), cie_index: BTreeMap::new(), sections: BTreeMap::new(), dead_rel: BTreeSet::new(), total: 0 },
        eh_cie_off: Vec::new(),
        merged_off: BTreeMap::new(),
        used_names: BTreeSet::new(),
        resolved,
        relaxed: BTreeSet::new(),
        needed_libs: BTreeSet::new(),
        exec_stack: false,
        rela_idx: ridx,
    };
    Ok(g)
}

fn fnv(data: &[u8]) -> u64 {
    let mut h = 0xcbf29ce484222325u64;
    for &b in data {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}

fn reloc_field_width(kind: u32) -> usize {
    match kind {
        1 | 24 => 8,
        _ => 4,
    }
}


pub fn icf_fold(g: &mut Graph, parsed: &[Object]) -> Result<(), LinkError> {
    use rayon::prelude::*;
    let mut ids: Vec<(usize, u32)> = Vec::new();
    for (oi, _obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if g.kept[oi][si] && *c != SecClass::Skip {
                ids.push((oi, si as u32));
            }
        }
    }
    let built: Vec<((usize, u32), u64, Vec<(u64, u32, i64, u64, Option<(usize, u32, u64)>)>)> = ids
        .par_iter()
        .map(|&(oi, si)| {
            let obj = &parsed[oi];
            let bytes = obj.section_bytes(si as usize)?;
            let mut masked = bytes.to_vec();
            let mut rl = Vec::new();
            for &i in g.rela_idx[oi][si as usize].iter() {
                let r = &obj.relas[i];
                let w = reloc_field_width(r.kind);
                let o = r.offset as usize;
                if o + w <= masked.len() {
                    for b in masked[o..o + w].iter_mut() {
                        *b = 0;
                    }
                }
                let t = g.resolved[oi].get(r.sym as usize).and_then(|o| o.as_ref());
                let (tid, target) = match t {
                    Some(t) => {
                        if t.dynamic || t.weak_zero || t.common {
                            (fnv(t.name.as_bytes()), None)
                        } else if t.obj == usize::MAX || t.sec == u32::MAX {
                            (0, None)
                        } else {
                            (0, Some((t.obj, t.sec, t.value)))
                        }
                    }
                    None => (0, None),
                };
                rl.push((r.offset, r.kind, r.addend, tid, target));
            }
            rl.sort();
            Ok(((oi, si), fnv(&masked), rl))
        })
        .collect::<Result<Vec<_>, LinkError>>()?;
    let mut pos: BTreeMap<(usize, u32), usize> = BTreeMap::new();
    for (p, id) in ids.iter().enumerate() {
        pos.insert(*id, p);
    }
    let mut base: Vec<u64> = Vec::with_capacity(ids.len());
    let mut sigs: Vec<Vec<(u64, u32, i64, u64, Option<(Option<usize>, u64)>)>> = Vec::with_capacity(ids.len());
    for (_id, h, rl) in built {
        base.push(h);
        let mut v = Vec::with_capacity(rl.len());
        for (off, kind, add, tid, target) in rl {
            let t = target.map(|(to, ts, tv)| (pos.get(&(to, ts)).copied(), tv));
            v.push((off, kind, add, tid, t));
        }
        sigs.push(v);
    }
    let mut keys = base.clone();
    for _ in 0..16 {
        let next: Vec<u64> = (0..ids.len())
            .into_par_iter()
            .map(|p| {
                let mut h = base[p];
                let mut sig: Vec<(u64, u32, i64, u64)> = Vec::with_capacity(sigs[p].len());
                for (off, kind, add, tid, target) in sigs[p].iter() {
                    let tid = match target {
                        Some((Some(tp), tv)) => keys[*tp] ^ tv.wrapping_mul(0x9e3779b97f4a7c15),
                        Some((None, tv)) => tv.wrapping_mul(0x9e3779b97f4a7c15),
                        None => *tid,
                    };
                    sig.push((*off, *kind, *add, tid));
                }
                sig.sort();
                for (off, kind, add, tid) in sig {
                    h ^= off.wrapping_mul(0x9e3779b97f4a7c15).wrapping_add(kind as u64).wrapping_add(add as u64);
                    h = h.wrapping_mul(0x100000001b3);
                    h ^= tid;
                    h = h.wrapping_mul(0x100000001b3);
                }
                h
            })
            .collect();
        if next == keys {
            break;
        }
        keys = next;
    }
    let keys: BTreeMap<(usize, u32), u64> = ids.iter().copied().zip(keys).collect();
    let mut groups: BTreeMap<u64, Vec<(usize, u32)>> = BTreeMap::new();
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if *c != SecClass::Text || !g.kept[oi][si] {
                continue;
            }
            if obj.sections[si].size == 0 {
                continue;
            }
            if g.rela_idx[oi][si].iter().any(|&i| matches!(obj.relas[i].kind, 1 | 10 | 11)) {
                continue;
            }
            groups.entry(keys[&(oi, si as u32)]).or_default().push((oi, si as u32));
        }
    }
    for members in groups.values() {
        if members.len() < 2 {
            continue;
        }
        let mut sorted = members.clone();
        sorted.sort();
        let winner = sorted[0];
        let mut align = 1u64;
        for &(oi, si) in sorted.iter() {
            align = align.max(parsed[oi].sections[si as usize].align.max(1));
        }
        g.icf_align.insert(winner, align);
        for &loser in sorted.iter().skip(1) {
            g.icf_redirect.insert(loser, winner);
            g.kept[loser.0][loser.1 as usize] = false;
        }
    }
    Ok(())
}

pub fn mapped_target_pub(g: &Graph, t: &TargetSym) -> (usize, u32) {
    mapped_target(g, t)
}

fn mapped_target(g: &Graph, t: &TargetSym) -> (usize, u32) {
    let mut id = (t.obj, t.sec);
    while let Some(&w) = g.icf_redirect.get(&id) {
        id = w;
    }
    id
}

fn is_got_kind(kind: u32) -> bool {
    matches!(kind, 9 | 41 | 42)
}

pub fn plan_slots(
    g: &mut Graph,
    parsed: &[Object],
    db: &SymDb,
    dyn_syms: &BTreeMap<String, DynSym>,
    target: &dyn Target,
    relax: bool,
) -> Result<(), LinkError> {
    let mut ordered: Vec<(usize, u32, usize)> = Vec::new();
    for oi in 0..parsed.len() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if !g.kept[oi][si] || *c == SecClass::Skip {
                continue;
            }
            let mut v = g.rela_idx[oi][si].clone();
            v.sort_unstable();
            for ri in v {
                ordered.push((oi, si as u32, ri));
            }
        }
    }
    use rayon::prelude::*;
    struct SlotOut {
        oi: usize,
        si: u32,
        offset: u64,
        plan: Plan,
        ifunc: bool,
        dynamic: bool,
        key: Option<GotKey>,
        name: Option<String>,
        used: Option<String>,
        tls_want: u64,
        tls_pair: Option<String>,
        tls_missing: bool,
        relax: bool,
    }
    enum SlotItem {
        Dead,
        Ready(SlotOut),
        Failed { oi: usize, si: u32, offset: u64, err: LinkError },
    }
    let computed: Vec<SlotItem> = ordered
        .par_iter()
        .map(|&(oi, si, ri)| {
            let obj = &parsed[oi];
            let r = &obj.relas[ri];
            if g.eh.dead_rel.contains(&(oi, si, r.offset)) {
                return SlotItem::Dead;
            }
            let owned;
            let t = match g.resolved[oi].get(r.sym as usize).and_then(|o| o.as_ref()) {
                Some(t) => t,
                None => {
                    match resolve_sym(oi, &g.objs[oi].label, obj, &g.objs[oi].local_target, r.sym as usize, db, dyn_syms) {
                        Ok(ts) => {
                            owned = ts;
                            &owned
                        }
                        Err(err) => return SlotItem::Failed { oi, si, offset: r.offset, err },
                    }
                }
            };
            let plan = match target.plan(r.kind, sym_class(t)).map_err(|e| {
                LinkError::Native(format!("{}: section `{}`: {e}", g.objs[oi].label, obj.section_name(si as usize).unwrap_or("?")))
            }) {
                Ok(plan) => plan,
                Err(err) => return SlotItem::Failed { oi, si, offset: r.offset, err },
            };
            let mut out = SlotOut {
                oi,
                si,
                offset: r.offset,
                plan,
                ifunc: t.ifunc,
                dynamic: t.dynamic,
                key: None,
                name: None,
                used: if (t.dynamic || t.weak_zero) && !t.name.is_empty() {
                    Some(t.name.clone())
                } else {
                    None
                },
                tls_want: 0,
                tls_pair: None,
                tls_missing: false,
                relax: false,
            };
            if relax
                && (r.kind == R_GOTPCRELX || r.kind == R_REX_GOTPCRELX)
                && !t.weak_zero
                && !t.dynamic
                && !t.tls
                && !t.ifunc
                && !t.common
                && t.obj != usize::MAX
                && t.sec != u32::MAX
                && obj
                    .section_bytes(si as usize)
                    .ok()
                    .is_some_and(|b| crate::target::x86_64::gotpcrelx_form(b, r.offset as usize) != 0)
            {
                out.plan = Plan::Direct;
                out.relax = true;
            }
            match out.plan {
                Plan::Skip | Plan::Direct | Plan::TpoffRelax => {}
                Plan::Got => {
                    out.key = Some(got_key(g, t));
                }
                Plan::Plt => {
                    if is_got_kind(r.kind) {
                        out.key = Some(got_key(g, t));
                    } else {
                        out.name = Some(t.name.clone());
                    }
                }
                Plan::TlsGd | Plan::TlsLd => {
                    let delta = if matches!(plan, Plan::TlsGd) { 8u64 } else { 5u64 };
                    out.tls_want = r.offset + delta;
                    match obj.relas.iter().find(|q| q.section == si && q.offset == out.tls_want && q.kind == 4) {
                        Some(q) => out.tls_pair = Some(obj.symbol_name(q.sym as usize).unwrap_or("?").to_string()),
                        None => out.tls_missing = true,
                    }
                }
                Plan::IFuncGot => {
                    out.key = Some(got_key(g, t));
                }
                Plan::IFuncPlt => {
                    out.key = Some(got_key(g, t));
                    out.name = Some(t.name.clone());
                }
            }
            SlotItem::Ready(out)
        })
        .collect();
    for item in computed {
        let s = match item {
            SlotItem::Dead => continue,
            SlotItem::Ready(s) => s,
            SlotItem::Failed { oi, si, offset, err } => {
                if g.consumed_call.contains(&(oi, si, offset)) {
                    continue;
                }
                if g.eh.dead_rel.contains(&(oi, si, offset)) {
                    continue;
                }
                return Err(err);
            }
        };
        if g.consumed_call.contains(&(s.oi, s.si, s.offset)) {
            continue;
        }
        if g.eh.dead_rel.contains(&(s.oi, s.si, s.offset)) {
            continue;
        }
        if s.relax {
            g.relaxed.insert((s.oi, s.si, s.offset));
        }
        if let Some(nm) = s.used {
            if !g.used_names.contains(nm.as_str()) {
                g.used_names.insert(nm.clone());
            }
            if s.dynamic {
                if let Some(ds) = dyn_syms.get(nm.as_str()) {
                    g.needed_libs.insert(ds.lib.clone());
                }
            }
        }
        match s.plan {
            Plan::Skip | Plan::Direct | Plan::TpoffRelax => {}
            Plan::Got => {
                intern_got(g, s.key.unwrap(), false, s.dynamic);
            }
            Plan::Plt => {
                if let Some(key) = s.key {
                    intern_got(g, key, s.ifunc, s.dynamic);
                } else {
                    intern_plt(g, &s.name.unwrap(), s.ifunc);
                }
            }
            Plan::TlsGd | Plan::TlsLd => {
                if s.tls_missing {
                    return Err(LinkError::Native(format!(
                        "{}: TLS relocation without a paired call to `__tls_get_addr`",
                        g.objs[s.oi].label
                    )));
                }
                let qname = s.tls_pair.unwrap();
                g.used_names.insert(qname.clone());
                if qname != "__tls_get_addr" {
                    return Err(LinkError::Native(format!(
                        "{}: TLS sequence calls `{qname}` instead of `__tls_get_addr`",
                        g.objs[s.oi].label
                    )));
                }
                g.consumed_call.insert((s.oi, s.si, s.tls_want));
            }
            Plan::IFuncGot => {
                intern_got(g, s.key.unwrap(), true, false);
            }
            Plan::IFuncPlt => {
                let gi = intern_got(g, s.key.unwrap(), true, false);
                let pi = intern_plt(g, &s.name.unwrap(), true);
                g.plt_got.insert(pi, gi);
            }
        }
    }
    Ok(())
}


pub fn got_key(g: &Graph, t: &TargetSym) -> GotKey {
    if t.name.is_empty() {
        let id = mapped_target(g, t);
        GotKey::Local(id.0, id.1, t.value)
    } else {
        GotKey::Named(t.name.clone())
    }
}

fn intern_got(g: &mut Graph, key: GotKey, ifunc: bool, dynamic: bool) -> usize {
    if let Some(&idx) = g.got_index.get(&key) {
        if ifunc {
            g.got[idx].ifunc = true;
        }
        if dynamic {
            g.got[idx].dynamic = true;
        }
        return idx;
    }
    let idx = g.got.len();
    g.got_index.insert(key.clone(), idx);
    g.got.push(GotSlot { key, ifunc, dynamic });
    idx
}

fn intern_plt(g: &mut Graph, name: &str, ifunc: bool) -> usize {
    if let Some(&idx) = g.plt_index.get(name) {
        if ifunc {
            g.plt[idx].ifunc = true;
        }
        return idx;
    }
    let idx = g.plt.len();
    g.plt_index.insert(name.to_string(), idx);
    g.plt.push(PltSlot { name: name.to_string(), ifunc });
    idx
}

pub fn collect_tls(g: &mut Graph, parsed: &[Object]) {
    let mut tdata: Vec<(usize, u32)> = Vec::new();
    let mut tbss: Vec<(usize, u32)> = Vec::new();
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if !g.kept[oi][si] {
                continue;
            }
            match c {
                SecClass::Tdata => tdata.push((oi, si as u32)),
                SecClass::Tbss => tbss.push((oi, si as u32)),
                _ => {}
            }
            let _ = obj;
        }
    }
    let mut align = 1u64;
    for &(oi, si) in tdata.iter().chain(tbss.iter()) {
        align = align.max(parsed[oi].sections[si as usize].align.max(1));
    }
    let mut off = 0u64;
    let mut dtpoff = BTreeMap::new();
    for &(oi, si) in tdata.iter().chain(tbss.iter()) {
        let sec = &parsed[oi].sections[si as usize];
        let a = sec.align.max(1);
        off = off.next_multiple_of(a);
        dtpoff.insert((oi, si), off);
        off += sec.size;
    }
    let mut tdata_end = 0u64;
    for &(oi, si) in tdata.iter() {
        tdata_end = tdata_end.max(dtpoff[&(oi, si)] + parsed[oi].sections[si as usize].size);
    }
    let size = off.next_multiple_of(align.max(1));
    let mut order = tdata;
    order.extend(tbss);
    g.tls = TlsInfo { order, dtpoff, size, align: align.max(1), tdata_end };
}

pub fn collect_commons(g: &mut Graph, db: &SymDb) {
    for (name, e) in db.entries() {
        if e.common_size == 0 {
            continue;
        }
        if e.def.as_ref().is_some_and(|d| d.bind == STB_GLOBAL) {
            continue;
        }
        g.commons.push((name.clone(), e.common_size, e.common_align.max(1)));
    }
}

fn eh_reloc_in(sec_relas: &[usize], obj: &Object, off: usize, size: usize) -> Option<usize> {
    let pos = sec_relas.partition_point(|&i| (obj.relas[i].offset as usize) < off);
    sec_relas.get(pos).copied().filter(|&i| (obj.relas[i].offset as usize) < off + size)
}

pub fn compute_eh_plan(g: &mut Graph, parsed: &[Object]) -> Result<(), LinkError> {
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if *c != SecClass::ExceptTable || g.kept[oi][si] {
                continue;
            }
            let name = obj.section_name(si).unwrap_or("");
            let suffix = name.strip_prefix(".gcc_except_table").unwrap_or("");
            let wanted = if suffix.is_empty() {
                g.objs[oi].classes.iter().zip(g.kept[oi].iter()).any(|(&cc, &k)| k && cc == SecClass::Text)
            } else {
                let a = format!(".text{suffix}");
                let b = format!(".text.unlikely{suffix}");
                obj.sections.iter().enumerate().any(|(ti, _)| {
                    g.kept[oi].get(ti).copied().unwrap_or(false)
                        && obj.section_name(ti).is_ok_and(|n| n == a || n == b)
                })
            };
            if wanted {
                g.kept[oi][si] = true;
            }
        }
    }
    let mut plan = EhPlan { cies: Vec::new(), cie_index: BTreeMap::new(), sections: BTreeMap::new(), dead_rel: BTreeSet::new(), total: 0 };
    let mut keep_extra: Vec<(usize, u32)> = Vec::new();
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if *c != SecClass::EhFrame || !g.kept[oi][si] {
                continue;
            }
            let bytes = obj.section_bytes(si)?;
            let sec_relas = &g.rela_idx[oi][si];
            let mut sec = EhSecPlan { fdes: Vec::new(), out_off: 0, out_size: 0 };
            for f in walk_eh_frame(bytes) {
                let alive = match eh_reloc_in(sec_relas, obj, f.pc_off, f.pc_size) {
                    Some(i) => {
                        let r = &obj.relas[i];
                        match g.resolved[oi].get(r.sym as usize).and_then(|o| o.as_ref()) {
                            Some(t) => {
                                if t.dynamic || t.weak_zero || t.common || t.obj == usize::MAX || t.sec == u32::MAX {
                                    true
                                } else {
                                    let id = mapped_target(g, &t);
                                    g.kept.get(id.0).is_some_and(|k| k.get(id.1 as usize).copied().unwrap_or(false))
                                        && !g.icf_redirect.contains_key(&(t.obj, t.sec))
                                }
                            }
                            None => false,
                        }
                    }
                    None => true,
                };
                if !alive {
                    continue;
                }
                let key = cie_masked_key(bytes, f.cie_start, f.cie_end, obj, &g.resolved[oi], sec_relas);
                let cie_idx = match plan.cie_index.get(&key) {
                    Some(&i) => i,
                    None => {
                        let i = plan.cies.len();
                        let total = (f.cie_end - f.cie_start) as u64;
                        plan.cies.push(CieEntry {
                            body: bytes[f.cie_start..f.cie_end].to_vec(),
                            total,
                            live: false,
                            first: (oi, si as u32, f.cie_start),
                        });
                        plan.cie_index.insert(key, i);
                        i
                    }
                };
                plan.cies[cie_idx].live = true;
                if f.cie.has_lsda {
                    for sym_idx in lsda_relocs(obj, &f, bytes, sec_relas) {
                        if let Some(t) = g.resolved[oi].get(sym_idx).and_then(|o| o.as_ref()) {
                            if !t.dynamic && !t.weak_zero && !t.common && t.obj != usize::MAX && t.sec != u32::MAX {
                                let id = mapped_target(g, &t);
                                if id.0 < parsed.len() {
                                    keep_extra.push(id);
                                }
                            }
                        }
                    }
                }
                sec.fdes.push(FdeOut { start: f.start, end: f.end, cie: cie_idx, out_off: 0 });
            }
            plan.sections.insert((oi, si as u32), sec);
        }
    }
    for (oi, si) in keep_extra {
        if (si as usize) < g.kept[oi].len() && !g.kept[oi][si as usize] && g.objs[oi].classes[si as usize] != SecClass::Skip {
            g.kept[oi][si as usize] = true;
        }
    }
    let mut cie_off: Vec<u64> = Vec::with_capacity(plan.cies.len());
    let mut out_off = 0u64;
    for cie in plan.cies.iter() {
        if cie.live {
            cie_off.push(out_off);
            out_off += cie.total;
        } else {
            cie_off.push(u64::MAX);
        }
    }
    g.eh_cie_off = cie_off;
    let mut sec_ids: Vec<(usize, u32)> = plan.sections.keys().copied().collect();
    sec_ids.sort();
    for id in sec_ids {
        let sec = plan.sections.get_mut(&id).unwrap();
        sec.out_off = out_off;
        let mut local = 0u64;
        for f in sec.fdes.iter_mut() {
            f.out_off = local;
            local += (f.end - f.start) as u64;
        }
        sec.out_size = local;
        g.merged_off.insert(id, out_off);
        out_off += local;
    }
    out_off += 4;
    plan.total = out_off;
    let mut dead_rel: BTreeSet<(usize, u32, u64)> = BTreeSet::new();
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if *c != SecClass::EhFrame || !g.kept[oi][si] {
                continue;
            }
            let sec = match plan.sections.get(&(oi, si as u32)) {
                Some(s) => s,
                None => continue,
            };
            let mut cies: Vec<(usize, usize)> = Vec::new();
            for cie in plan.cies.iter() {
                if cie.live && cie.first.0 == oi && cie.first.1 == si as u32 {
                    cies.push((cie.first.2, cie.first.2 + cie.body.len()));
                }
            }
            cies.sort();
            for &i in g.rela_idx[oi][si].iter() {
                let r = &obj.relas[i];
                let o = r.offset as usize;
                let pos = sec.fdes.partition_point(|f| f.start <= o);
                if pos > 0 && o < sec.fdes[pos - 1].end {
                    continue;
                }
                let cpos = cies.partition_point(|&(s, _)| s <= o);
                if cpos > 0 && o < cies[cpos - 1].1 {
                    continue;
                }
                dead_rel.insert((oi, si as u32, r.offset));
            }
        }
    }
    plan.dead_rel = dead_rel;
    g.eh = plan;
    Ok(())
}

pub fn lsda_field(f: &FdeSpan, bytes: &[u8]) -> Option<(usize, u8)> {
    if f.aug_len == 0 {
        return None;
    }
    let mut p = f.aug_off;
    let mut shift = 0u32;
    while p < bytes.len() && p < f.aug_off + f.aug_len {
        let b = bytes[p];
        p += 1;
        shift += 7;
        if b & 0x80 == 0 {
            break;
        }
        if shift > 63 {
            return None;
        }
    }
    Some((p, f.cie.lsda_enc))
}

fn lsda_relocs(obj: &Object, f: &FdeSpan, bytes: &[u8], sec_relas: &[usize]) -> Vec<usize> {
    let mut out = Vec::new();
    if f.aug_len == 0 {
        return out;
    }
    let mut p = f.aug_off;
    let mut shift = 0u32;
    while p < bytes.len() && p < f.aug_off + f.aug_len {
        let b = bytes[p];
        p += 1;
        shift += 7;
        if b & 0x80 == 0 {
            break;
        }
        if shift > 63 {
            return out;
        }
    }
    let payload = p;
    let enc = f.cie.lsda_enc;
    let indirect = enc & 0x80 != 0;
    let base = enc & 0x70;
    let size = enc_size(enc);
    if size == 0 {
        return out;
    }
    if !indirect && base == 0x00 {
        if let Some(&i) = sec_relas.iter().find(|&&i| obj.relas[i].offset as usize == payload) {
            out.push(obj.relas[i].sym as usize);
        }
    } else if base == 0x10 {
        if payload + size <= bytes.len() {
            let mut v = 0u64;
            for i in 0..size {
                v |= (bytes[payload + i] as u64) << (8 * i);
            }
            if size < 8 && bytes[payload + size - 1] & 0x80 != 0 {
                v |= !0u64 << (8 * size);
            }
            let cell = payload as i64 + v as i64;
            if cell >= 0 {
                let cell = cell as usize;
                if let Some(&i) = sec_relas.iter().find(|&&i| {
                    let o = obj.relas[i].offset as usize;
                    o >= cell && o < cell + 8
                }) {
                    out.push(obj.relas[i].sym as usize);
                }
            }
        }
    }
    out
}

