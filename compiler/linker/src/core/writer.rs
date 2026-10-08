use crate::core::graph::{Graph, SecClass, TargetSym};
use crate::core::layout::{outsec_class, Layout, OutSec, SegClass, SynthSec, PAGE};
use crate::core::obj::{
    Object, SHN_ABS, SHN_COMMON, SHN_UNDEF, SHT_NOBITS, STB_GLOBAL, STB_WEAK, STT_FILE, STT_SECTION, STT_TLS,
};
use crate::core::string::StrTab;
use crate::target::{elf, ApplyCtx, Plan, Target};
use crate::LinkError;
use std::collections::BTreeMap;

pub struct DynInfo {
    pub needed: Vec<String>,
    pub dynsym_names: Vec<String>,
    pub dynsym_bind: Vec<u8>,
    pub dynsym_kind: Vec<u8>,
    pub dynsym_name_off: Vec<u32>,
    pub dynstr_bytes: Vec<u8>,
    pub hash_bytes: Vec<u8>,
    pub versym: Vec<u16>,
    pub verneed: Vec<u8>,
    pub verneed_num: usize,
    pub plt_rel: Vec<(u64, u32, u32, i64)>,
    pub dyn_rel: Vec<(u64, u32, u32, i64)>,
    pub interp: Option<String>,
    pub runpath: Option<String>,
    pub init_addr: u64,
    pub fini_addr: u64,
    pub force_dynamic: bool,
    pub relacount: u64,
    pub osabi: u8,
    pub build_id: Option<[u8; 20]>,
}

impl DynInfo {
    pub fn empty() -> DynInfo {
        DynInfo {
            dynsym_name_off: Vec::new(),
            dynstr_bytes: Vec::new(),
            hash_bytes: Vec::new(),
            needed: Vec::new(),
            dynsym_names: Vec::new(),
            dynsym_bind: Vec::new(),
            dynsym_kind: Vec::new(),
            versym: Vec::new(),
            verneed: Vec::new(),
            verneed_num: 0,
            plt_rel: Vec::new(),
            dyn_rel: Vec::new(),
            interp: None,
            runpath: None,
            init_addr: 0,
            fini_addr: 0,
            force_dynamic: false,
            relacount: 0,
            osabi: 0,
            build_id: None,
        }
    }
}

pub struct SymAddr {
    pub value: u64,
    pub shdr: u32,
    pub kind: u8,
    pub bind: u8,
    pub vis: u8,
    pub size: u64,
}

fn merged_base_of(layout: &Layout, class: SecClass) -> (SynthSec, u64) {
    let synth = crate::core::layout::merged_synth_of(class);
    let base = layout.order.iter().find(|o| o.synth == synth).map(|o| o.addr).unwrap_or(0);
    (synth, base)
}

fn is_merged_class(class: SecClass) -> bool {
    crate::core::layout::is_merged(class)
}

fn def_value(
    g: &Graph,
    layout: &Layout,
    db: &crate::core::symdb::SymDb,
    name: &str,
) -> Option<(u64, u8, u8, u8, u64)> {
    let e = db.get(name)?;
    let d = e.def.as_ref()?;
    if d.obj == usize::MAX {
        let v = layout.synth_addr.get(name).copied().unwrap_or(0);
        let (bind, vis) = synth_bind_vis(name);
        return Some((v, d.kind, bind, vis, d.size));
    }
    if d.absolute {
        return Some((d.value, d.kind, d.bind, d.vis, d.size));
    }
    let mut id = (d.obj, d.sec);
    while let Some(&w) = g.icf_redirect.get(&id) {
        id = w;
    }
    if !g.kept.get(id.0).is_some_and(|k| k.get(id.1 as usize).copied().unwrap_or(false)) {
        return None;
    }
    let class = g.objs[id.0].classes[id.1 as usize];
    if is_merged_class(class) {
        let (_, base) = merged_base_of(layout, class);
        let addr = base + g.merged_off.get(&id).copied().unwrap_or(0) + d.value;
        let value = if d.tls { g.tls.dtpoff.get(&id).copied().unwrap_or(0) + d.value } else { addr };
        return Some((value, d.kind, d.bind, d.vis, d.size));
    }
    let addr = layout.sec_addr.get(&id).copied().unwrap_or(0) + d.value;
    let value = if d.tls { g.tls.dtpoff.get(&id).copied().unwrap_or(0) + d.value } else { addr };
    Some((value, d.kind, d.bind, d.vis, d.size))
}

fn synth_bind_vis(name: &str) -> (u8, u8) {
    match name {
        "_end" | "__end" | "end" | "__bss_start" | "etext" | "__etext" | "edata" | "__edata" => (STB_GLOBAL, 0),
        _ => (0, 2),
    }
}

pub(crate) fn target_address(
    g: &Graph,
    layout: &Layout,
    t: &TargetSym,
) -> Result<u64, LinkError> {
    if t.weak_zero || t.dynamic {
        return Ok(0);
    }
    if t.common {
        return Ok(g.common_addr.get(&t.name).copied().unwrap_or(0) + t.value);
    }
    if t.obj == usize::MAX {
        return Ok(layout.synth_addr.get(&t.name).copied().unwrap_or(0) + t.value);
    }
    if t.sec == u32::MAX {
        return Ok(t.value);
    }
    let mut id = (t.obj, t.sec);
    while let Some(&w) = g.icf_redirect.get(&id) {
        id = w;
    }
    let class = g.objs[id.0].classes[id.1 as usize];
    if is_merged_class(class) {
        let (_, base) = merged_base_of(layout, class);
        let off = g.merged_off.get(&id).copied().ok_or_else(|| {
            LinkError::Native(format!("reference to unplaced merged section in `{}`", t.name))
        })?;
        return Ok(base + off + t.value);
    }
    layout.sec_addr.get(&id).copied().map(|a| a + t.value).ok_or_else(|| {
        LinkError::Native(format!("reference to discarded section in `{}`", t.name))
    })
}

fn tls_values(g: &Graph, t: &TargetSym) -> (i64, u64) {
    if t.obj == usize::MAX || t.weak_zero {
        return (0, 0);
    }
    let mut id = (t.obj, t.sec);
    while let Some(&w) = g.icf_redirect.get(&id) {
        id = w;
    }
    let dtpoff = g.tls.dtpoff.get(&id).copied().unwrap_or(0) + t.value;
    let tpoff = dtpoff as i64 - g.tls.size as i64;
    (tpoff, dtpoff)
}

