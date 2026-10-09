use crate::core::graph::Graph;
use crate::core::obj::{SHT_NOBITS, SHF_COMPRESSED};
use crate::LinkError;
use std::collections::BTreeMap;

pub use crate::core::graph::SecClass;

pub const BASE_ADDR: u64 = 0x400000;
pub const PAGE: u64 = 0x1000;

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum SegClass {
    R,
    RX,
    Relro,
    RW,
}

#[derive(Clone, Debug)]
pub struct OutSec {
    pub obj: usize,
    pub sec: u32,
    pub synth: SynthSec,
    pub merge: Option<MergeKey>,
    pub addr: u64,
    pub offset: u64,
    pub size: u64,
    pub align: u64,
    pub seg: SegClass,
    pub out_name: &'static str,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct MergeKey {
    pub name: &'static str,
    pub kind: u32,
    pub flags: u64,
    pub align: u64,
}

#[derive(Clone, Debug)]
pub struct MergedGroup {
    pub class: SecClass,
    pub members: Vec<(usize, u32)>,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum SynthSec {
    None,
    Got,
    GotPlt,
    Plt,
    IPlt,
    RelaDyn,
    RelaPlt,
    DynSym,
    DynStr,
    Hash,
    GnuVersion,
    GnuVersionR,
    Dynamic,
    Interp,
    BuildId,
    MergedEh,
    MergedInitArray,
    MergedFiniArray,
    MergedTdata,
    MergedTbss,
    MergedInit,
    MergedFini,
}

#[derive(Clone, Debug)]
pub struct Segment {
    pub seg: SegClass,
    pub vaddr: u64,
    pub offset: u64,
    pub filesz: u64,
    pub memsz: u64,
}

#[derive(Clone, Debug)]
pub struct Layout {
    pub order: Vec<OutSec>,
    pub sec_addr: BTreeMap<(usize, u32), u64>,
    pub merged: BTreeMap<MergeKey, MergedGroup>,
    pub synth_addr: BTreeMap<String, u64>,
    pub segments: Vec<Segment>,
    pub base: u64,
    pub entry_addr: u64,
    pub got_addr: u64,
    pub got_size: u64,
    pub got_plt_addr: u64,
    pub plt_addr: u64,
    pub plt_size: u64,
    pub iplt_addr: u64,
    pub iplt_size: u64,
    pub tls_start: u64,
    pub tls_filesz: u64,
    pub tls_size: u64,
    pub relro_start: u64,
    pub relro_end: u64,
    pub file_size: u64,
}

#[derive(Clone, Debug)]
pub struct SynthItem {
    pub synth: SynthSec,
    pub name: &'static str,
    pub size: u64,
    pub align: u64,
    pub seg: SegClass,
}

fn align_up(v: u64, a: u64) -> u64 {
    v.next_multiple_of(a.max(1))
}


fn seg_of(class: SecClass) -> SegClass {
    match class {
        SecClass::Text | SecClass::Init | SecClass::Fini => SegClass::RX,
        SecClass::Tdata | SecClass::InitArray | SecClass::FiniArray | SecClass::DataRelRo => SegClass::Relro,
        SecClass::Data | SecClass::Bss | SecClass::Tbss => SegClass::RW,
        _ => SegClass::R,
    }
}

fn out_name_of(class: SecClass, sec_name: &str) -> &'static str {
    match class {
        SecClass::Text => ".text",
        SecClass::Init => ".init",
        SecClass::Fini => ".fini",
        SecClass::Rodata => ".rodata",
        SecClass::EhFrame => ".eh_frame",
        SecClass::ExceptTable => ".gcc_except_table",
        SecClass::InitArray => ".init_array",
        SecClass::FiniArray => ".fini_array",
        SecClass::DataRelRo => ".data.rel.ro",
        SecClass::Data => ".data",
        SecClass::Bss => ".bss",
        SecClass::Tdata => ".tdata",
        SecClass::Tbss => ".tbss",
        SecClass::Note => {
            if sec_name.contains("ABI-tag") {
                ".note.ABI-tag"
            } else {
                ".note"
            }
        }
        SecClass::Debug => match sec_name {
            ".debug_info" => ".debug_info",
            ".debug_abbrev" => ".debug_abbrev",
            ".debug_line" => ".debug_line",
            ".debug_str" => ".debug_str",
            ".debug_aranges" => ".debug_aranges",
            ".debug_ranges" => ".debug_ranges",
            ".debug_loc" => ".debug_loc",
            ".debug_loclists" => ".debug_loclists",
            ".debug_rnglists" => ".debug_rnglists",
            ".debug_addr" => ".debug_addr",
            ".debug_names" => ".debug_names",
            ".debug_frame" => ".debug_frame",
            ".debug_macinfo" => ".debug_macinfo",
            ".debug_macro" => ".debug_macro",
            ".debug_types" => ".debug_types",
            _ => ".debug",
        },
        SecClass::NoteProperty | SecClass::GnuStack | SecClass::Skip => ".note",
    }
}