pub fn write_output(
    parsed: &[Object],
    g: &Graph,
    db: &crate::core::symdb::SymDb,
    layout: &Layout,
    target: &dyn Target,
    dyn_info: &DynInfo,
    is_exec: bool,
    symdata: &SymtabData,
    strip: bool,
    out: &mut [u8],
) -> Result<ShdrTable, LinkError> {
    for o in layout.order.iter() {
        let dest = out.get_mut(o.offset as usize..(o.offset + o.size) as usize).ok_or_else(|| LinkError::Native("output overflow".to_string()))?;
        match o.synth {
            SynthSec::None => {
                if let Some(key) = o.merge {
                    if key.kind == SHT_NOBITS {
                        continue;
                    }
                    let grp = layout.merged.get(&key).ok_or_else(|| LinkError::Native("merged group lost".to_string()))?;
                    for &(oi, si) in grp.members.iter() {
                        let bytes = parsed[oi].section_bytes(si as usize)?;
                        let at = g.merged_off.get(&(oi, si)).copied().unwrap_or(0) as usize;
                        dest.get_mut(at..at + bytes.len())
                            .ok_or_else(|| LinkError::Native("merged copy overflow".to_string()))?
                            .copy_from_slice(bytes);
                    }
                    continue;
                }
                let obj = &parsed[o.obj];
                let class = g.objs[o.obj].classes[o.sec as usize];
                if matches!(class, SecClass::InitArray | SecClass::FiniArray | SecClass::Tdata | SecClass::Tbss | SecClass::EhFrame) {
                    continue;
                }
                let bytes = obj.section_bytes(o.sec as usize)?;
                if obj.sections[o.sec as usize].kind == SHT_NOBITS {
                    continue;
                } else {
                    if bytes.len() != dest.len() {
                        return Err(LinkError::Native("section size mismatch".to_string()));
                    }
                    dest.copy_from_slice(bytes);
                }
            }
            SynthSec::MergedTbss => {
                dest.fill(0);
            }
            SynthSec::MergedTdata => {
                for &(oi, si) in g.tls.order.iter() {
                    if g.objs[oi].classes[si as usize] != SecClass::Tdata {
                        continue;
                    }
                    let bytes = parsed[oi].section_bytes(si as usize)?;
                    let at = g.tls.dtpoff[&(oi, si)] as usize;
                    dest.get_mut(at..at + bytes.len()).ok_or_else(|| LinkError::Native("tls copy overflow".to_string()))?.copy_from_slice(bytes);
                }
            }
            SynthSec::MergedInitArray | SynthSec::MergedFiniArray | SynthSec::MergedInit | SynthSec::MergedFini => {
                let want = match o.synth {
                    SynthSec::MergedInitArray => SecClass::InitArray,
                    SynthSec::MergedFiniArray => SecClass::FiniArray,
                    SynthSec::MergedInit => SecClass::Init,
                    _ => SecClass::Fini,
                };
                for (oi, obj) in parsed.iter().enumerate() {
                    for (si, c) in g.objs[oi].classes.iter().enumerate() {
                        if *c != want || !g.kept[oi][si] {
                            continue;
                        }
                        let bytes = obj.section_bytes(si)?;
                        let at = g.merged_off[&(oi, si as u32)] as usize;
                        dest.get_mut(at..at + bytes.len()).ok_or_else(|| LinkError::Native("init_array copy overflow".to_string()))?.copy_from_slice(bytes);
                    }
                }
            }
            SynthSec::MergedEh => {
                emit_eh_frame(parsed, g, layout, target, dest, &g.rela_idx)?;
            }
            SynthSec::Got => {
                for (i, slot) in g.got.iter().enumerate() {
                    let v = got_slot_value(g, layout, db, slot)?;
                    dest[i * 8..i * 8 + 8].copy_from_slice(&v.to_le_bytes());
                }
            }
            SynthSec::GotPlt => {
                dest.fill(0);
                if layout.got_plt_addr != 0 && layout.plt_addr != 0 {
                    let mut n = 0u32;
                    for slot in g.plt.iter() {
                        if slot.ifunc {
                            continue;
                        }
                        let push_addr = layout.plt_addr + 16 + n as u64 * 16 + 6;
                        let at = (3 + n) as usize * 8;
                        if at + 8 <= dest.len() {
                            dest[at..at + 8].copy_from_slice(&push_addr.to_le_bytes());
                        }
                        n += 1;
                    }
                }
            }
            SynthSec::Plt => {
                emit_plt(g, layout, dest, false, target)?;
            }
            SynthSec::IPlt => {
                emit_plt(g, layout, dest, true, target)?;
            }
            SynthSec::RelaDyn | SynthSec::RelaPlt => {
                let rels = if o.synth == SynthSec::RelaDyn { &dyn_info.dyn_rel } else { &dyn_info.plt_rel };
                for (i, r) in rels.iter().enumerate() {
                    elf::write_rela(&mut dest[i * 24..i * 24 + 24], r.0, r.1, r.2, r.3);
                }
            }
            SynthSec::DynSym => {
                for (i, _name) in dyn_info.dynsym_names.iter().enumerate() {
                    elf::write_sym(
                        &mut dest[i * 24..i * 24 + 24],
                        dyn_info.dynsym_name_off[i],
                        (dyn_info.dynsym_bind[i] << 4) | dyn_info.dynsym_kind[i],
                        0,
                        0,
                        0,
                        0,
                    );
                }
            }
            SynthSec::DynStr => {
                dest[..dyn_info.dynstr_bytes.len()].copy_from_slice(&dyn_info.dynstr_bytes);
            }
            SynthSec::Hash => {
                dest[..dyn_info.hash_bytes.len()].copy_from_slice(&dyn_info.hash_bytes);
            }
            SynthSec::GnuVersion => {
                for (i, v) in dyn_info.versym.iter().enumerate() {
                    dest[i * 2..i * 2 + 2].copy_from_slice(&v.to_le_bytes());
                }
            }
            SynthSec::GnuVersionR => {
                dest[..dyn_info.verneed.len()].copy_from_slice(&dyn_info.verneed);
            }
            SynthSec::Dynamic => {
                emit_dynamic(layout, dyn_info, dest)?;
            }
            SynthSec::Interp => {
                let path = dyn_info.interp.as_deref().unwrap_or("");
                let b = path.as_bytes();
                dest[..b.len()].copy_from_slice(b);
            }
            SynthSec::BuildId => {
                let id = dyn_info.build_id.unwrap_or([0u8; 20]);
                dest[0..4].copy_from_slice(&4u32.to_le_bytes());
                dest[4..8].copy_from_slice(&20u32.to_le_bytes());
                dest[8..12].copy_from_slice(&3u32.to_le_bytes());
                dest[12..16].copy_from_slice(b"GNU\0");
                dest[16..36].copy_from_slice(&id);
            }
        }
    }
    apply_relocations(parsed, g, layout, target, out, &g.rela_idx)?;
    let table = emit_tables(g, layout, dyn_info, is_exec, symdata, strip, out, target)?;
    Ok(table)
}

pub struct ShdrEntry {
    pub name: u32,
    pub kind: u32,
    pub flags: u64,
    pub addr: u64,
    pub offset: u64,
    pub size: u64,
    pub link: u32,
    pub info: u32,
    pub align: u64,
    pub entsize: u64,
}

pub struct ShdrTable {
    pub entries: Vec<ShdrEntry>,
    pub shoff: u64,
}

pub(crate) fn got_slot_value(
    g: &Graph,
    layout: &Layout,
    db: &crate::core::symdb::SymDb,
    slot: &crate::core::graph::GotSlot,
) -> Result<u64, LinkError> {
    if slot.dynamic || slot.ifunc {
        return Ok(0);
    }
    match &slot.key {
        crate::core::graph::GotKey::Named(name) => {
            Ok(def_value(g, layout, db, name).map(|v| v.0).unwrap_or(0))
        }
        crate::core::graph::GotKey::Local(oi, sec, val) => {
            let class = g.objs[*oi].classes[*sec as usize];
            if matches!(class, SecClass::InitArray | SecClass::FiniArray | SecClass::Tdata | SecClass::Tbss | SecClass::EhFrame) {
                let base_synth = match class {
                    SecClass::InitArray => SynthSec::MergedInitArray,
                    SecClass::FiniArray => SynthSec::MergedFiniArray,
                    SecClass::Tdata => SynthSec::MergedTdata,
                    SecClass::Tbss => SynthSec::MergedTbss,
                    _ => SynthSec::MergedEh,
                };
                let base = layout.order.iter().find(|o| o.synth == base_synth).map(|o| o.addr).unwrap_or(0);
                Ok(base + g.merged_off.get(&(*oi, *sec)).copied().unwrap_or(0) + val)
            } else {
                Ok(layout.sec_addr.get(&(*oi, *sec)).copied().unwrap_or(0) + val)
            }
        }
    }
}

fn emit_plt(g: &Graph, layout: &Layout, dest: &mut [u8], iplt: bool, target: &dyn Target) -> Result<(), LinkError> {
    if iplt {
        let mut n = 0usize;
        for (pi, slot) in g.plt.iter().enumerate() {
            if !slot.ifunc {
                continue;
            }
            let gi = g.plt_got.get(&pi).copied().ok_or_else(|| LinkError::Native("ifunc plt without got slot".to_string()))?;
            let entry_addr = layout.iplt_addr + n as u64 * 16;
            let slot_addr = layout.got_addr + gi as u64 * 8;
            dest[n * 16..n * 16 + 16].copy_from_slice(&target.iplt_entry(entry_addr, slot_addr));
            n += 1;
        }
        return Ok(());
    }
    let plt0 = layout.plt_addr;
    let got_plt = layout.got_plt_addr;
    {
        let mut p0 = [0u8; 16];
        p0[0] = 0xff;
        p0[1] = 0x35;
        p0[2..6].copy_from_slice(&((got_plt as i64 + 8 - (plt0 as i64 + 6)) as i32).to_le_bytes());
        p0[6] = 0xff;
        p0[7] = 0x25;
        p0[8..12].copy_from_slice(&((got_plt as i64 + 16 - (plt0 as i64 + 12)) as i32).to_le_bytes());
        p0[12..16].copy_from_slice(&[0x0f, 0x1f, 0x40, 0x00]);
        dest.get_mut(0..16).ok_or_else(|| LinkError::Native("plt overflow".to_string()))?.copy_from_slice(&p0);
    }
    let mut n = 0u32;
    for slot in g.plt.iter() {
        if slot.ifunc {
            continue;
        }
        let entry_addr = plt0 + (n as u64 + 1) * 16;
        let slot_addr = got_plt + (n as u64 + 3) * 8;
        dest.get_mut((n as usize + 1) * 16..(n as usize + 2) * 16)
            .ok_or_else(|| LinkError::Native("plt overflow".to_string()))?
            .copy_from_slice(&target.plt_entry(entry_addr, slot_addr, n, plt0));
        n += 1;
    }
    Ok(())
}

use crate::core::graph::{got_key, lsda_field, sym_class, walk_eh_frame};


fn emit_eh_frame(
    parsed: &[Object],
    g: &Graph,
    layout: &Layout,
    target: &dyn Target,
    dest: &mut [u8],
    ridx: &[Vec<Vec<usize>>],
) -> Result<(), LinkError> {
    let eh_base = layout.order.iter().find(|o| o.synth == SynthSec::MergedEh).map(|o| o.addr).unwrap_or(0);
    for (ci, cie) in g.eh.cies.iter().enumerate() {
        if !cie.live {
            continue;
        }
        let at = g.eh_cie_off[ci] as usize;
        let slot = dest.get_mut(at..at + cie.body.len()).ok_or_else(|| LinkError::Native("eh_frame overflow".to_string()))?;
        slot.copy_from_slice(&cie.body);
        let (foi, fsi, fstart) = cie.first;
        let fobj = &parsed[foi];
        let fend = fstart + cie.body.len();
        let relas = &ridx[foi][fsi as usize];
        let lo = relas.partition_point(|&i| (fobj.relas[i].offset as usize) < fstart);
        let hi = relas.partition_point(|&i| (fobj.relas[i].offset as usize) < fend);
        for &i in relas[lo..hi].iter() {
            let r = &fobj.relas[i];
            if g.eh.dead_rel.contains(&(foi, fsi, r.offset)) {
                continue;
            }
            let t = g.resolved[foi].get(r.sym as usize).and_then(|o| o.as_ref()).ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
            let s = target_address(g, layout, &t)?;
            let (tpoff, dtpoff) = tls_values(g, &t);
            let place = eh_base + g.eh_cie_off[ci] + (r.offset as usize - fstart) as u64;
            if g.relaxed.contains(&(foi, fsi, r.offset)) {
                crate::target::x86_64::apply_relaxed(slot, r.offset as usize - fstart, s, place, r.addend)?;
                continue;
            }
            let class = sym_class(&t);
            let plan = target.plan(r.kind, class)?;
            let slot_addr = got_or_plt_addr(g, layout, &t, r.kind, plan);
            let ctx = ApplyCtx { sym_addr: s, place_addr: place, slot_addr, addend: r.addend, tpoff, dtpoff };
            target.apply(r.kind, plan, &ctx, slot, r.offset as usize - fstart)?;
        }
    }
    let mut sec_ids: Vec<(usize, u32)> = g.eh.sections.keys().copied().collect();
    sec_ids.sort();
    for id in sec_ids {
        let (oi, si) = id;
        let obj = &parsed[oi];
        let bytes = obj.section_bytes(si as usize)?;
        let sec = &g.eh.sections[&id];
        let sec_out = g.merged_off[&id];
        let spans = walk_eh_frame(bytes);
        let mut by_start: BTreeMap<usize, usize> = BTreeMap::new();
        for (i, s) in spans.iter().enumerate() {
            by_start.insert(s.start, i);
        }
        let relas = &ridx[oi][si as usize];
        for f in sec.fdes.iter() {
            let in_start = f.start;
            let out_start = (sec_out + f.out_off) as usize;
            let len = f.end - f.start;
            let slot = dest.get_mut(out_start..out_start + len).ok_or_else(|| LinkError::Native("eh_frame overflow".to_string()))?;
            slot.copy_from_slice(&bytes[in_start..in_start + len]);
            let field_out = eh_base + sec_out + f.out_off + 4;
            let cie_out = eh_base + g.eh_cie_off[f.cie];
            let v = field_out.wrapping_sub(cie_out) as u32;
            slot[4..8].copy_from_slice(&v.to_le_bytes());
            let lo = relas.partition_point(|&i| (obj.relas[i].offset as usize) < in_start);
            let hi = relas.partition_point(|&i| (obj.relas[i].offset as usize) < in_start + len);
            for &i in relas[lo..hi].iter() {
                let r = &obj.relas[i];
                if g.eh.dead_rel.contains(&(oi, si, r.offset)) {
                    continue;
                }
                let t = g.resolved[oi].get(r.sym as usize).and_then(|o| o.as_ref()).ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
                let s = target_address(g, layout, &t)?;
                let (tpoff, dtpoff) = tls_values(g, &t);
                let place = field_out + (r.offset as usize - in_start) as u64 - 4;
                let rel_off = r.offset as usize - in_start;
                if g.relaxed.contains(&(oi, si, r.offset)) {
                    crate::target::x86_64::apply_relaxed(slot, rel_off, s, place, r.addend)?;
                    continue;
                }
                let class = sym_class(&t);
                let plan = target.plan(r.kind, class)?;
                let slot_addr = got_or_plt_addr(g, layout, &t, r.kind, plan);
                let ctx = ApplyCtx { sym_addr: s, place_addr: place, slot_addr, addend: r.addend, tpoff, dtpoff };
                target.apply(r.kind, plan, &ctx, slot, rel_off)?;
            }
            let span_idx = *by_start.get(&in_start).ok_or_else(|| LinkError::Native("fde lost".to_string()))?;
            let span = spans.get(span_idx).ok_or_else(|| LinkError::Native("fde lost".to_string()))?;
            if let Some((payload_in, enc)) = lsda_field(span, bytes) {
                let indirect = enc & 0x80 != 0;
                let base = enc & 0x70;
                if indirect && base == 0x10 {
                    let size = match enc & 0x0f {
                        0 => 1,
                        1 => 2,
                        2 => 4,
                        _ => 8,
                    };
                    let mut v = 0u64;
                    for i in 0..size {
                        v |= (bytes[payload_in + i] as u64) << (8 * i);
                    }
                    if size < 8 && bytes[payload_in + size - 1] & 0x80 != 0 {
                        v |= !0u64 << (8 * size);
                    }
                    let cell_in = payload_in as i64 + v as i64;
                    if cell_in >= 0 {
                        let cell_in = cell_in as usize;
                        let cpos = relas.partition_point(|&i| (obj.relas[i].offset as usize) < cell_in);
                        if let Some(&i) = relas[cpos..].iter().find(|&&i| (obj.relas[i].offset as usize) < cell_in + 8) {
                            let r = &obj.relas[i];
                            let t = g.resolved[oi].get(r.sym as usize).and_then(|o| o.as_ref()).ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
                            let cell_out = target_address(g, layout, &t)?;
                            let field_out_addr = eh_base + sec_out + f.out_off + (payload_in - in_start) as u64;
                            let nv = cell_out.wrapping_sub(field_out_addr) as i32;
                            let at = out_start + (payload_in - in_start);
                            dest.get_mut(at..at + size).ok_or_else(|| LinkError::Native("lsda fixup overflow".to_string()))?.copy_from_slice(&nv.to_le_bytes()[..size]);
                        }
                    }
                }
            }
        }
    }
    let term_at = g.eh.total as usize - 4;
    dest.get_mut(term_at..term_at + 4).ok_or_else(|| LinkError::Native("eh_frame overflow".to_string()))?.copy_from_slice(&0u32.to_le_bytes());
    Ok(())
}