pub fn is_merged(class: SecClass) -> bool {
    matches!(
        class,
        SecClass::EhFrame | SecClass::InitArray | SecClass::FiniArray | SecClass::Tdata | SecClass::Tbss | SecClass::Init | SecClass::Fini
    )
}

pub fn merged_synth_of(class: SecClass) -> SynthSec {
    match class {
        SecClass::InitArray => SynthSec::MergedInitArray,
        SecClass::FiniArray => SynthSec::MergedFiniArray,
        SecClass::Tdata => SynthSec::MergedTdata,
        SecClass::Tbss => SynthSec::MergedTbss,
        SecClass::Init => SynthSec::MergedInit,
        SecClass::Fini => SynthSec::MergedFini,
        _ => SynthSec::MergedEh,
    }
}

fn merge_key(class: SecClass, sec: &crate::core::obj::Section, sec_name: &str) -> Option<MergeKey> {
    if sec.flags & SHF_COMPRESSED != 0 {
        return None;
    }
    match class {
        SecClass::Text
        | SecClass::Rodata
        | SecClass::ExceptTable
        | SecClass::DataRelRo
        | SecClass::Data
        | SecClass::Bss
        | SecClass::Debug
        | SecClass::Note => Some(MergeKey {
            name: out_name_of(class, sec_name),
            kind: sec.kind,
            flags: sec.flags,
            align: sec.align.max(1),
        }),
        _ => None,
    }
}

pub fn outsec_class(g: &Graph, layout: &Layout, o: &OutSec) -> Option<SecClass> {
    if o.synth != SynthSec::None {
        return None;
    }
    if let Some(key) = o.merge {
        return layout.merged.get(&key).map(|m| m.class);
    }
    if o.obj == usize::MAX {
        return None;
    }
    g.objs.get(o.obj).and_then(|d| d.classes.get(o.sec as usize).copied())
}

pub fn layout(
    g: &mut Graph,
    parsed: &[crate::core::obj::Object],
    synth: &[SynthItem],
    base: u64,
) -> Result<Layout, LinkError> {
    let mut singles: BTreeMap<SegClass, Vec<(usize, u32)>> = BTreeMap::new();
    let mut merges: BTreeMap<MergeKey, MergedGroup> = BTreeMap::new();
    for (oi, _obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if !g.kept[oi][si] {
                continue;
            }
            if *c == SecClass::Skip || *c == SecClass::GnuStack || *c == SecClass::NoteProperty {
                continue;
            }
            if is_merged(*c) {
                continue;
            }
            let sec = &parsed[oi].sections[si];
            let name = parsed[oi].section_name(si).unwrap_or("");
            match merge_key(*c, sec, name) {
                Some(key) => {
                    let grp = merges.entry(key).or_insert_with(|| MergedGroup { class: *c, members: Vec::new() });
                    grp.members.push((oi, si as u32));
                }
                None => {
                    singles.entry(seg_of(*c)).or_default().push((oi, si as u32));
                }
            }
        }
    }
    for v in singles.values_mut() {
        v.sort();
    }
    let mut merged_groups: BTreeMap<SegClass, Vec<SynthSec>> = BTreeMap::new();
    let push_merged = |groups: &mut BTreeMap<SegClass, Vec<SynthSec>>, synth: SynthSec, seg: SegClass, present: bool| {
        if present {
            groups.entry(seg).or_default().push(synth);
        }
    };
    let has_eh = g.eh.total > 0;
    let has_init = parsed.iter().enumerate().any(|(oi, _)| {
        g.objs[oi].classes.iter().zip(g.kept[oi].iter()).any(|(&c, &k)| k && c == SecClass::InitArray)
    });
    let has_fini = parsed.iter().enumerate().any(|(oi, _)| {
        g.objs[oi].classes.iter().zip(g.kept[oi].iter()).any(|(&c, &k)| k && c == SecClass::FiniArray)
    });
    let has_tls = g.tls.size > 0;
    let has_init_text = parsed.iter().enumerate().any(|(oi, _)| {
        g.objs[oi].classes.iter().zip(g.kept[oi].iter()).any(|(&c, &k)| k && c == SecClass::Init)
    });
    let has_fini_text = parsed.iter().enumerate().any(|(oi, _)| {
        g.objs[oi].classes.iter().zip(g.kept[oi].iter()).any(|(&c, &k)| k && c == SecClass::Fini)
    });
    push_merged(&mut merged_groups, SynthSec::MergedEh, SegClass::R, has_eh);
    push_merged(&mut merged_groups, SynthSec::MergedInit, SegClass::RX, has_init_text);
    push_merged(&mut merged_groups, SynthSec::MergedFini, SegClass::RX, has_fini_text);
    push_merged(&mut merged_groups, SynthSec::MergedTdata, SegClass::Relro, has_tls);
    push_merged(&mut merged_groups, SynthSec::MergedTbss, SegClass::Relro, has_tls);
    push_merged(&mut merged_groups, SynthSec::MergedInitArray, SegClass::Relro, has_init);
    push_merged(&mut merged_groups, SynthSec::MergedFiniArray, SegClass::Relro, has_fini);
    let note_count: usize = singles
        .values()
        .flatten()
        .filter(|&&(oi, si)| g.objs[oi].classes[si as usize] == SecClass::Note)
        .count()
        + merges.values().filter(|m| m.class == SecClass::Note).count();
    let merges_in = |seg: SegClass| merges.values().any(|m| seg_of(m.class) == seg);
    let nloads = [SegClass::R, SegClass::RX, SegClass::Relro, SegClass::RW]
        .iter()
        .filter(|s| {
            singles.get(s).is_some_and(|v| !v.is_empty())
                || merges_in(**s)
                || merged_groups.get(s).is_some_and(|v| !v.is_empty())
                || synth.iter().any(|it| &it.seg == *s)
        })
        .count();
    let has_tls = g.tls.size > 0;
    let has_relro = merged_groups.get(&SegClass::Relro).is_some_and(|v| !v.is_empty())
        || singles.get(&SegClass::Relro).is_some_and(|v| !v.is_empty())
        || merges_in(SegClass::Relro)
        || synth.iter().any(|it| it.seg == SegClass::Relro);
    let has_interp = synth.iter().any(|it| it.synth == SynthSec::Interp);
    let has_dynamic = synth.iter().any(|it| it.synth == SynthSec::Dynamic);
    let build_notes = synth.iter().filter(|it| it.synth == SynthSec::BuildId).count();
    let phnum = 1 + nloads + usize::from(has_tls) + usize::from(has_relro) + 1 + note_count + usize::from(has_interp) + usize::from(has_dynamic) + build_notes;
    let mut order: Vec<OutSec> = Vec::new();
    let mut sec_addr: BTreeMap<(usize, u32), u64> = BTreeMap::new();
    let mut synth_addr: BTreeMap<String, u64> = BTreeMap::new();
    let mut vaddr = base;
    let header_reserve = (64 + phnum as u64 * 56).next_multiple_of(16);
    let mut file_off = header_reserve;
    let mut segments: Vec<Segment> = Vec::new();
    let mut relro_start = 0u64;
    let mut relro_end = 0u64;
    let mut first_seg = true;
    for seg in [SegClass::R, SegClass::RX, SegClass::Relro, SegClass::RW] {
        let members = singles.get(&seg).cloned().unwrap_or_default();
        let merged_here = merged_groups.get(&seg).cloned().unwrap_or_default();
        let grouped: Vec<(MergeKey, SecClass)> = merges
            .iter()
            .filter(|(_, m)| seg_of(m.class) == seg)
            .map(|(k, m)| (*k, m.class))
            .collect();
        let synth_here: Vec<&SynthItem> = synth.iter().filter(|s| s.seg == seg).collect();
        if members.is_empty() && grouped.is_empty() && merged_here.is_empty() && synth_here.is_empty() {
            continue;
        }
        if first_seg {
            vaddr = base;
            file_off = 0;
        } else {
            vaddr = align_up(vaddr, PAGE);
            file_off = align_up(file_off, PAGE);
        }
        let seg_vaddr = vaddr;
        let seg_off = file_off;
        let k = seg_vaddr.wrapping_sub(seg_off);
        if first_seg {
            vaddr += header_reserve;
            file_off += header_reserve;
        }
        first_seg = false;
        for (oi, si) in members {
            let class = g.objs[oi].classes[si as usize];
            let sec = &parsed[oi].sections[si as usize];
            let align = sec.align.max(1);
            let size = sec.size;
            let name = parsed[oi].section_name(si as usize).unwrap_or("");
            vaddr = align_up(vaddr, align);
            file_off = vaddr.wrapping_sub(k);
            sec_addr.insert((oi, si), vaddr);
            order.push(OutSec {
                obj: oi,
                sec: si,
                synth: SynthSec::None,
                merge: None,
                addr: vaddr,
                offset: file_off,
                size,
                align,
                seg,
                out_name: out_name_of(class, name),
            });
            vaddr += size;
            if sec.kind != SHT_NOBITS {
                file_off += size;
            }
        }
        for (key, _class) in grouped {
            let members = merges.get(&key).map(|m| m.members.clone()).unwrap_or_default();
            vaddr = align_up(vaddr, key.align);
            file_off = vaddr.wrapping_sub(k);
            let base = vaddr;
            let base_off = file_off;
            let mut running = 0u64;
            for &(oi, si) in members.iter() {
                running = align_up(running, key.align);
                sec_addr.insert((oi, si), base + running);
                g.merged_off.insert((oi, si), running);
                running += parsed[oi].sections[si as usize].size;
            }
            order.push(OutSec {
                obj: usize::MAX,
                sec: u32::MAX,
                synth: SynthSec::None,
                merge: Some(key),
                addr: base,
                offset: base_off,
                size: running,
                align: key.align,
                seg,
                out_name: key.name,
            });
            vaddr = base + running;
            if key.kind != SHT_NOBITS {
                file_off += running;
            }
        }
        let mut skip_tbss = false;
        for m in merged_here {
            if m == SynthSec::MergedTbss && skip_tbss {
                continue;
            }
            if m == SynthSec::MergedTdata {
                vaddr = align_up(vaddr, 8);
                file_off = vaddr.wrapping_sub(k);
                let base = vaddr;
                let tdata_end = g.tls.tdata_end;
                for &(oi, si) in g.tls.order.iter() {
                    let addr = base + g.tls.dtpoff[&(oi, si)];
                    sec_addr.insert((oi, si), addr);
                    g.merged_off.insert((oi, si), g.tls.dtpoff[&(oi, si)]);
                }
                order.push(OutSec {
                    obj: usize::MAX,
                    sec: u32::MAX,
                    synth: SynthSec::MergedTdata,
                    merge: None,
                    addr: base,
                    offset: file_off,
                    size: tdata_end,
                    align: 8,
                    seg,
                    out_name: ".tdata",
                });
                vaddr = base + tdata_end;
                file_off += tdata_end;
                let tbss_size = g.tls.size - tdata_end;
                order.push(OutSec {
                    obj: usize::MAX,
                    sec: u32::MAX,
                    synth: SynthSec::MergedTbss,
                    merge: None,
                    addr: vaddr,
                    offset: file_off,
                    size: tbss_size,
                    align: 8,
                    seg,
                    out_name: ".tbss",
                });
                vaddr += tbss_size;
                skip_tbss = true;
                continue;
            }
            let (_, oname, osize, oalign) = merged_spec(g, parsed, m);
            if m == SynthSec::MergedInitArray
                || m == SynthSec::MergedFiniArray
                || m == SynthSec::MergedInit
                || m == SynthSec::MergedFini
            {
                let want = match m {
                    SynthSec::MergedInitArray => SecClass::InitArray,
                    SynthSec::MergedFiniArray => SecClass::FiniArray,
                    SynthSec::MergedInit => SecClass::Init,
                    _ => SecClass::Fini,
                };
                vaddr = align_up(vaddr, oalign);
                file_off = vaddr.wrapping_sub(k);
                let base = vaddr;
                let mut running = 0u64;
                for (oi, obj) in parsed.iter().enumerate() {
                    for (si, c) in g.objs[oi].classes.iter().enumerate() {
                        if *c != want || !g.kept[oi][si] {
                            continue;
                        }
                        sec_addr.insert((oi, si as u32), base + running);
                        g.merged_off.insert((oi, si as u32), running);
                        running += obj.sections[si].size;
                    }
                }
                order.push(OutSec {
                    obj: usize::MAX,
                    sec: u32::MAX,
                    synth: m,
                    merge: None,
                    addr: base,
                    offset: file_off,
                    size: osize,
                    align: oalign,
                    seg,
                    out_name: oname,
                });
                vaddr = base + osize;
                file_off += osize;
                continue;
            }
            vaddr = align_up(vaddr, oalign);
            file_off = vaddr.wrapping_sub(k);
            order.push(OutSec {
                obj: usize::MAX,
                sec: u32::MAX,
                synth: m,
                merge: None,
                addr: vaddr,
                offset: file_off,
                size: osize,
                align: oalign,
                seg,
                out_name: oname,
            });
            vaddr += osize;
            file_off += osize;
        }
        for s in synth_here {
            vaddr = align_up(vaddr, s.align);
            file_off = vaddr.wrapping_sub(k);
            order.push(OutSec {
                obj: usize::MAX,
                sec: u32::MAX,
                synth: s.synth,
                merge: None,
                addr: vaddr,
                offset: file_off,
                size: s.size,
                align: s.align,
                seg,
                out_name: s.name,
            });
            vaddr += s.size;
            file_off += s.size;
        }
        let seg_filesz = file_off - seg_off;
        let mut seg_memsz = vaddr - seg_vaddr;
        if seg == SegClass::RW {
            for (name, size, a) in g.commons.iter() {
                vaddr = align_up(vaddr, *a);
                g.common_addr.insert(name.clone(), vaddr);
                vaddr += size;
            }
            seg_memsz = vaddr - seg_vaddr;
        }
        segments.push(Segment { seg, vaddr: seg_vaddr, offset: seg_off, filesz: seg_filesz, memsz: seg_memsz });
        if seg == SegClass::Relro {
            relro_start = align_up(seg_vaddr, PAGE);
            relro_end = vaddr;
        }
    }
    let entry_addr = match g.entry {
        Some((oi, si, val)) => sec_addr.get(&(oi, si)).copied().unwrap_or(0) + val,
        None => return Err(LinkError::Native("entry symbol has no address".to_string())),
    };
    let find = |synth: SynthSec| -> (u64, u64) {
        order.iter().find(|o| o.synth == synth).map(|o| (o.addr, o.size)).unwrap_or((0, 0))
    };
    let (got_addr, got_size) = find(SynthSec::Got);
    let (got_plt_addr, _) = find(SynthSec::GotPlt);
    let (plt_addr, plt_size) = find(SynthSec::Plt);
    let (iplt_addr, iplt_size) = find(SynthSec::IPlt);
    let tls_start = order.iter().find(|o| o.synth == SynthSec::MergedTdata).map(|o| o.addr).unwrap_or(0);
    let tls_filesz = order.iter().find(|o| o.synth == SynthSec::MergedTdata).map(|o| o.size).unwrap_or(0);
    if got_addr != 0 {
        synth_addr.insert(
            "_GLOBAL_OFFSET_TABLE_".to_string(),
            if got_plt_addr != 0 { got_plt_addr } else { got_addr },
        );
    }
    Ok(Layout {
        order,
        sec_addr,
        merged: merges,
        synth_addr,
        segments,
        base,
        entry_addr,
        got_addr,
        got_size,
        got_plt_addr,
        plt_addr,
        plt_size,
        iplt_addr,
        iplt_size,
        tls_start,
        tls_filesz,
        tls_size: g.tls.size,
        relro_start,
        relro_end,
        file_size: file_off,
    })
}