fn got_or_plt_addr(g: &Graph, layout: &Layout, t: &crate::core::graph::TargetSym, kind: u32, plan: Plan) -> u64 {
    match plan {
        Plan::Got | Plan::IFuncGot => {
            let key = got_key(g, t);
            g.got_index.get(&key).map(|&i| layout.got_addr + i as u64 * 8).unwrap_or(0)
        }
        Plan::Plt => {
            if matches!(kind, 9 | 41 | 42) {
                let key = got_key(g, t);
                g.got_index.get(&key).map(|&i| layout.got_addr + i as u64 * 8).unwrap_or(0)
            } else {
                plt_entry_addr(g, layout, &t.name, t.ifunc)
            }
        }
        Plan::IFuncPlt => plt_entry_addr(g, layout, &t.name, true),
        _ => 0,
    }
}

fn plt_entry_addr(g: &Graph, layout: &Layout, name: &str, ifunc: bool) -> u64 {
    let pos = g.plt.iter().filter(|s| s.ifunc == ifunc).position(|s| s.name == name).unwrap_or(0) as u64;
    if ifunc {
        layout.iplt_addr + pos * 16
    } else {
        layout.plt_addr + 16 + pos * 16
    }
}

fn apply_relocations(
    parsed: &[Object],
    g: &Graph,
    layout: &Layout,
    target: &dyn Target,
    out: &mut [u8],
    ridx: &[Vec<Vec<usize>>],
) -> Result<(), LinkError> {
    for o in layout.order.iter() {
        let jobs: Vec<(usize, u32, u64, u64)> = match o.synth {
            SynthSec::None => {
                if let Some(key) = o.merge {
                    let grp = layout.merged.get(&key).ok_or_else(|| LinkError::Native("merged group lost".to_string()))?;
                    let mut v = Vec::with_capacity(grp.members.len());
                    for &(oi, si) in grp.members.iter() {
                        let off = g.merged_off.get(&(oi, si)).copied().unwrap_or(0);
                        v.push((oi, si, o.addr + off, o.offset + off));
                    }
                    v
                } else {
                    let class = g.objs[o.obj].classes[o.sec as usize];
                    if matches!(class, SecClass::InitArray | SecClass::FiniArray | SecClass::Tdata | SecClass::Tbss | SecClass::EhFrame) {
                        continue;
                    }
                    vec![(o.obj, o.sec, o.addr, o.offset)]
                }
            }
            SynthSec::MergedInitArray | SynthSec::MergedFiniArray | SynthSec::MergedTdata | SynthSec::MergedInit | SynthSec::MergedFini => {
                let mut v = Vec::new();
                for (oi, obj) in parsed.iter().enumerate() {
                    for (si, c) in g.objs[oi].classes.iter().enumerate() {
                        let want = match o.synth {
                            SynthSec::MergedInitArray => SecClass::InitArray,
                            SynthSec::MergedFiniArray => SecClass::FiniArray,
                            SynthSec::MergedInit => SecClass::Init,
                            SynthSec::MergedFini => SecClass::Fini,
                            _ => SecClass::Tdata,
                        };
                        if *c != want || !g.kept[oi][si] {
                            continue;
                        }
                        let off = g.merged_off[&(oi, si as u32)];
                        v.push((oi, si as u32, o.addr + off, o.offset + off));
                        let _ = obj;
                    }
                }
                v
            }
            _ => continue,
        };
        for (oi, si, base_addr, base_off) in jobs {
            let obj = &parsed[oi];
            let n = obj.sections[si as usize].size as usize;
            let buf = out.get_mut(base_off as usize..base_off as usize + n).ok_or_else(|| LinkError::Native("output overflow".to_string()))?;
            for &ri in ridx[oi][si as usize].iter() {
                let r = &obj.relas[ri];
                if g.consumed_call.contains(&(oi, si, r.offset)) {
                    continue;
                }
                let t = g.resolved[oi].get(r.sym as usize).and_then(|o| o.as_ref()).ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
                let s = target_address(g, layout, &t)?;
                let (tpoff, dtpoff) = tls_values(g, &t);
                let place = base_addr + r.offset;
                if g.relaxed.contains(&(oi, si, r.offset)) {
                    crate::target::x86_64::apply_relaxed(buf, r.offset as usize, s, place, r.addend).map_err(|e| {
                        LinkError::Native(format!("{}: section `{}` offset {:#x}: {e}", g.objs[oi].label, parsed[oi].section_name(si as usize).unwrap_or("?"), r.offset))
                    })?;
                    continue;
                }
                let class = sym_class(&t);
                let plan = target.plan(r.kind, class).map_err(|e| {
                    LinkError::Native(format!("{}: {e}", g.objs[oi].label))
                })?;
                let slot_addr = got_or_plt_addr(g, layout, &t, r.kind, plan);
                let ctx = ApplyCtx { sym_addr: s, place_addr: place, slot_addr, addend: r.addend, tpoff, dtpoff };
                if r.kind == 22 {
                }
                target.apply(r.kind, plan, &ctx, buf, r.offset as usize)?;
            }
        }
    }
    Ok(())
}

pub fn order_rela_dyn(dyn_rel: &mut Vec<(u64, u32, u32, i64)>, rel_relative: u32) -> u64 {
    dyn_rel.sort_by(|a, b| {
        (a.1 != rel_relative)
            .cmp(&(b.1 != rel_relative))
            .then_with(|| a.0.cmp(&b.0))
            .then_with(|| a.2.cmp(&b.2))
            .then_with(|| a.3.cmp(&b.3))
    });
    dyn_rel.iter().take_while(|r| r.1 == rel_relative).count() as u64
}

pub fn dynamic_count(dyn_info: &DynInfo, has_init_array: bool, has_fini_array: bool) -> usize {
    let mut n = dyn_info.needed.len();
    if !dyn_info.plt_rel.is_empty() {
        n += 4;
    }
    if !dyn_info.dyn_rel.is_empty() {
        n += 3;
    }
    if dyn_info.relacount > 0 {
        n += 1;
    }
    n += 5;
    n += 2;
    if !dyn_info.versym.is_empty() {
        n += 3;
    }
    if has_init_array {
        n += 2;
    }
    if has_fini_array {
        n += 2;
    }
    if dyn_info.init_addr != 0 {
        n += 1;
    }
    if dyn_info.fini_addr != 0 {
        n += 1;
    }
    if dyn_info.runpath.is_some() {
        n += 1;
    }
    n + 1
}

fn dyn_addr(layout: &Layout, synth: SynthSec) -> u64 {
    layout.order.iter().find(|o| o.synth == synth).map(|o| o.addr).unwrap_or(0)
}

fn dyn_size(layout: &Layout, synth: SynthSec) -> u64 {
    layout.order.iter().find(|o| o.synth == synth).map(|o| o.size).unwrap_or(0)
}

fn emit_dynamic(layout: &Layout, dyn_info: &DynInfo, dest: &mut [u8]) -> Result<(), LinkError> {
    let str_off = |name: &str| -> Result<u64, LinkError> {
        let mut off = 0usize;
        let raw = dyn_info.dynstr_bytes.as_slice();
        while off < raw.len() {
            let end = raw[off..].iter().position(|&b| b == 0).map(|p| off + p).unwrap_or(raw.len());
            if &raw[off..end] == name.as_bytes() {
                return Ok(off as u64);
            }
            off = end + 1;
        }
        Err(LinkError::Native(format!("dynamic string `{name}` missing")))
    };
    let mut n = 0usize;
    let mut put = |tag: i64, val: u64| -> Result<(), LinkError> {
        let slot = dest.get_mut(n * 16..n * 16 + 16).ok_or_else(|| LinkError::Native("dynamic overflow".to_string()))?;
        elf::write_dyn(slot, tag, val);
        n += 1;
        Ok(())
    };
    for lib in dyn_info.needed.iter() {
        put(elf::DT_NEEDED as i64, str_off(lib)?)?;
    }
    if !dyn_info.plt_rel.is_empty() {
        put(elf::DT_PLTRELSZ as i64, dyn_info.plt_rel.len() as u64 * 24)?;
        put(elf::DT_PLTGOT as i64, dyn_addr(layout, SynthSec::GotPlt))?;
        put(elf::DT_PLTREL as i64, elf::DT_RELA)?;
        put(elf::DT_JMPREL as i64, dyn_addr(layout, SynthSec::RelaPlt))?;
    }
    if !dyn_info.dyn_rel.is_empty() {
        put(elf::DT_RELA as i64, dyn_addr(layout, SynthSec::RelaDyn))?;
        put(elf::DT_RELASZ as i64, dyn_info.dyn_rel.len() as u64 * 24)?;
        put(elf::DT_RELAENT as i64, 24)?;
    }
    if dyn_info.relacount > 0 {
        put(elf::DT_RELACOUNT as i64, dyn_info.relacount)?;
    }
    put(elf::DT_SYMTAB as i64, dyn_addr(layout, SynthSec::DynSym))?;
    put(elf::DT_SYMENT as i64, 24)?;
    put(elf::DT_STRTAB as i64, dyn_addr(layout, SynthSec::DynStr))?;
    put(elf::DT_STRSZ as i64, dyn_size(layout, SynthSec::DynStr))?;
    put(elf::DT_HASH as i64, dyn_addr(layout, SynthSec::Hash))?;
    put(elf::DT_DEBUG as i64, 0)?;
    put(elf::DT_FLAGS_1 as i64, elf::DF_1_PIE)?;
    if !dyn_info.versym.is_empty() {
        put(elf::DT_VERSYM as i64, dyn_addr(layout, SynthSec::GnuVersion))?;
        put(elf::DT_VERNEED as i64, dyn_addr(layout, SynthSec::GnuVersionR))?;
        put(elf::DT_VERNEEDNUM as i64, dyn_info.verneed_num as u64)?;
    }
    if dyn_size(layout, SynthSec::MergedInitArray) > 0 {
        put(elf::DT_INIT_ARRAY as i64, dyn_addr(layout, SynthSec::MergedInitArray))?;
        put(elf::DT_INIT_ARRAYSZ as i64, dyn_size(layout, SynthSec::MergedInitArray))?;
    }
    if dyn_size(layout, SynthSec::MergedFiniArray) > 0 {
        put(elf::DT_FINI_ARRAY as i64, dyn_addr(layout, SynthSec::MergedFiniArray))?;
        put(elf::DT_FINI_ARRAYSZ as i64, dyn_size(layout, SynthSec::MergedFiniArray))?;
    }
    if dyn_info.init_addr != 0 {
        put(elf::DT_INIT as i64, dyn_info.init_addr)?;
    }
    if dyn_info.fini_addr != 0 {
        put(elf::DT_FINI as i64, dyn_info.fini_addr)?;
    }
    if let Some(rp) = dyn_info.runpath.as_deref() {
        put(elf::DT_RUNPATH as i64, str_off(rp)?)?;
    }
    put(elf::DT_NULL as i64, 0)?;
    Ok(())
}

pub struct SymOut {
    pub name_off: u32,
    pub info: u8,
    pub other: u8,
    pub shndx: u16,
    pub value: u64,
    pub size: u64,
}

pub struct SymtabData {
    pub syms: Vec<SymOut>,
    pub strtab: Vec<u8>,
    pub shstrtab: Vec<u8>,
    pub shstr_offs: Vec<u32>,
    pub first_global: u32,
}