pub fn synth_base(layout: &Layout, synth: SynthSec) -> u64 {
    layout.order.iter().find(|o| o.synth == synth).map(|o| o.addr).unwrap_or(0)
}

fn kept_class_size(g: &Graph, parsed: &[crate::core::obj::Object], class: SecClass) -> u64 {
    let mut total = 0u64;
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, c) in g.objs[oi].classes.iter().enumerate() {
            if *c == class && g.kept[oi][si] {
                total += obj.sections[si].size;
            }
        }
    }
    total
}

fn merged_spec(g: &Graph, parsed: &[crate::core::obj::Object], m: SynthSec) -> (SynthSec, &'static str, u64, u64) {
    match m {
        SynthSec::MergedInit => (m, ".init", kept_class_size(g, parsed, SecClass::Init), 4),
        SynthSec::MergedFini => (m, ".fini", kept_class_size(g, parsed, SecClass::Fini), 4),
        SynthSec::MergedEh => (m, ".eh_frame", g.eh.total, 8),
        SynthSec::MergedInitArray => (m, ".init_array", kept_class_size(g, parsed, SecClass::InitArray), 8),
        SynthSec::MergedFiniArray => (m, ".fini_array", kept_class_size(g, parsed, SecClass::FiniArray), 8),
        SynthSec::MergedTdata => (m, ".tdata", tls_filesz(g, parsed), 8),
        SynthSec::MergedTbss => (m, ".tbss", g.tls.size.saturating_sub(tls_filesz(g, parsed)), 8),
        _ => (m, ".merged", 0, 1),
    }
}

fn tls_filesz(g: &Graph, parsed: &[crate::core::obj::Object]) -> u64 {
    let mut total = 0u64;
    for &(oi, si) in g.tls.order.iter() {
        if g.objs[oi].classes[si as usize] == SecClass::Tdata {
            total += parsed[oi].sections[si as usize].size;
        }
    }
    total
}