fn shdr_kind_flags(o: &OutSec, g: &Graph, layout: &Layout, dyn_sym_idx: u32, dyn_str_idx: u32) -> (u32, u64, u32, u32, u64) {
    if o.synth != SynthSec::None {
        return match o.synth {
            SynthSec::Got | SynthSec::GotPlt => (1, 0x3, 0, 0, 0),
            SynthSec::Plt | SynthSec::IPlt => (1, 0x6, 0, 0, 0),
            SynthSec::RelaDyn => (4, 0x2, if dyn_sym_idx != 0 { dyn_sym_idx } else { 0 }, 0, 24),
            SynthSec::RelaPlt => (4, 0x2, dyn_sym_idx, 0, 24),
            SynthSec::DynSym => (11, 0x2, dyn_str_idx, 1, 24),
            SynthSec::DynStr => (3, 0x2, 0, 0, 0),
            SynthSec::Hash => (5, 0x2, dyn_sym_idx, 0, 4),
            SynthSec::GnuVersion => (0x6fffffff, 0x2, dyn_sym_idx, 0, 2),
            SynthSec::GnuVersionR => (0x6ffffffe, 0x0, dyn_str_idx, 0, 0),
            SynthSec::Dynamic => (6, 0x3, dyn_str_idx, 0, 16),
            SynthSec::Interp => (1, 0x2, 0, 0, 0),
            SynthSec::BuildId => (7, 0x2, 0, 0, 0),
            SynthSec::MergedEh => (1, 0x2, 0, 0, 0),
            SynthSec::MergedInitArray => (14, 0x3, 0, 0, 8),
            SynthSec::MergedFiniArray => (15, 0x3, 0, 0, 8),
            SynthSec::MergedInit | SynthSec::MergedFini => (1, 0x6, 0, 0, 0),
            SynthSec::MergedTdata => (1, 0x403, 0, 0, 0),
            SynthSec::MergedTbss => (8, 0x403, 0, 0, 0),
            SynthSec::None => (0, 0, 0, 0, 0),
        };
    }
    if let Some(key) = o.merge {
        let class = layout.merged.get(&key).map(|m| m.class).unwrap_or(SecClass::Rodata);
        return match class {
            SecClass::Text => (1, 0x6, 0, 0, 0),
            SecClass::Rodata | SecClass::ExceptTable => (1, 0x2, 0, 0, 0),
            SecClass::DataRelRo | SecClass::Data => (1, 0x3, 0, 0, 0),
            SecClass::Bss => (8, 0x3, 0, 0, 0),
            SecClass::Note => (7, 0x2, 0, 0, 0),
            _ => (1, 0x2, 0, 0, 0),
        };
    }
    match g.objs[o.obj].classes[o.sec as usize] {
        SecClass::Text | SecClass::Init | SecClass::Fini => (1, 0x6, 0, 0, 0),
        SecClass::Rodata | SecClass::ExceptTable => (1, 0x2, 0, 0, 0),
        SecClass::InitArray => (14, 0x3, 0, 0, 8),
        SecClass::FiniArray => (15, 0x3, 0, 0, 8),
        SecClass::DataRelRo | SecClass::Data => (1, 0x3, 0, 0, 0),
        SecClass::Bss => (8, 0x3, 0, 0, 0),
        SecClass::Note => (7, 0x2, 0, 0, 0),
        _ => (1, 0x2, 0, 0, 0),
    }
}

pub fn build_symtab(
    parsed: &[Object],
    g: &Graph,
    db: &crate::core::symdb::SymDb,
    layout: &Layout,
    static_link: bool,
) -> Result<SymtabData, LinkError> {
    let mut sec_shdr: BTreeMap<(usize, u32), u32> = BTreeMap::new();
    let mut merged_shdr: BTreeMap<SynthSec, u32> = BTreeMap::new();
    for (pos, o) in layout.order.iter().enumerate() {
        let idx = (pos + 1) as u32;
        match o.synth {
            SynthSec::None => {
                if let Some(key) = o.merge {
                    if let Some(grp) = layout.merged.get(&key) {
                        for &m in grp.members.iter() {
                            sec_shdr.insert(m, idx);
                        }
                    }
                    continue;
                }
                let class = g.objs[o.obj].classes[o.sec as usize];
                if !is_merged_class(class) {
                    sec_shdr.insert((o.obj, o.sec), idx);
                }
            }
            _ => {
                merged_shdr.insert(o.synth, idx);
            }
        }
    }
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if !g.kept[oi][si] || !is_merged_class(*c) {
                continue;
            }
            let synth = crate::core::layout::merged_synth_of(*c);
            if let Some(&idx) = merged_shdr.get(&synth) {
                sec_shdr.insert((oi, si as u32), idx);
            }
            let _ = obj;
        }
    }
    let bss_shdr = layout
        .order
        .iter()
        .enumerate()
        .filter(|(_, o)| outsec_class(g, layout, o) == Some(SecClass::Bss))
        .map(|(pos, _)| (pos + 1) as u32)
        .next()
        .or_else(|| merged_shdr.get(&SynthSec::MergedTbss).copied())
        .unwrap_or(0);
    let mut strtab = StrTab::new();
    let mut syms: Vec<SymOut> = Vec::new();
    syms.push(SymOut { name_off: 0, info: 0, other: 0, shndx: 0, value: 0, size: 0 });
    for (pos, o) in layout.order.iter().enumerate() {
        syms.push(SymOut {
            name_off: 0,
            info: 3,
            other: 0,
            shndx: (pos + 1) as u16,
            value: o.addr,
            size: 0,
        });
    }
    let mut local_defs: Vec<(String, u8, u8, u64, u64, u16)> = Vec::new();
    for (oi, obj) in parsed.iter().enumerate() {
        for (sym_idx, sym) in obj.symbols.iter().enumerate() {
            if sym.bind == STB_GLOBAL || sym.bind == STB_WEAK {
                continue;
            }
            if sym.kind == STT_FILE || sym.kind == STT_SECTION {
                continue;
            }
            if sym.shndx == SHN_UNDEF || sym.shndx == SHN_ABS || sym.shndx == SHN_COMMON {
                continue;
            }
            let si = sym.shndx as usize;
            if si >= g.objs[oi].classes.len() || !g.kept[oi][si] {
                continue;
            }
            if g.icf_redirect.contains_key(&(oi, sym.shndx)) {
                continue;
            }
            let name = match obj.symbol_name(sym_idx) {
                Ok(n) if !n.is_empty() && !n.starts_with(".L") => n.to_string(),
                _ => continue,
            };
            let class = g.objs[oi].classes[si];
            let addr = if is_merged_class(class) {
                let (synth, base) = merged_base_of(layout, class);
                let _ = synth;
                base + g.merged_off.get(&(oi, sym.shndx)).copied().unwrap_or(0) + sym.value
            } else {
                layout.sec_addr.get(&(oi, sym.shndx)).copied().unwrap_or(0) + sym.value
            };
            let value = if sym.kind == STT_TLS {
                g.tls.dtpoff.get(&(oi, sym.shndx)).copied().unwrap_or(0) + sym.value
            } else {
                addr
            };
            let shndx = sec_shdr.get(&(oi, sym.shndx)).copied().unwrap_or(0) as u16;
            local_defs.push((name, sym.kind, sym.vis, value, sym.size, shndx));
        }
    }
    local_defs.sort();
    local_defs.dedup();
    for (name, kind, vis, value, size, shndx) in local_defs {
        let name_off = strtab.add(&name);
        syms.push(SymOut { name_off, info: kind & 15, other: vis, shndx, value, size });
    }
    let first_global = syms.len() as u32;
    let mut names: Vec<&String> = db.names().collect();
    let mut und_names: Vec<&String> = Vec::new();
    for name in names.iter() {
        let e = db.get(name).unwrap();
        if e.def.is_none() && e.common_size == 0 && !e.refs.is_empty() && g.used_names.contains(*name) {
            und_names.push(name);
        }
    }
    und_names.sort();
    for name in und_names {
        let e = db.get(name).unwrap();
        let name_off = strtab.add(name);
        let bind = if e.ref_strong { STB_GLOBAL } else { STB_WEAK };
        let vis = if e.ref_default_vis { 0 } else { 2 };
        let (bind, vis) = if vis == 2 { (0, 2) } else { (bind, vis) };
        syms.push(SymOut { name_off, info: (bind << 4) | (e.ref_kind & 15), other: vis, shndx: 0, value: 0, size: 0 });
    }
    names.sort();
    for name in names {
        let e = db.get(name).unwrap();
        let d = e.def.as_ref();
        if d.is_some_and(|d| d.obj == usize::MAX && !layout.synth_addr.contains_key(name)) {
            continue;
        }
        let (value, kind, bind, _vis, size) = match def_value(g, layout, db, name) {
            Some(v) => v,
            None => {
                if e.common_size > 0 {
                    let v = g.common_addr.get(name).copied().unwrap_or(0);
                    (v, 1, 1, 0, e.common_size)
                } else {
                    continue;
                }
            }
        };
        let d = match d {
            Some(d) => d,
            None => {
                let name_off = strtab.add(name);
                syms.push(SymOut { name_off, info: (1 << 4) | 1, other: 0, shndx: bss_shdr as u16, value, size });
                continue;
            }
        };
        if e.common_size > 0 && d.bind != crate::core::obj::STB_GLOBAL {
            let name_off = strtab.add(name);
            syms.push(SymOut {
                name_off,
                info: (1 << 4) | 1,
                other: 0,
                shndx: bss_shdr as u16,
                value: g.common_addr.get(name).copied().unwrap_or(0),
                size: e.common_size,
            });
            continue;
        }
        let shndx = if d.obj == usize::MAX {
            synth_shdr_for(layout, &merged_shdr, name)
        } else if d.absolute {
            0xfff1
        } else {
            let mut id = (d.obj, d.sec);
            while let Some(&w) = g.icf_redirect.get(&id) {
                id = w;
            }
            sec_shdr.get(&id).copied().unwrap_or(0) as u16
        };
        let _ = (value, kind, size);
        let eff_vis = db.effective_vis(name);
        let (bind, vis) = if static_link && eff_vis == 2 { (0, 2) } else { (bind, eff_vis) };
        let name_off = strtab.add(name);
        syms.push(SymOut { name_off, info: (bind << 4) | (kind & 15), other: vis, shndx, value, size });
    }
    let strtab_bytes = strtab.bytes().to_vec();
    let mut shstrtab = StrTab::new();
    let mut shstr_offs = Vec::with_capacity(layout.order.len() + 3);
    for o in layout.order.iter() {
        shstr_offs.push(shstrtab.add(o.out_name));
    }
    shstr_offs.push(shstrtab.add(".symtab"));
    shstr_offs.push(shstrtab.add(".strtab"));
    shstr_offs.push(shstrtab.add(".shstrtab"));
    Ok(SymtabData { syms, strtab: strtab_bytes, shstrtab: shstrtab.bytes().to_vec(), shstr_offs, first_global })
}

fn synth_shdr_for(layout: &Layout, merged_shdr: &BTreeMap<SynthSec, u32>, name: &str) -> u16 {
    let _ = layout;
    let key = if name == "_GLOBAL_OFFSET_TABLE_" {
        SynthSec::Got
    } else if name.contains("init_array") {
        SynthSec::MergedInitArray
    } else if name.contains("fini_array") {
        SynthSec::MergedFiniArray
    } else if name.contains("rela_iplt") {
        SynthSec::RelaDyn
    } else if name.contains("preinit_array") {
        SynthSec::MergedInitArray
    } else {
        SynthSec::Got
    };
    merged_shdr.get(&key).copied().unwrap_or(0) as u16
}

pub struct TableLayout {
    pub shoff: u64,
    pub symtab_off: u64,
    pub strtab_off: u64,
    pub shstrtab_off: u64,
    pub total: u64,
    pub shnum: usize,
}

pub fn table_layout(layout: &Layout, symdata: &SymtabData, strip: bool) -> TableLayout {
    let shnum = 1 + layout.order.len() + if strip { 1 } else { 3 };
    let shoff = layout.file_size.next_multiple_of(8);
    if strip {
        let shstrtab_off = (shoff + shnum as u64 * 64).next_multiple_of(8);
        let total = (shstrtab_off + symdata.shstrtab.len() as u64).next_multiple_of(8);
        return TableLayout { shoff, symtab_off: 0, strtab_off: 0, shstrtab_off, total, shnum };
    }
    let symtab_off = shoff + shnum as u64 * 64;
    let strtab_off = (symtab_off + symdata.syms.len() as u64 * 24).next_multiple_of(8);
    let shstrtab_off = (strtab_off + symdata.strtab.len() as u64).next_multiple_of(8);
    let total = (shstrtab_off + symdata.shstrtab.len() as u64).next_multiple_of(8);
    TableLayout { shoff, symtab_off, strtab_off, shstrtab_off, total, shnum }
}

pub fn output_size(
    parsed: &[Object],
    g: &Graph,
    db: &crate::core::symdb::SymDb,
    layout: &Layout,
    static_link: bool,
    strip: bool,
) -> Result<u64, LinkError> {
    let symdata = build_symtab(parsed, g, db, layout, static_link)?;
    Ok(table_layout(layout, &symdata, strip).total)
}

fn seg_flags(seg: SegClass) -> u32 {
    match seg {
        SegClass::R => elf::PF_R,
        SegClass::RX => elf::PF_R | elf::PF_X,
        SegClass::Relro | SegClass::RW => elf::PF_R | elf::PF_W,
    }
}

pub fn emit_tables(
    g: &Graph,
    layout: &Layout,
    dyn_info: &DynInfo,
    is_exec: bool,
    symdata: &SymtabData,
    strip: bool,
    out: &mut [u8],
    target: &dyn Target,
) -> Result<ShdrTable, LinkError> {
    let tl = table_layout(layout, symdata, strip);
    let dyn_sym_idx = layout.order.iter().position(|o| o.synth == SynthSec::DynSym).map(|p| (p + 1) as u32).unwrap_or(0);
    let dyn_str_idx = layout.order.iter().position(|o| o.synth == SynthSec::DynStr).map(|p| (p + 1) as u32).unwrap_or(0);
    let mut entries: Vec<ShdrEntry> = Vec::new();
    entries.push(ShdrEntry { name: 0, kind: 0, flags: 0, addr: 0, offset: 0, size: 0, link: 0, info: 0, align: 0, entsize: 0 });
    for (i, o) in layout.order.iter().enumerate() {
        let name = symdata.shstr_offs[i];
        let (kind, flags, link, mut info, entsize) = shdr_kind_flags(o, g, layout, dyn_sym_idx, dyn_str_idx);
        if o.synth == SynthSec::GnuVersionR {
            info = dyn_info.verneed_num as u32;
        }
        entries.push(ShdrEntry {
            name,
            kind,
            flags,
            addr: o.addr,
            offset: o.offset,
            size: o.size,
            link,
            info,
            align: o.align,
            entsize,
        });
    }
    let symtab_idx = entries.len() as u32;
    if !strip {
        entries.push(ShdrEntry {
            name: symdata.shstr_offs[layout.order.len()],
            kind: 2,
            flags: 0,
            addr: 0,
            offset: tl.symtab_off,
            size: symdata.syms.len() as u64 * 24,
            link: symtab_idx + 1,
            info: symdata.first_global,
            align: 8,
            entsize: 24,
        });
        entries.push(ShdrEntry {
            name: symdata.shstr_offs[layout.order.len() + 1],
            kind: 3,
            flags: 0,
            addr: 0,
            offset: tl.strtab_off,
            size: symdata.strtab.len() as u64,
            link: 0,
            info: 0,
            align: 1,
            entsize: 0,
        });
    }
    let shstrtab_idx = entries.len() as u32;
    entries.push(ShdrEntry {
        name: symdata.shstr_offs[layout.order.len() + 2],
        kind: 3,
        flags: 0,
        addr: 0,
        offset: tl.shstrtab_off,
        size: symdata.shstrtab.len() as u64,
        link: 0,
        info: 0,
        align: 1,
        entsize: 0,
    });
    let shstrtab_bytes = symdata.shstrtab.clone();
    for (i, e) in entries.iter().enumerate() {
        let at = tl.shoff as usize + i * 64;
        let slot = out.get_mut(at..at + 64).ok_or_else(|| LinkError::Native("shdr overflow".to_string()))?;
        elf::write_shdr(slot, e.name, e.kind, e.flags, e.addr, e.offset, e.size, e.link, e.info, e.align, e.entsize);
    }
    if !strip {
        for (i, s) in symdata.syms.iter().enumerate() {
            let at = tl.symtab_off as usize + i * 24;
            let slot = out.get_mut(at..at + 24).ok_or_else(|| LinkError::Native("symtab overflow".to_string()))?;
            elf::write_sym(slot, s.name_off, s.info, s.other, s.shndx, s.value, s.size);
        }
        out.get_mut(tl.strtab_off as usize..tl.strtab_off as usize + symdata.strtab.len())
            .ok_or_else(|| LinkError::Native("strtab overflow".to_string()))?
            .copy_from_slice(&symdata.strtab);
    }
    out.get_mut(tl.shstrtab_off as usize..tl.shstrtab_off as usize + shstrtab_bytes.len())
        .ok_or_else(|| LinkError::Native("shstrtab overflow".to_string()))?
        .copy_from_slice(&shstrtab_bytes);
    let mut loads: Vec<(u64, u64, u64, u64, u32)> = Vec::new();
    for seg in layout.segments.iter() {
        if seg.memsz == 0 {
            continue;
        }
        loads.push((seg.offset, seg.vaddr, seg.filesz, seg.memsz, seg_flags(seg.seg)));
    }
    let mut extra: Vec<(u32, u32, u64, u64, u64, u64, u64)> = Vec::new();
    for o in layout.order.iter() {
        if o.out_name == ".interp" {
            extra.push((elf::PT_INTERP, elf::PF_R, o.offset, o.addr, o.size, o.size, 1));
        }
    }
    for o in layout.order.iter() {
        if o.synth == SynthSec::Dynamic {
            extra.push((elf::PT_DYNAMIC, elf::PF_R | elf::PF_W, o.offset, o.addr, o.size, o.size, 8));
        }
    }
    if layout.tls_size > 0 {
        if let Some(o) = layout.order.iter().find(|o| o.synth == SynthSec::MergedTdata) {
            extra.push((elf::PT_TLS, elf::PF_R, o.offset, o.addr, o.size, layout.tls_size, g.tls.align.max(1)));
        }
    }
    if layout.relro_end > layout.relro_start {
        if let Some(seg) = layout.segments.iter().find(|s| s.seg == SegClass::Relro && s.memsz > 0) {
            let off = seg.offset + (layout.relro_start - seg.vaddr);
            extra.push((elf::PT_GNU_RELRO, elf::PF_R, off, layout.relro_start, layout.relro_end - layout.relro_start, layout.relro_end - layout.relro_start, 1));
        }
    }
    let stack_flags = if g.exec_stack { elf::PF_R | elf::PF_W | elf::PF_X } else { elf::PF_R | elf::PF_W };
    let note_count = layout
        .order
        .iter()
        .filter(|o| outsec_class(g, layout, o) == Some(SecClass::Note) || o.synth == SynthSec::BuildId)
        .count();
    let phnum = 1 + loads.len() + extra.len() + 1 + note_count;
    let mut ph_at = 64usize;
    fn write_ph(out: &mut [u8], ph_at: &mut usize, kind: u32, flags: u32, offset: u64, vaddr: u64, filesz: u64, memsz: u64, align: u64) -> Result<(), LinkError> {
        let slot = out.get_mut(*ph_at..*ph_at + 56).ok_or_else(|| LinkError::Native("phdr overflow".to_string()))?;
        elf::write_phdr(slot, kind, flags, offset, vaddr, filesz, memsz, align);
        *ph_at += 56;
        Ok(())
    }
    write_ph(out, &mut ph_at, elf::PT_PHDR, elf::PF_R, 64, layout.base + 64, phnum as u64 * 56, phnum as u64 * 56, 8)?;
    for (offset, vaddr, filesz, memsz, flags) in loads {
        write_ph(out, &mut ph_at, elf::PT_LOAD, flags, offset, vaddr, filesz, memsz, PAGE)?;
    }
    for (kind, flags, offset, vaddr, filesz, memsz, align) in extra {
        write_ph(out, &mut ph_at, kind, flags, offset, vaddr, filesz, memsz, align)?;
    }
    write_ph(out, &mut ph_at, elf::PT_GNU_STACK, stack_flags, 0, 0, 0, 0, 0)?;
    for o in layout.order.iter() {
        if outsec_class(g, layout, o) == Some(SecClass::Note) || o.synth == SynthSec::BuildId {
            write_ph(out, &mut ph_at, elf::PT_NOTE, elf::PF_R, o.offset, o.addr, o.size, o.size, o.align)?;
        }
    }
    let shnum = entries.len();
    elf::write_ehdr(
        out.get_mut(0..64).ok_or_else(|| LinkError::Native("ehdr overflow".to_string()))?,
        is_exec,
        target.machine(),
        layout.entry_addr,
        64,
        tl.shoff,
        phnum as u16,
        shnum as u16,
        shstrtab_idx as u16,
        dyn_info.osabi,
    );
    Ok(ShdrTable { entries, shoff: tl.shoff })
}

pub fn sha1(data: &[u8]) -> [u8; 20] {
    let mut h: [u32; 5] = [0x67452301, 0xefcdab89, 0x98badcfe, 0x10325476, 0xc3d2e1f0];
    let bit_len = (data.len() as u64).wrapping_mul(8);
    let mut msg = data.to_vec();
    msg.push(0x80);
    while msg.len() % 64 != 56 {
        msg.push(0);
    }
    msg.extend_from_slice(&bit_len.to_be_bytes());
    for chunk in msg.chunks_exact(64) {
        let mut w = [0u32; 80];
        for i in 0..16 {
            w[i] = u32::from_be_bytes([chunk[4 * i], chunk[4 * i + 1], chunk[4 * i + 2], chunk[4 * i + 3]]);
        }
        for i in 16..80 {
            w[i] = (w[i - 3] ^ w[i - 8] ^ w[i - 14] ^ w[i - 16]).rotate_left(1);
        }
        let (mut a, mut b, mut c, mut d, mut e) = (h[0], h[1], h[2], h[3], h[4]);
        for i in 0..80 {
            let (f, k) = match i {
                0..=19 => ((b & c) | ((!b) & d), 0x5a827999),
                20..=39 => (b ^ c ^ d, 0x6ed9eba1),
                40..=59 => ((b & c) | (b & d) | (c & d), 0x8f1bbcdc),
                _ => (b ^ c ^ d, 0xca62c1d6),
            };
            let t = a
                .rotate_left(5)
                .wrapping_add(f)
                .wrapping_add(e)
                .wrapping_add(k)
                .wrapping_add(w[i]);
            e = d;
            d = c;
            c = b.rotate_left(30);
            b = a;
            a = t;
        }
        h[0] = h[0].wrapping_add(a);
        h[1] = h[1].wrapping_add(b);
        h[2] = h[2].wrapping_add(c);
        h[3] = h[3].wrapping_add(d);
        h[4] = h[4].wrapping_add(e);
    }
    let mut out = [0u8; 20];
    for (i, v) in h.iter().enumerate() {
        out[4 * i..4 * i + 4].copy_from_slice(&v.to_be_bytes());
    }
    out
}
