use crate::core::archive::Archive;
use crate::core::graph::{self, DynSym, Graph, SecClass};
use crate::core::layout::{self, Layout, SegClass, SynthItem, SynthSec};
use crate::core::obj::{Object, STB_GLOBAL, STB_WEAK, STT_FILE};
use crate::core::symdb::{Def, SymDb};
use crate::core::writer::{self, DynInfo};
use crate::target::x86_64::{R_32, R_32S, R_64, X86_64};
use crate::target::{Plan, Target};
use crate::LinkError;
use crate::LinkInput;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub struct StaticOpts {
    pub entry: String,
    pub icf: bool,
    pub target: String,
    pub strip: bool,
}

impl Default for StaticOpts {
    fn default() -> StaticOpts {
        StaticOpts { entry: "_start".to_string(), icf: true, target: "x86_64-unknown-linux-gnu".to_string(), strip: false }
    }
}

pub struct DynamicOpts {
    pub entry: String,
    pub icf: bool,
    pub target: String,
    pub lib_dirs: Vec<PathBuf>,
    pub libs: Vec<String>,
    pub runpath: Option<String>,
    pub strip: bool,
}

impl Default for DynamicOpts {
    fn default() -> DynamicOpts {
        DynamicOpts {
            entry: "_start".to_string(),
            icf: true,
            target: "x86_64-unknown-linux-gnu".to_string(),
            lib_dirs: Vec::new(),
            libs: Vec::new(),
            runpath: None,
            strip: false,
        }
    }
}

pub static TIME: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

macro_rules! tphase {
    ($name:expr, $block:expr) => {{
        let __t0 = std::time::Instant::now();
        let __r = $block;
        if TIME.load(std::sync::atomic::Ordering::Relaxed) {
            eprintln!("t-{} {:?}", $name, __t0.elapsed());
        }
        __r
    }};
}

#[derive(Debug)]
pub struct NativeStats {
    pub output_size: u64,
    pub kept_sections: usize,
    pub got_slots: usize,
    pub plt_entries: usize,
    pub folded: usize,
}

enum Storage<'a> {
    Heap(Vec<u8>),
    Map(memmap2::Mmap),
    Borrowed(&'a [u8]),
}

impl<'a> Storage<'a> {
    fn bytes(&self) -> &[u8] {
        match self {
            Storage::Heap(v) => v,
            Storage::Map(m) => m,
            Storage::Borrowed(b) => b,
        }
    }
}

struct Input<'a> {
    label: String,
    command_line: bool,
    storage: Storage<'a>,
}

fn load_inputs(inputs: &[LinkInput]) -> Result<(Vec<Input<'_>>, Vec<ArchiveInput<'_>>, Vec<String>), LinkError> {
    let mut objects = Vec::new();
    let mut archives = Vec::new();
    let mut pending_libs = Vec::new();
    for input in inputs {
        match input {
            LinkInput::ObjectBytes(b) => objects.push(Input {
                label: format!("object{}", objects.len()),
                command_line: true,
                storage: Storage::Borrowed(b),
            }),
            LinkInput::ObjectPath(p) => {
                let file = std::fs::File::open(p).map_err(|e| LinkError::Io(format!("{}: {e}", p.display())))?;
                let map = unsafe { memmap2::Mmap::map(&file).map_err(|e| LinkError::Io(e.to_string()))? };
                objects.push(Input { label: p.display().to_string(), command_line: true, storage: Storage::Map(map) });
            }
            LinkInput::ArchiveBytes(b) => archives.push(ArchiveInput {
                label: format!("archive{}", archives.len()),
                storage: Storage::Borrowed(b),
            }),
            LinkInput::ArchiveRef(b) => archives.push(ArchiveInput {
                label: format!("archive{}", archives.len()),
                storage: Storage::Borrowed(b),
            }),
            LinkInput::ArchivePath(p) => {
                let file = std::fs::File::open(p).map_err(|e| LinkError::Io(format!("{}: {e}", p.display())))?;
                let map = unsafe { memmap2::Mmap::map(&file).map_err(|e| LinkError::Io(e.to_string()))? };
                archives.push(ArchiveInput { label: p.display().to_string(), storage: Storage::Map(map) });
            }
            LinkInput::Lib(name) => {
                pending_libs.push(name.clone());
            }
        }
    }
    Ok((objects, archives, pending_libs))
}

struct ArchiveInput<'a> {
    label: String,
    storage: Storage<'a>,
}

fn cached_system_dirs() -> Vec<PathBuf> {
    static DIRS: std::sync::OnceLock<Vec<PathBuf>> = std::sync::OnceLock::new();
    DIRS.get_or_init(|| {
        let mut dirs: Vec<PathBuf> = Vec::new();
        for triplet in ["x86_64-pc-linux-gnu", "x86_64-linux-gnu"] {
            let base = PathBuf::from(format!("/usr/lib/gcc/{triplet}"));
            let mut vers: Vec<PathBuf> = Vec::new();
            if let Ok(entries) = std::fs::read_dir(&base) {
                for entry in entries.flatten() {
                    let path = entry.path();
                    if path.is_dir() {
                        vers.push(path);
                    }
                }
            }
            vers.sort();
            vers.reverse();
            dirs.extend(vers);
        }
        for fixed in [
            "/usr/lib/x86_64-linux-gnu",
            "/usr/lib64",
            "/usr/lib",
            "/lib64",
            "/lib",
        ] {
            dirs.push(PathBuf::from(fixed));
        }
        dirs
    })
    .clone()
}

fn system_search_dirs() -> Vec<PathBuf> {
    let mut dirs: Vec<PathBuf> = Vec::new();
    if let Ok(extra) = std::env::var("RNX_LIB_DIRS") {
        for part in extra.split(':') {
            if !part.is_empty() {
                dirs.push(PathBuf::from(part));
            }
        }
    }
    dirs.extend(cached_system_dirs());
    dirs
}

pub fn system_file(name: &str) -> Result<PathBuf, LinkError> {
    for dir in system_search_dirs() {
        let cand = dir.join(name);
        if cand.is_file() {
            return Ok(cand);
        }
    }
    Err(LinkError::NoLinker(format!(
        "system file `{name}` not found in [{}]",
        system_search_dirs().iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
    )))
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum CrtKind {
    Static,
    DynPie,
    StaticPie,
}

fn crt_objects(crt: CrtKind) -> Result<Vec<Input<'static>>, LinkError> {
    let names: &[&str] = match crt {
        CrtKind::Static => &["crt1.o", "crti.o", "crtbeginT.o"],
        CrtKind::DynPie => &["Scrt1.o", "crti.o", "crtbeginS.o"],
        CrtKind::StaticPie => &["rcrt1.o", "crti.o", "crtbeginS.o"],
    };
    let mut out = Vec::new();
    for n in names {
        let path = system_file(n)?;
        let file = std::fs::File::open(&path).map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
        let map = unsafe { memmap2::Mmap::map(&file).map_err(|e| LinkError::Io(e.to_string()))? };
        out.push(Input { label: path.display().to_string(), command_line: true, storage: Storage::Map(map) });
    }
    Ok(out)
}

fn crt_end(crt: CrtKind) -> Result<Vec<Input<'static>>, LinkError> {
    let names: &[&str] = match crt {
        CrtKind::Static => &["crtend.o", "crtn.o"],
        CrtKind::DynPie | CrtKind::StaticPie => &["crtendS.o", "crtn.o"],
    };
    let mut out = Vec::new();
    for n in names {
        let path = system_file(n)?;
        let file = std::fs::File::open(&path).map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
        let map = unsafe { memmap2::Mmap::map(&file).map_err(|e| LinkError::Io(e.to_string()))? };
        out.push(Input { label: path.display().to_string(), command_line: true, storage: Storage::Map(map) });
    }
    Ok(out)
}

fn archive_file(file: &str) -> Result<Vec<ArchiveInput<'static>>, LinkError> {
    let path = system_file(file)?;
    let f = std::fs::File::open(&path).map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
    let map = unsafe { memmap2::Mmap::map(&f).map_err(|e| LinkError::Io(e.to_string()))? };
    Ok(vec![ArchiveInput { label: path.display().to_string(), storage: Storage::Map(map) }])
}

fn system_archive(name: &str) -> Result<Vec<ArchiveInput<'static>>, LinkError> {
    let file = match name {
        "c" | "pthread" | "dl" => "libc.a",
        "m" => "libm.a",
        _ => return Err(LinkError::Native(format!("unsupported system library `-l{name}` in static native link"))),
    };
    let path = system_file(file)?;
    load_archive_path(&path)
}

fn load_archive_path(path: &Path) -> Result<Vec<ArchiveInput<'static>>, LinkError> {
    let bytes = std::fs::read(path).map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
    if bytes.starts_with(b"!<arch>") {
        let f = std::fs::File::open(path).map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
        let map = unsafe { memmap2::Mmap::map(&f).map_err(|e| LinkError::Io(e.to_string()))? };
        return Ok(vec![ArchiveInput { label: path.display().to_string(), storage: Storage::Map(map) }]);
    }
    let text = std::str::from_utf8(&bytes).map_err(|_| LinkError::Native(format!("{}: not an archive or linker script", path.display())))?;
    let mut out = Vec::new();
    for member in script_members(text, path)? {
        if member.starts_with("-l") {
            out.extend(system_archive(member.trim_start_matches("-l"))?);
        } else {
            out.extend(load_archive_path(Path::new(&member))?);
        }
    }
    Ok(out)
}

fn script_members(text: &str, path: &Path) -> Result<Vec<String>, LinkError> {
    let mut uncommented = String::with_capacity(text.len());
    let chars: Vec<char> = text.chars().collect();
    let mut i = 0usize;
    while i < chars.len() {
        if chars[i] == '/' && chars.get(i + 1) == Some(&'*') {
            i += 2;
            while i + 1 < chars.len() && !(chars[i] == '*' && chars[i + 1] == '/') {
                i += 1;
            }
            i += 2;
        } else {
            uncommented.push(chars[i]);
            i += 1;
        }
    }
    let mut members = Vec::new();
    let mut rest = uncommented.as_str();
    while let Some(i) = rest.find("GROUP").or_else(|| rest.find("INPUT")) {
        let after = &rest[i..];
        let open = after.find('(').ok_or_else(|| LinkError::Native(format!("{}: bad linker script", path.display())))?;
        let close = after.find(')').ok_or_else(|| LinkError::Native(format!("{}: bad linker script", path.display())))?;
        if close < open {
            return Err(LinkError::Native(format!("{}: bad linker script", path.display())));
        }
        for tok in after[open + 1..close].split_whitespace() {
            let tok = tok.trim_matches(',');
            if tok == "AS_NEEDED" {
                continue;
            }
            if tok.is_empty() || tok == "(" || tok == ")" {
                continue;
            }
            members.push(tok.to_string());
        }
        rest = &after[close + 1..];
    }
    if members.is_empty() {
        return Err(LinkError::Native(format!("{}: unsupported linker script", path.display())));
    }
    Ok(members)
}

fn member_symbols(bytes: &[u8], label: &str) -> Result<Vec<String>, LinkError> {
    let obj = Object::parse(bytes, label)?;
    let mut out = Vec::new();
    for (i, sym) in obj.symbols.iter().enumerate() {
        if sym.bind != STB_GLOBAL && sym.bind != STB_WEAK {
            continue;
        }
        if sym.kind == STT_FILE {
            continue;
        }
        if sym.shndx == crate::core::obj::SHN_UNDEF || sym.shndx == crate::core::obj::SHN_COMMON {
            continue;
        }
        out.push(obj.symbol_name(i)?.to_string());
    }
    Ok(out)
}

const SYNTHETIC_WEAK: [&str; 19] = [
    "_GLOBAL_OFFSET_TABLE_",
    "__ehdr_start",
    "__preinit_array_start",
    "__preinit_array_end",
    "__init_array_start",
    "__init_array_end",
    "__fini_array_start",
    "__fini_array_end",
    "__rela_iplt_start",
    "__rela_iplt_end",
    "_DYNAMIC",
    "_end",
    "__end",
    "end",
    "__bss_start",
    "etext",
    "__etext",
    "edata",
    "__edata",
];

fn seed_synthetics(db: &mut SymDb) {
    for name in SYNTHETIC_WEAK {
        let _ = db.define(
            name,
            Def {
                obj: usize::MAX,
                sec: u32::MAX,
                value: 0,
                size: 0,
                bind: STB_WEAK,
                kind: 0,
                vis: 2,
                tls: false,
                ifunc: false,
                absolute: false,
            },
            "linker",
            &|_| "linker".to_string(),
        );
    }
}

pub fn native_link_static(inputs: &[LinkInput], output: &Path, opts: &StaticOpts) -> Result<NativeStats, LinkError> {
    static_link_inner(inputs, output, opts, CrtKind::DynPie)
}

pub fn native_link_fully_static(inputs: &[LinkInput], output: &Path, opts: &StaticOpts) -> Result<NativeStats, LinkError> {
    static_link_inner(inputs, output, opts, CrtKind::Static)
}

pub fn native_link_static_pie(inputs: &[LinkInput], output: &Path, opts: &StaticOpts) -> Result<NativeStats, LinkError> {
    static_link_inner(inputs, output, opts, CrtKind::StaticPie)
}

struct SharedSet {
    early: Vec<DynLib>,
    late: Vec<DynLib>,
    loader: Option<DynLib>,
}

fn loader_dyn_lib() -> Option<DynLib> {
    system_file("ld-linux-x86-64.so.2")
        .or_else(|_| system_file("ld-linux.so.2"))
        .ok()
        .and_then(|p| parse_dyn_lib(&p).ok())
}

fn static_link_inner(
    inputs: &[LinkInput],
    output: &Path,
    opts: &StaticOpts,
    crt: CrtKind,
) -> Result<NativeStats, LinkError> {
    if opts.target != "x86_64-unknown-linux-gnu" {
        return Err(LinkError::Native(format!(
            "native backend not yet implemented for target `{}`",
            opts.target
        )));
    }
    let fully_static = matches!(crt, CrtKind::Static | CrtKind::StaticPie);
    let pie = !matches!(crt, CrtKind::Static);
    let target = X86_64 { static_link: fully_static };
    let (mut objects, mut archives, pending_libs) = load_inputs(inputs)?;
    let mut shared = SharedSet { early: Vec::new(), late: Vec::new(), loader: loader_dyn_lib() };
    if fully_static {
        for name in ["libgcc.a", "libgcc_eh.a"] {
            archives.extend(archive_file(name)?);
        }
        let mut seen_files: BTreeSet<String> = BTreeSet::new();
        for lib in pending_libs.iter().map(String::as_str).chain(["c", "m"]) {
            let file = match lib {
                "c" | "pthread" | "dl" => "libc.a",
                "m" => "libm.a",
                _ => "",
            };
            if file.is_empty() {
                archives.extend(system_archive(lib)?);
                continue;
            }
            if seen_files.insert(file.to_string()) {
                archives.extend(system_archive(lib)?);
            }
        }
    } else {
        archives.extend(archive_file("libgcc.a")?);
        let mut seen_sys: BTreeSet<&'static str> = BTreeSet::new();
        let mut push_sys = |name: &'static str, shared: &mut SharedSet| -> Result<(), LinkError> {
            if seen_sys.insert(name) {
                let path = system_file(name)?;
                let lib = parse_dyn_lib(&path)?;
                if name == "libgcc_s.so.1" {
                    shared.late.push(lib);
                } else {
                    shared.early.push(lib);
                }
            }
            Ok(())
        };
        for lib in pending_libs.iter().map(String::as_str).chain(["c", "m"]) {
            match lib {
                "c" | "pthread" | "dl" => push_sys("libc.so.6", &mut shared)?,
                "m" => push_sys("libm.so.6", &mut shared)?,
                _ => {
                    let path = find_shlib(&[], lib)?;
                    if !shared.early.iter().any(|l| l.path == path) {
                        shared.early.push(parse_dyn_lib(&path)?);
                    }
                }
            }
        }
        push_sys("libgcc_s.so.1", &mut shared)?;
    }
    let mut prefixed = crt_objects(crt)?;
    prefixed.append(&mut objects);
    prefixed.extend(crt_end(crt)?);
    objects = prefixed;
    link_all(&mut objects, archives, output, opts.entry.clone(), opts.icf, opts.strip, &target, &shared, pie, crt == CrtKind::StaticPie)
}

#[allow(clippy::too_many_arguments)]
fn link_all(
    objects: &mut Vec<Input<'_>>,
    archives: Vec<ArchiveInput<'_>>,
    output: &Path,
    entry: String,
    icf: bool,
    strip: bool,
    target: &dyn Target,
    shared: &SharedSet,
    pie: bool,
    force_dynamic: bool,
) -> Result<NativeStats, LinkError> {
    let mut db = SymDb::new();
    seed_synthetics(&mut db);
    let mut winners: BTreeMap<String, usize> = BTreeMap::new();
    let mut obj_data = Vec::new();
    for (oi, input) in objects.iter().enumerate() {
        let obj = Object::parse(input.storage.bytes(), &input.label)?;
        if obj.machine != target.machine() {
            return Err(LinkError::Native(format!("{}: unsupported machine type {}", input.label, obj.machine)));
        }
        obj_data.push(graph::build_object(&obj, oi, &input.label, input.command_line, &mut db, &mut winners)?.0);
    }
    let mut parsed_archives: Vec<Archive> = Vec::new();
    for a in archives.iter() {
        parsed_archives.push(Archive::parse(a.storage.bytes(), &a.label)?);
    }
    tphase!("link-index", {
        for (ai, arch) in parsed_archives.iter_mut().enumerate() {
            let label = archives[ai].label.clone();
            arch.build_index(&|bytes, member_label| member_symbols(bytes, member_label).map_err(|e| {
                LinkError::Native(format!("{label}: {e}"))
            }))?;
        }
        Ok::<(), LinkError>(())
    })?;
    let mut pulled: Vec<BTreeSet<usize>> = parsed_archives.iter().map(|_| BTreeSet::new()).collect();
    tphase!("link-pull", {
        let mut pending: BTreeSet<String> = db.unresolved().into_iter().map(|(name, _)| name).collect();
        while let Some(name) = pending.iter().next().cloned() {
            pending.remove(&name);
            if !db.is_unresolved(&name) {
                continue;
            }
            let mut found = None;
            for (ai, arch) in parsed_archives.iter().enumerate() {
                if let Some(m) = arch.providers(&name).and_then(|v| v.iter().find(|&&m| !pulled[ai].contains(&m)).copied()) {
                    found = Some((ai, m));
                    break;
                }
            }
            let (ai, m) = match found {
                Some(f) => f,
                None => continue,
            };
            pulled[ai].insert(m);
            let label = format!("{}({})", archives[ai].label, parsed_archives[ai].member_name(m));
            let bytes = parsed_archives[ai].member_bytes(m)?.to_vec();
            let oi = objects.len();
            let obj = Object::parse(&bytes, &label)?;
            if obj.machine != target.machine() {
                return Err(LinkError::Native(format!("{label}: unsupported machine type {}", obj.machine)));
            }
            let (data, defined, referenced) = graph::build_object(&obj, oi, &label, false, &mut db, &mut winners)?;
            for d in defined {
                pending.remove(&d);
            }
            for r in referenced {
                if db.is_unresolved(&r) {
                    pending.insert(r);
                }
            }
            objects.push(Input { label, command_line: false, storage: Storage::Heap(bytes) });
            obj_data.push(data);
        }
        Ok::<(), LinkError>(())
    })?;
    let parsed: Vec<Object> = tphase!("link-parse", {
        use rayon::prelude::*;
        objects
            .par_iter()
            .map(|input| Object::parse(input.storage.bytes(), &input.label))
            .collect::<Result<Vec<_>, LinkError>>()
    })?;
    let mut dyn_syms_map: BTreeMap<String, DynSym> = BTreeMap::new();
    if !shared.early.is_empty() || !shared.late.is_empty() {
        let mut prov: BTreeMap<&str, &DynLib> = BTreeMap::new();
        for lib in shared.early.iter().chain(shared.late.iter()) {
            for name in lib.syms.keys() {
                prov.entry(name.as_str()).or_insert(lib);
            }
        }
        for (name, e) in db.entries() {
            if e.def.is_some() || e.refs.is_empty() {
                continue;
            }
            if let Some(lib) = prov.get(name.as_str()) {
                let func = lib.syms.get(name.as_str()).map(|x| x.func).unwrap_or(false);
                dyn_syms_map.insert(name.clone(), DynSym { func, lib: lib.filename.clone() });
            }
        }
    }
    let mut g = tphase!("link-resolve", graph::mark(&parsed, obj_data, &db, &dyn_syms_map, &entry)?);
    let folded_before: usize = g.kept.iter().map(|k| k.iter().filter(|&&b| b).count()).sum();
    if icf {
        tphase!("link-icf", graph::icf_fold(&mut g, &parsed)?);
    }
    graph::collect_tls(&mut g, &parsed);
    graph::collect_commons(&mut g, &db);
    tphase!("link-eh", graph::compute_eh_plan(&mut g, &parsed)?);
    let relax = force_dynamic && pie && shared.early.is_empty() && shared.late.is_empty();
    tphase!("link-plan", graph::plan_slots(&mut g, &parsed, &db, &dyn_syms_map, target, relax)?);
    let folded_after: usize = g.kept.iter().map(|k| k.iter().filter(|&&b| b).count()).sum();
    let mut dyn_info = if shared.early.is_empty() && shared.late.is_empty() {
        let mut info = DynInfo::empty();
        info.force_dynamic = force_dynamic;
        info
    } else {
        tphase!("link-dyninfo", build_dyn_info(&g, &db, &shared.early, &shared.late, shared.loader.as_ref(), None, target)?)
    };
    if force_dynamic && pie {
        dyn_info.osabi = 3;
    }
    if dyn_info.interp.is_some() || dyn_info.force_dynamic {
        if db.get("_init").and_then(|e| e.def.as_ref()).is_some_and(|d| d.obj != usize::MAX) {
            dyn_info.init_addr = 1;
        }
        if db.get("_fini").and_then(|e| e.def.as_ref()).is_some_and(|d| d.obj != usize::MAX) {
            dyn_info.fini_addr = 1;
        }
    }
    let pie_targets = if pie { tphase!("link-pie", pie_targets(&parsed, &g, &db, target)?) } else { Vec::new() };
    let synth = synth_list(&g, &dyn_info, target, pie_targets.len(), strip);
    let base = if pie { 0 } else { layout::BASE_ADDR };
    let mut layout = tphase!("link-layout", layout::layout(&mut g, &parsed, &synth, base)?);
    if dyn_info.interp.is_none() {
        for (i, slot) in g.got.iter().enumerate() {
            if !slot.ifunc {
                continue;
            }
            let name = match &slot.key {
                graph::GotKey::Named(n) => n.clone(),
                _ => return Err(LinkError::Native("anonymous ifunc got slot".to_string())),
            };
            let e = db.get(&name).ok_or_else(|| LinkError::Native(format!("ifunc `{name}` undefined")))?;
            let d = e.def.as_ref().ok_or_else(|| LinkError::Native(format!("ifunc `{name}` undefined")))?;
            let mut id = (d.obj, d.sec);
            while let Some(&w) = g.icf_redirect.get(&id) {
                id = w;
            }
            let resolver = if g.objs[id.0].classes[id.1 as usize] == graph::SecClass::EhFrame {
                0
            } else if is_merged_init_fini_tls(&g, id) {
                merged_base_addr(&g, &layout, id) + g.merged_off.get(&id).copied().unwrap_or(0) + d.value
            } else {
                layout.sec_addr.get(&id).copied().unwrap_or(0) + d.value
            };
            dyn_info.dyn_rel.push((layout.got_addr + i as u64 * 8, target.rel_irelative(), 0, resolver as i64));
        }
    }
    if dyn_info.interp.is_some() {
        tphase!("link-dynrel", fill_dyn_relocs(&g, &layout, &db, &mut dyn_info, target)?);
    }
    if pie {
        let mut entries = tphase!("link-pierel", pie_entries(&pie_targets, &parsed, &g, &layout, &db, target)?);
        dyn_info.dyn_rel.append(&mut entries);
        dyn_info.relacount = writer::order_rela_dyn(&mut dyn_info.dyn_rel, target.rel_relative());
    }
    let dynamic_section = dyn_info.interp.is_some() || dyn_info.force_dynamic;
    let iplt = (dyn_info.interp.is_none() && dyn_info.force_dynamic).then_some((0, 0));
    fill_synthetics(&mut layout, &g, &db, dynamic_section.then_some(SynthFill { dynamic: true, iplt }))?;
    if let Some(init) = db.get("_init").and_then(|e| e.def.as_ref()) {
        if init.obj != usize::MAX {
            dyn_info.init_addr = init_value(&g, &layout, init);
        }
    }
    if let Some(fini) = db.get("_fini").and_then(|e| e.def.as_ref()) {
        if fini.obj != usize::MAX {
            dyn_info.fini_addr = init_value(&g, &layout, fini);
        }
    }
    let symdata = tphase!("link-symtab", writer::build_symtab(&parsed, &g, &db, &layout, dyn_info.interp.is_none())?);
    let total = writer::table_layout(&layout, &symdata, strip).total;
    let is_exec = !pie;
    tphase!("link-write", write_file(output, total, |out| {
        writer::write_output(&parsed, &g, &db, &layout, target, &dyn_info, is_exec, &symdata, strip, out)
    })?);
    if strip {
        tphase!("link-buildid", patch_build_id(output, &layout)?);
    }
    Ok(NativeStats {
        output_size: total,
        kept_sections: folded_after,
        got_slots: g.got.len(),
        plt_entries: g.plt.len(),
        folded: folded_before.saturating_sub(folded_after),
    })
}

fn is_merged_init_fini_tls(g: &Graph, id: (usize, u32)) -> bool {
    crate::core::layout::is_merged(g.objs[id.0].classes[id.1 as usize])
}

fn merged_base_addr(g: &Graph, layout: &Layout, id: (usize, u32)) -> u64 {
    let synth = crate::core::layout::merged_synth_of(g.objs[id.0].classes[id.1 as usize]);
    layout.order.iter().find(|o| o.synth == synth).map(|o| o.addr).unwrap_or(0)
}

enum PieTarget {
    Sec { oi: usize, si: u32, ri: usize },
    Cie { ci: usize, ri: usize },
    Fde { key: (usize, u32), fde: usize, ri: usize },
    Got { idx: usize },
}

fn pie_wants(kind: u32, plan: Plan, t: &graph::TargetSym) -> bool {
    if !matches!(plan, Plan::Direct) {
        return false;
    }
    if kind != R_64 && kind != R_32 && kind != R_32S {
        return false;
    }
    if t.weak_zero || t.dynamic || t.tls {
        return false;
    }
    t.common || (t.obj != usize::MAX && t.sec != u32::MAX)
}

fn pie_check(
    parsed: &[Object],
    g: &Graph,
    target: &dyn Target,
    oi: usize,
    ri: usize,
) -> Result<Option<(u32, graph::TargetSym)>, LinkError> {
    let obj = &parsed[oi];
    let r = &obj.relas[ri];
    if r.kind != R_64 && r.kind != R_32 && r.kind != R_32S {
        return Ok(None);
    }
    let t = g.resolved[oi]
        .get(r.sym as usize)
        .and_then(|o| o.as_ref())
        .ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
    let plan = target.plan(r.kind, graph::sym_class(t)).map_err(|e| {
        LinkError::Native(format!("{}: {e}", g.objs[oi].label))
    })?;
    if pie_wants(r.kind, plan, t) {
        Ok(Some((r.kind, t.clone())))
    } else {
        Ok(None)
    }
}

fn pie_targets(parsed: &[Object], g: &Graph, db: &SymDb, target: &dyn Target) -> Result<Vec<PieTarget>, LinkError> {
    let mut out = Vec::new();
    for (oi, obj) in parsed.iter().enumerate() {
        for (si, class) in g.objs[oi].classes.iter().enumerate() {
            if !g.kept[oi][si] {
                continue;
            }
            let si = si as u32;
            if matches!(
                class,
                SecClass::Skip | SecClass::GnuStack | SecClass::NoteProperty | SecClass::EhFrame
            ) {
                continue;
            }
            if crate::core::layout::is_merged(*class)
                && !matches!(
                    class,
                    SecClass::InitArray | SecClass::FiniArray | SecClass::Init | SecClass::Fini | SecClass::Tdata
                )
            {
                continue;
            }
            for &ri in g.rela_idx[oi][si as usize].iter() {
                let r = &obj.relas[ri];
                if g.consumed_call.contains(&(oi, si, r.offset)) {
                    continue;
                }
                if pie_check(parsed, g, target, oi, ri)?.is_some() {
                    out.push(PieTarget::Sec { oi, si, ri });
                }
            }
        }
    }
    for (ci, cie) in g.eh.cies.iter().enumerate() {
        if !cie.live {
            continue;
        }
        let (foi, fsi, fstart) = cie.first;
        let fobj = &parsed[foi];
        let fend = fstart + cie.body.len();
        let relas = &g.rela_idx[foi][fsi as usize];
        let lo = relas.partition_point(|&i| (fobj.relas[i].offset as usize) < fstart);
        let hi = relas.partition_point(|&i| (fobj.relas[i].offset as usize) < fend);
        for &ri in relas[lo..hi].iter() {
            let r = &fobj.relas[ri];
            if g.eh.dead_rel.contains(&(foi, fsi, r.offset)) {
                continue;
            }
            if pie_check(parsed, g, target, foi, ri)?.is_some() {
                out.push(PieTarget::Cie { ci, ri });
            }
        }
    }
    let mut sec_ids: Vec<(usize, u32)> = g.eh.sections.keys().copied().collect();
    sec_ids.sort();
    for key in sec_ids {
        let (oi, si) = key;
        let obj = &parsed[oi];
        let sec = &g.eh.sections[&key];
        let relas = &g.rela_idx[oi][si as usize];
        for (fi, f) in sec.fdes.iter().enumerate() {
            let lo = relas.partition_point(|&i| (obj.relas[i].offset as usize) < f.start);
            let hi = relas.partition_point(|&i| (obj.relas[i].offset as usize) < f.end);
            for &ri in relas[lo..hi].iter() {
                let r = &obj.relas[ri];
                if g.eh.dead_rel.contains(&(oi, si, r.offset)) {
                    continue;
                }
                if pie_check(parsed, g, target, oi, ri)?.is_some() {
                    out.push(PieTarget::Fde { key, fde: fi, ri });
                }
            }
        }
    }
    for (idx, slot) in g.got.iter().enumerate() {
        if slot.dynamic || slot.ifunc {
            continue;
        }
        if let graph::GotKey::Named(name) = &slot.key {
            let missing = db.get(name).is_some_and(|e| e.def.is_none() && e.common_size == 0);
            if missing {
                continue;
            }
        }
        out.push(PieTarget::Got { idx });
    }
    Ok(out)
}

fn pie_baked(kind: u32, sym_addr: u64, addend: i64) -> Result<i64, LinkError> {
    let v = (sym_addr as i64).wrapping_add(addend) as u64;
    if kind == R_32 && v > 0xffff_ffff {
        return Err(LinkError::Native(format!("R_X86_64_32 overflow: {v:#x} exceeds 32 bits")));
    }
    if kind == R_32S && v != (v as i32 as i64) as u64 {
        return Err(LinkError::Native(format!(
            "relocation overflow: value {v:#x} does not fit 32 bits"
        )));
    }
    Ok(v as i64)
}

fn pie_entries(
    targets: &[PieTarget],
    parsed: &[Object],
    g: &Graph,
    layout: &Layout,
    db: &SymDb,
    target: &dyn Target,
) -> Result<Vec<(u64, u32, u32, i64)>, LinkError> {
    let rel = target.rel_relative();
    let mut out = Vec::with_capacity(targets.len());
    for t in targets {
        match t {
            PieTarget::Sec { oi, si, ri } => {
                let obj = &parsed[*oi];
                let r = &obj.relas[*ri];
                let sym = g.resolved[*oi]
                    .get(r.sym as usize)
                    .and_then(|o| o.as_ref())
                    .ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
                let addr = writer::target_address(g, layout, sym)?;
                let class = g.objs[*oi].classes[*si as usize];
                let place = if crate::core::layout::is_merged(class) {
                    let synth = crate::core::layout::merged_synth_of(class);
                    layout::synth_base(layout, synth) + g.merged_off[&(*oi, *si)] + r.offset
                } else {
                    layout.sec_addr[&(*oi, *si)] + r.offset
                };
                out.push((place, rel, 0, pie_baked(r.kind, addr, r.addend)?));
            }
            PieTarget::Cie { ci, ri } => {
                let cie = &g.eh.cies[*ci];
                let (foi, _, fstart) = cie.first;
                let fobj = &parsed[foi];
                let r = &fobj.relas[*ri];
                let sym = g.resolved[foi]
                    .get(r.sym as usize)
                    .and_then(|o| o.as_ref())
                    .ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
                let addr = writer::target_address(g, layout, sym)?;
                let place =
                    layout::synth_base(layout, SynthSec::MergedEh) + g.eh_cie_off[*ci] + (r.offset as usize - fstart) as u64;
                out.push((place, rel, 0, pie_baked(r.kind, addr, r.addend)?));
            }
            PieTarget::Fde { key, fde, ri } => {
                let (oi, _) = *key;
                let obj = &parsed[oi];
                let r = &obj.relas[*ri];
                let sym = g.resolved[oi]
                    .get(r.sym as usize)
                    .and_then(|o| o.as_ref())
                    .ok_or_else(|| LinkError::Native("unresolved relocation".to_string()))?;
                let addr = writer::target_address(g, layout, sym)?;
                let sec = &g.eh.sections[key];
                let f = &sec.fdes[*fde];
                let in_start = f.start;
                let place = layout::synth_base(layout, SynthSec::MergedEh)
                    + g.merged_off[key]
                    + f.out_off
                    + (r.offset as usize - in_start) as u64;
                out.push((place, rel, 0, pie_baked(r.kind, addr, r.addend)?));
            }
            PieTarget::Got { idx } => {
                let slot = &g.got[*idx];
                let value = writer::got_slot_value(g, layout, db, slot)?;
                out.push((layout.got_addr + *idx as u64 * 8, rel, 0, value as i64));
            }
        }
    }
    Ok(out)
}

fn init_value(g: &Graph, layout: &Layout, d: &Def) -> u64 {
    let mut id = (d.obj, d.sec);
    while let Some(&w) = g.icf_redirect.get(&id) {
        id = w;
    }
    layout.sec_addr.get(&id).copied().unwrap_or(0) + d.value
}

struct SynthFill {
    dynamic: bool,
    iplt: Option<(u64, u64)>,
}

fn fill_synthetics(layout: &mut Layout, _g: &Graph, db: &SymDb, extra: Option<SynthFill>) -> Result<(), LinkError> {
    let find = |synth: SynthSec| layout.order.iter().find(|o| o.synth == synth).map(|o| (o.addr, o.size));
    let referenced = |name: &str| db.get(name).is_some_and(|e| !e.refs.is_empty());
    if referenced("_GLOBAL_OFFSET_TABLE_") && layout.got_addr != 0 {
        let v = if layout.got_plt_addr != 0 { layout.got_plt_addr } else { layout.got_addr };
        layout.synth_addr.insert("_GLOBAL_OFFSET_TABLE_".to_string(), v);
    }
    layout.synth_addr.insert("__ehdr_start".to_string(), layout.base);
    if let Some((addr, _)) = find(SynthSec::MergedInitArray) {
        if referenced("__init_array_start") {
            layout.synth_addr.insert("__init_array_start".to_string(), addr);
        }
        if referenced("__init_array_end") {
            let size = find(SynthSec::MergedInitArray).map(|(_, s)| s).unwrap_or(0);
            layout.synth_addr.insert("__init_array_end".to_string(), addr + size);
        }
    } else {
        if referenced("__init_array_start") {
            layout.synth_addr.insert("__init_array_start".to_string(), 0);
        }
        if referenced("__init_array_end") {
            layout.synth_addr.insert("__init_array_end".to_string(), 0);
        }
    }
    if let Some((addr, _)) = find(SynthSec::MergedFiniArray) {
        if referenced("__fini_array_start") {
            layout.synth_addr.insert("__fini_array_start".to_string(), addr);
        }
        if referenced("__fini_array_end") {
            let size = find(SynthSec::MergedFiniArray).map(|(_, s)| s).unwrap_or(0);
            layout.synth_addr.insert("__fini_array_end".to_string(), addr + size);
        }
    }
    if referenced("__preinit_array_start") {
        layout.synth_addr.insert("__preinit_array_start".to_string(), layout.base);
    }
    if referenced("__preinit_array_end") {
        layout.synth_addr.insert("__preinit_array_end".to_string(), layout.base);
    }
    if find(SynthSec::RelaDyn).is_some_and(|(_, s)| s > 0) {
        if let Some((addr, size)) = find(SynthSec::RelaDyn) {
            let (start, end) = extra.as_ref().and_then(|e| e.iplt).unwrap_or((addr, addr + size));
            if referenced("__rela_iplt_start") {
                layout.synth_addr.insert("__rela_iplt_start".to_string(), start);
            }
            if referenced("__rela_iplt_end") {
                layout.synth_addr.insert("__rela_iplt_end".to_string(), end);
            }
        }
    }
    let text_end = layout.order.iter().filter(|o| o.seg == SegClass::RX).map(|o| o.addr + o.size).max().unwrap_or(0);
    let data_end = layout.order.iter().filter(|o| o.seg == SegClass::RW).map(|o| o.addr + o.size).max().unwrap_or(0);
    let bss_start = layout
        .order
        .iter()
        .filter(|o| {
            o.seg == SegClass::RW
                && (o.out_name == ".bss" || o.synth == SynthSec::MergedTbss)
        })
        .map(|o| o.addr)
        .min()
        .unwrap_or(data_end);
    let bss_end = layout.segments.iter().filter(|s| s.seg == SegClass::RW).map(|s| s.vaddr + s.memsz).max().unwrap_or(data_end);
    for (name, val) in [
        ("_end", bss_end),
        ("__end", bss_end),
        ("end", bss_end),
        ("__bss_start", bss_start),
        ("etext", text_end),
        ("__etext", text_end),
        ("edata", data_end),
        ("__edata", data_end),
    ] {
        if referenced(name) {
            layout.synth_addr.insert(name.to_string(), val);
        }
    }
    if extra.is_some_and(|e| e.dynamic) {
        if let Some((addr, _)) = find(SynthSec::Dynamic) {
            if referenced("_DYNAMIC") {
                layout.synth_addr.insert("_DYNAMIC".to_string(), addr);
            }
        }
    }
    Ok(())
}

fn synth_list(g: &Graph, dyn_info: &DynInfo, target: &dyn Target, pie_rels: usize, strip: bool) -> Vec<SynthItem> {
    let _ = target;
    let mut out = Vec::new();
    if strip {
        out.push(SynthItem { synth: SynthSec::BuildId, name: ".note.gnu.build-id", size: 36, align: 4, seg: SegClass::R });
    }
    let got_plt_count = if dyn_info.interp.is_some() {
        3 + g.plt.iter().filter(|s| !s.ifunc).count()
    } else {
        0
    };
    let plt_count = g.plt.iter().filter(|s| !s.ifunc).count();
    let iplt_count = g.plt.iter().filter(|s| s.ifunc).count();
    let rela_dyn_count = g.got.iter().filter(|s| s.ifunc || s.dynamic).count() + pie_rels;
    let rela_plt_count = if dyn_info.interp.is_some() { g.plt.len() } else { 0 };
    let dynamic_section = dyn_info.interp.is_some() || dyn_info.force_dynamic;
    if dyn_info.interp.is_some() {
        out.push(SynthItem { synth: SynthSec::Interp, name: ".interp", size: 32, align: 1, seg: SegClass::R });
    }
    if rela_dyn_count > 0 {
        out.push(SynthItem {
            synth: SynthSec::RelaDyn,
            name: ".rela.dyn",
            size: rela_dyn_count as u64 * 24,
            align: 8,
            seg: SegClass::R,
        });
    }
    if rela_plt_count > 0 {
        out.push(SynthItem {
            synth: SynthSec::RelaPlt,
            name: ".rela.plt",
            size: rela_plt_count as u64 * 24,
            align: 8,
            seg: SegClass::R,
        });
    }
    if !dyn_info.dynsym_names.is_empty() {
        out.push(SynthItem {
            synth: SynthSec::DynSym,
            name: ".dynsym",
            size: dyn_info.dynsym_names.len() as u64 * 24,
            align: 8,
            seg: SegClass::R,
        });
        out.push(SynthItem {
            synth: SynthSec::DynStr,
            name: ".dynstr",
            size: dyn_info.dynstr_bytes.len() as u64,
            align: 1,
            seg: SegClass::R,
        });
        out.push(SynthItem {
            synth: SynthSec::Hash,
            name: ".hash",
            size: dyn_info.hash_bytes.len() as u64,
            align: 8,
            seg: SegClass::R,
        });
        if !dyn_info.versym.is_empty() {
            out.push(SynthItem { synth: SynthSec::GnuVersion, name: ".gnu.version", size: dyn_info.versym.len() as u64 * 2, align: 2, seg: SegClass::R });
            out.push(SynthItem {
                synth: SynthSec::GnuVersionR,
                name: ".gnu.version_r",
                size: dyn_info.verneed.len() as u64,
                align: 4,
                seg: SegClass::R,
            });
        }
    }
    if plt_count > 0 {
        out.push(SynthItem {
            synth: SynthSec::Plt,
            name: ".plt",
            size: 16 + plt_count as u64 * 16,
            align: 16,
            seg: SegClass::RX,
        });
    }
    if iplt_count > 0 {
        out.push(SynthItem {
            synth: SynthSec::IPlt,
            name: ".iplt",
            size: iplt_count as u64 * 16,
            align: 16,
            seg: SegClass::RX,
        });
    }
    if !g.got.is_empty() {
        out.push(SynthItem { synth: SynthSec::Got, name: ".got", size: g.got.len() as u64 * 8, align: 8, seg: SegClass::Relro });
    }
    if dynamic_section {
        let has_init = g.objs.iter().enumerate().any(|(oi, _)| {
            g.objs[oi].classes.iter().zip(g.kept[oi].iter()).any(|(&c, &k)| k && c == graph::SecClass::InitArray)
        });
        let has_fini = g.objs.iter().enumerate().any(|(oi, _)| {
            g.objs[oi].classes.iter().zip(g.kept[oi].iter()).any(|(&c, &k)| k && c == graph::SecClass::FiniArray)
        });
        let mut sized = DynInfo::empty();
        sized.needed = dyn_info.needed.clone();
        if rela_plt_count > 0 {
            sized.plt_rel.push((0, 0, 0, 0));
        }
        if rela_dyn_count > 0 {
            sized.dyn_rel.push((0, 0, 0, 0));
        }
        if rela_dyn_count > 0 && pie_rels > 0 {
            sized.relacount = 1;
        }
        if !dyn_info.versym.is_empty() {
            sized.versym.push(0);
        }
        if dyn_info.init_addr != 0 {
            sized.init_addr = 1;
        }
        if dyn_info.fini_addr != 0 {
            sized.fini_addr = 1;
        }
        if dyn_info.runpath.is_some() {
            sized.runpath = Some(String::new());
        }
        let count = writer::dynamic_count(&sized, has_init, has_fini);
        out.push(SynthItem { synth: SynthSec::Dynamic, name: ".dynamic", size: count as u64 * 16, align: 8, seg: SegClass::Relro });
    }
    if got_plt_count > 0 {
        out.push(SynthItem {
            synth: SynthSec::GotPlt,
            name: ".got.plt",
            size: got_plt_count as u64 * 8,
            align: 8,
            seg: SegClass::RW,
        });
    }
    out
}

fn write_file(output: &Path, total: u64, fill: impl FnOnce(&mut [u8]) -> Result<writer::ShdrTable, LinkError>) -> Result<(), LinkError> {
    if total == 0 {
        return Err(LinkError::Native("empty link output".to_string()));
    }
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(true)
        .open(output)
        .map_err(|e| LinkError::Io(format!("{}: {e}", output.display())))?;
    file.set_len(total).map_err(|e| LinkError::Io(e.to_string()))?;
    let mut map = unsafe { memmap2::MmapMut::map_mut(&file).map_err(|e| LinkError::Io(e.to_string()))? };
    fill(&mut map)?;
    map.flush().map_err(|e| LinkError::Io(e.to_string()))?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        std::fs::set_permissions(output, std::fs::Permissions::from_mode(0o755)).map_err(|e| LinkError::Io(e.to_string()))?;
    }
    Ok(())
}

fn patch_build_id(output: &Path, layout: &Layout) -> Result<(), LinkError> {
    let at = layout
        .order
        .iter()
        .find(|o| o.synth == SynthSec::BuildId)
        .map(|o| o.offset as usize)
        .ok_or_else(|| LinkError::Native("stripped output has no .note.gnu.build-id".to_string()))?;
    let file = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(output)
        .map_err(|e| LinkError::Io(format!("{}: {e}", output.display())))?;
    let mut map = unsafe { memmap2::MmapMut::map_mut(&file).map_err(|e| LinkError::Io(e.to_string()))? };
    let desc = map
        .get_mut(at + 16..at + 36)
        .ok_or_else(|| LinkError::Native("build-id note out of range".to_string()))?;
    desc.fill(0);
    let id = writer::sha1(&map);
    map.get_mut(at + 16..at + 36)
        .ok_or_else(|| LinkError::Native("build-id note out of range".to_string()))?
        .copy_from_slice(&id);
    map.flush().map_err(|e| LinkError::Io(e.to_string()))?;
    Ok(())
}

pub struct DynLib {
    pub filename: String,
    pub path: PathBuf,
    pub syms: BTreeMap<String, DynExport>,
    pub vernames: BTreeMap<u16, (String, String)>,
}

#[derive(Clone, Debug)]
pub struct DynExport {
    pub func: bool,
    pub weak: bool,
    pub ver: u16,
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

fn parse_dyn_lib(path: &Path) -> Result<DynLib, LinkError> {
    let file = std::fs::File::open(path).map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
    let map = unsafe { memmap2::Mmap::map(&file).map_err(|e| LinkError::Io(e.to_string()))? };
    let d = &map[..];
    let err = |m: &str| LinkError::Native(format!("{}: {m}", path.display()));
    if d.len() < 64 || &d[0..4] != b"\x7fELF" || d[4] != 2 || d[5] != 1 {
        return Err(err("not a 64-bit little-endian ELF shared object"));
    }
    if u16_at(d, 16).ok_or_else(|| err("truncated header"))? != 3 {
        return Err(err("not a shared object"));
    }
    let shoff = u64_at(d, 40).ok_or_else(|| err("truncated header"))? as usize;
    let shentsize = u16_at(d, 58).ok_or_else(|| err("truncated header"))? as usize;
    let shnum = u16_at(d, 60).ok_or_else(|| err("truncated header"))? as usize;
    let shstrndx = u16_at(d, 62).ok_or_else(|| err("truncated header"))? as usize;
    if shoff == 0 || shnum == 0 {
        return Err(err("stripped shared object without section headers"));
    }
    let mut shs = Vec::with_capacity(shnum);
    for i in 0..shnum {
        let o = shoff + i * shentsize;
        shs.push((
            u32_at(d, o).ok_or_else(|| err("truncated section"))?,
            u32_at(d, o + 4).ok_or_else(|| err("truncated section"))?,
            u64_at(d, o + 24).ok_or_else(|| err("truncated section"))?,
            u64_at(d, o + 32).ok_or_else(|| err("truncated section"))?,
            u32_at(d, o + 40).ok_or_else(|| err("truncated section"))?,
            u32_at(d, o + 44).ok_or_else(|| err("truncated section"))?,
            u64_at(d, o + 56).ok_or_else(|| err("truncated section"))?,
        ));
    }
    let shstr = &shs[shstrndx];
    let getname = |off: u32| -> String {
        let s = shstr.2 as usize + off as usize;
        let e = d[s..].iter().position(|&b| b == 0).map(|p| s + p).unwrap_or(d.len());
        String::from_utf8_lossy(&d[s..e]).into_owned()
    };
    let find = |want: &str| -> Option<(u64, u64, u32, u32)> {
        shs.iter().find(|s| getname(s.0) == want).map(|s| (s.2, s.3, s.4, s.5))
    };
    let (dynsym_off, dynsym_size, _, _) = find(".dynsym").ok_or_else(|| err("no .dynsym"))?;
    let (dynstr_off, dynstr_size, _, _) = find(".dynstr").ok_or_else(|| err("no .dynstr"))?;
    let dynstr = d.get(dynstr_off as usize..dynstr_off as usize + dynstr_size as usize).ok_or_else(|| err("bad .dynstr"))?;
    let str_at = |off: u32| -> String {
        let s = off as usize;
        let e = dynstr[s..].iter().position(|&b| b == 0).map(|p| s + p).unwrap_or(dynstr.len());
        String::from_utf8_lossy(&dynstr[s..e]).into_owned()
    };
    let nsym = dynsym_size as usize / 24;
    let versym = find(".gnu.version").map(|v| v.0).unwrap_or(0);
    let mut syms = BTreeMap::new();
    for i in 0..nsym {
        let o = dynsym_off as usize + i * 24;
        let st_name = u32_at(d, o).ok_or_else(|| err("bad .dynsym"))?;
        let bind = d[o + 4] >> 4;
        let kind = d[o + 4] & 15;
        let shndx = u16_at(d, o + 6).ok_or_else(|| err("bad .dynsym"))?;
        if shndx == 0 {
            continue;
        }
        if bind != 1 && bind != 2 {
            continue;
        }
        if kind != 2 && kind != 1 && kind != 10 && kind != 0 {
            continue;
        }
        let full = str_at(st_name);
        let name = full.split('@').next().unwrap_or("").to_string();
        if name.is_empty() {
            continue;
        }
        let ver = if versym != 0 {
            u16_at(d, versym as usize + i * 2).unwrap_or(0) & 0x7fff
        } else {
            0
        };
        let exp = DynExport { func: kind == 2 || kind == 10, weak: bind == 2, ver };
        if !syms.contains_key(&name) {
            syms.insert(name, exp);
        }
    }
    let mut vernames: BTreeMap<u16, (String, String)> = BTreeMap::new();
    if let Some((verdef_off, verdef_size, _, _)) = find(".gnu.version_d") {
        let mut off = verdef_off as usize;
        let end = off + verdef_size as usize;
        while off + 20 <= end && off + 20 <= d.len() {
            let ndx = u16_at(d, off + 4).unwrap_or(0);
            let cnt = u16_at(d, off + 6).unwrap_or(0);
            let aux = u32_at(d, off + 12).unwrap_or(0);
            let next = u32_at(d, off + 16).unwrap_or(0);
            if cnt > 0 && ndx > 1 {
                let name = str_at(u32_at(d, off + aux as usize).unwrap_or(0));
                if name.starts_with("GLIBC") || name.starts_with("GCC") {
                    vernames.insert(ndx, (String::new(), name));
                }
            }
            if next == 0 {
                break;
            }
            off += next as usize;
        }
    }
    let filename = path.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    Ok(DynLib { filename, path: path.to_path_buf(), syms, vernames })
}

fn find_shlib(lib_dirs: &[PathBuf], name: &str) -> Result<PathBuf, LinkError> {
    for dir in lib_dirs {
        for candidate in [format!("lib{name}.so"), format!("lib{name}.so.1"), format!("lib{name}.so.6"), format!("{name}.so")] {
            let p = dir.join(&candidate);
            if p.is_file() {
                return Ok(p);
            }
        }
        let direct = dir.join(name);
        if direct.is_file() {
            return Ok(direct);
        }
    }
    for candidate in [format!("lib{name}.so.1"), format!("lib{name}.so.6"), format!("lib{name}.so")] {
        if let Ok(p) = system_file(&candidate) {
            return Ok(p);
        }
    }
    Err(LinkError::Native(format!(
        "shared library `{name}` not found in {lib_dirs:?} or [{}]",
        system_search_dirs().iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(", ")
    )))
}

pub fn native_link_dynamic(
    objects: &[LinkInput],
    output: &Path,
    opts: &DynamicOpts,
) -> Result<NativeStats, LinkError> {
    if opts.target != "x86_64-unknown-linux-gnu" {
        return Err(LinkError::Native(format!(
            "native backend not yet implemented for target `{}`",
            opts.target
        )));
    }
    let target = X86_64 { static_link: false };
    let (mut cmd_objects, archives, pending) = load_inputs(objects)?;
    let _ = pending;
    let mut prefixed = crt_objects(CrtKind::DynPie)?;
    prefixed.append(&mut cmd_objects);
    prefixed.extend(crt_end(CrtKind::DynPie)?);
    let mut objects = prefixed;
    let mut lib_files: Vec<PathBuf> = Vec::new();
    for name in opts.libs.iter() {
        lib_files.push(find_shlib(&opts.lib_dirs, name)?);
    }
    let mut dyn_libs: Vec<DynLib> = Vec::new();
    for path in lib_files.iter() {
        dyn_libs.push(parse_dyn_lib(path)?);
    }
    let mut sys_libs: Vec<DynLib> = Vec::new();
    let mut sys_paths: Vec<PathBuf> = Vec::new();
    for candidate in ["libc.so.6", "libm.so.6"] {
        if let Ok(p) = system_file(candidate) {
            if !sys_paths.contains(&p) && !lib_files.contains(&p) {
                sys_paths.push(p);
            }
        }
    }
    for path in sys_paths.iter() {
        sys_libs.push(parse_dyn_lib(path)?);
    }
    let interp_lib: Option<DynLib> = loader_dyn_lib();
    let mut dyn_prov: BTreeMap<&str, &DynLib> = BTreeMap::new();
    for lib in dyn_libs.iter().chain(sys_libs.iter()) {
        for name in lib.syms.keys() {
            dyn_prov.entry(name.as_str()).or_insert(lib);
        }
    }
    let mut db = SymDb::new();
    seed_synthetics(&mut db);
    let mut winners: BTreeMap<String, usize> = BTreeMap::new();
    let mut obj_data = Vec::new();
    for (oi, input) in objects.iter().enumerate() {
        let obj = Object::parse(input.storage.bytes(), &input.label)?;
        if obj.machine != target.machine() {
            return Err(LinkError::Native(format!("{}: unsupported machine type {}", input.label, obj.machine)));
        }
        obj_data.push(graph::build_object(&obj, oi, &input.label, input.command_line, &mut db, &mut winners)?.0);
    }
    let mut parsed_archives: Vec<Archive> = Vec::new();
    for a in archives.iter() {
        parsed_archives.push(Archive::parse(a.storage.bytes(), &a.label)?);
    }
    for (ai, arch) in parsed_archives.iter_mut().enumerate() {
        let label = archives[ai].label.clone();
        arch.build_index(&|bytes, member_label| member_symbols(bytes, member_label).map_err(|e| {
            LinkError::Native(format!("{label}: {e}"))
        }))?;
    }
    let mut pulled: Vec<BTreeSet<usize>> = parsed_archives.iter().map(|_| BTreeSet::new()).collect();
    let mut pending: BTreeSet<String> = db.unresolved().into_iter().map(|(name, _)| name).collect();
    while let Some(name) = pending.iter().next().cloned() {
        pending.remove(&name);
        if !db.is_unresolved(&name) {
            continue;
        }
        let mut found = None;
        for (ai, arch) in parsed_archives.iter().enumerate() {
            if let Some(m) = arch.providers(&name).and_then(|v| v.iter().find(|&&m| !pulled[ai].contains(&m)).copied()) {
                found = Some((ai, m));
                break;
            }
        }
        let (ai, m) = match found {
            Some(f) => f,
            None => continue,
        };
        pulled[ai].insert(m);
        let label = format!("{}({})", archives[ai].label, parsed_archives[ai].member_name(m));
        let bytes = parsed_archives[ai].member_bytes(m)?.to_vec();
        let oi = objects.len();
        let obj = Object::parse(&bytes, &label)?;
        if obj.machine != target.machine() {
            return Err(LinkError::Native(format!("{label}: unsupported machine type {}", obj.machine)));
        }
        let (data, defined, referenced) = graph::build_object(&obj, oi, &label, false, &mut db, &mut winners)?;
        for d in defined {
            pending.remove(&d);
        }
        for r in referenced {
            if db.is_unresolved(&r) {
                pending.insert(r);
            }
        }
        objects.push(Input { label, command_line: false, storage: Storage::Heap(bytes) });
        obj_data.push(data);
    }
    for (name, objs) in db.unresolved() {
        if dyn_prov.contains_key(name.as_str()) {
            continue;
        }
        let mut from: Vec<String> = objs.iter().map(|&o| objects[o].label.clone()).collect();
        from.sort();
        from.dedup();
        return Err(LinkError::Native(format!("undefined symbol `{name}` referenced by {}", from.join(", "))));
    }
    let mut dyn_syms_map: BTreeMap<String, DynSym> = BTreeMap::new();
    for (name, e) in db.entries() {
        if e.def.is_some() || e.refs.is_empty() {
            continue;
        }
        if let Some(lib) = dyn_prov.get(name.as_str()) {
            if let Some(x) = lib.syms.get(name.as_str()) {
                dyn_syms_map.insert(name.clone(), DynSym { func: x.func, lib: lib.filename.clone() });
            }
        }
    }
    let parsed: Vec<Object> = {
        use rayon::prelude::*;
        objects
            .par_iter()
            .map(|input| Object::parse(input.storage.bytes(), &input.label))
            .collect::<Result<Vec<_>, _>>()?
    };
    let mut g = graph::mark(&parsed, obj_data, &db, &dyn_syms_map, &opts.entry)?;
    if opts.icf {
        graph::icf_fold(&mut g, &parsed)?;
    }
    graph::plan_slots(&mut g, &parsed, &db, &dyn_syms_map, &target, false)?;
    graph::collect_tls(&mut g, &parsed);
    if g.tls.size > 0 {
        return Err(LinkError::Native("TLS sections in dynamic native link are not implemented yet".to_string()));
    }
    graph::collect_commons(&mut g, &db);
    graph::compute_eh_plan(&mut g, &parsed)?;
    let mut dyn_info = build_dyn_info(&g, &db, &dyn_libs, &sys_libs, interp_lib.as_ref(), opts.runpath.as_deref(), &target)?;
    if db.get("_init").and_then(|e| e.def.as_ref()).is_some_and(|d| d.obj != usize::MAX) {
        dyn_info.init_addr = 1;
    }
    if db.get("_fini").and_then(|e| e.def.as_ref()).is_some_and(|d| d.obj != usize::MAX) {
        dyn_info.fini_addr = 1;
    }
    let pie_targets = pie_targets(&parsed, &g, &db, &target)?;
    let synth = synth_list(&g, &dyn_info, &target, pie_targets.len(), opts.strip);
    let mut layout = layout::layout(&mut g, &parsed, &synth, 0)?;
    fill_synthetics(&mut layout, &g, &db, Some(SynthFill { dynamic: true, iplt: None }))?;
    if let Some((addr, _)) = layout.order.iter().find(|o| o.synth == layout::SynthSec::Dynamic).map(|o| (o.addr, o.size)) {
        layout.synth_addr.insert("_DYNAMIC".to_string(), addr);
    }
    if let Some(init) = db.get("_init").and_then(|e| e.def.as_ref()) {
        if init.obj != usize::MAX {
            dyn_info.init_addr = init_value(&g, &layout, init);
        }
    }
    if let Some(fini) = db.get("_fini").and_then(|e| e.def.as_ref()) {
        if fini.obj != usize::MAX {
            dyn_info.fini_addr = init_value(&g, &layout, fini);
        }
    }
    fill_dyn_relocs(&mut g, &layout, &db, &mut dyn_info, &target)?;
    {
        let mut entries = pie_entries(&pie_targets, &parsed, &g, &layout, &db, &target)?;
        dyn_info.dyn_rel.append(&mut entries);
        dyn_info.relacount = writer::order_rela_dyn(&mut dyn_info.dyn_rel, target.rel_relative());
    }
    let symdata = writer::build_symtab(&parsed, &g, &db, &layout, false)?;
    let total = writer::table_layout(&layout, &symdata, opts.strip).total;
    write_file(output, total, |out| {
        writer::write_output(&parsed, &g, &db, &layout, &target, &dyn_info, false, &symdata, opts.strip, out)
    })?;
    if opts.strip {
        patch_build_id(output, &layout)?;
    }
    Ok(NativeStats {
        output_size: total,
        kept_sections: g.kept.iter().map(|k| k.iter().filter(|&&b| b).count()).sum(),
        got_slots: g.got.len(),
        plt_entries: g.plt.len(),
        folded: 0,
    })
}

fn build_dyn_info(
    g: &Graph,
    db: &SymDb,
    dyn_libs: &[DynLib],
    sys_libs: &[DynLib],
    interp: Option<&DynLib>,
    runpath: Option<&str>,
    _target: &dyn Target,
) -> Result<DynInfo, LinkError> {
    let mut needed: Vec<String> = Vec::new();
    for lib in dyn_libs.iter() {
        if !needed.contains(&lib.filename) {
            needed.push(lib.filename.clone());
        }
    }
    let mut sys_used: BTreeSet<String> = BTreeSet::new();
    let mut dyn_names: BTreeSet<String> = BTreeSet::new();
    for (name, e) in db.entries() {
        if e.def.is_some() || e.refs.is_empty() {
            continue;
        }
        if !g.used_names.contains(name.as_str()) {
            continue;
        }
        dyn_names.insert(name.clone());
        for lib in sys_libs.iter() {
            if lib.syms.contains_key(name) {
                sys_used.insert(lib.filename.clone());
            }
        }
    }
    for f in sys_used.into_iter() {
        if !needed.contains(&f) {
            needed.push(f);
        }
    }
    let dynsym_names: Vec<String> = dyn_names.into_iter().collect();
    let mut dynsym_bind = Vec::with_capacity(dynsym_names.len() + 1);
    let mut dynsym_kind = Vec::with_capacity(dynsym_names.len() + 1);
    dynsym_bind.push(0);
    dynsym_kind.push(0);
    let mut strtab = crate::core::string::StrTab::new();
    for lib in needed.iter() {
        strtab.add(lib);
    }
    if let Some(rp) = runpath {
        strtab.add(rp);
    }
    let mut name_offs = vec![0u32];
    for name in dynsym_names.iter() {
        let e = db.get(name).unwrap();
        let strong = e.refs.iter().any(|&(_, s)| s);
        dynsym_bind.push(if strong { 1 } else { 2 });
        let func = dyn_libs
            .iter()
            .chain(sys_libs.iter())
            .chain(interp.into_iter())
            .flat_map(|l| l.syms.get(name))
            .next()
            .map(|x| x.func)
            .unwrap_or(false);
        dynsym_kind.push(if func { 2 } else { 0 });
        name_offs.push(strtab.add(name));
    }
    let mut ver_pairs: BTreeSet<(String, String)> = BTreeSet::new();
    let mut sym_ver: Vec<(String, String)> = Vec::with_capacity(dynsym_names.len() + 1);
    sym_ver.push((String::new(), String::new()));
    for name in dynsym_names.iter() {
        let mut found = (String::new(), String::new());
        for lib in dyn_libs.iter().chain(sys_libs.iter()) {
            if let Some(exp) = lib.syms.get(name) {
                if exp.ver > 1 {
                    if let Some((_, vname)) = lib.vernames.get(&exp.ver) {
                        found = (lib.filename.clone(), vname.clone());
                        break;
                    }
                }
            }
        }
        if !found.0.is_empty() {
            ver_pairs.insert(found.clone());
        }
        sym_ver.push(found);
    }
    let ver_list: Vec<(String, String)> = ver_pairs.into_iter().collect();
    let mut versym = vec![0u16];
    for (file, vname) in sym_ver.iter().skip(1) {
        if file.is_empty() {
            versym.push(0);
        } else {
            let idx = ver_list.iter().position(|v| v.0 == *file && v.1 == *vname).unwrap_or(0);
            versym.push((idx + 2) as u16);
        }
    }
    let mut verneed = Vec::new();
    let mut verneed_num = 0usize;
    if !ver_list.is_empty() {
        let mut files: BTreeMap<String, Vec<(String, u16)>> = BTreeMap::new();
        for (file, vname) in ver_list.iter() {
            let idx = ver_list.iter().position(|v| v.0 == *file && v.1 == *vname).unwrap_or(0) as u16 + 2;
            files.entry(file.clone()).or_default().push((vname.clone(), idx));
        }
        let mut file_blobs: Vec<(Vec<u8>, Vec<Vec<u8>>)> = Vec::new();
        for (file, vnames) in files.iter() {
            verneed_num += 1;
            let file_off = strtab.add(file);
            let mut auxes: Vec<Vec<u8>> = Vec::new();
            for (vname, idx) in vnames.iter() {
                let name_off = strtab.add(vname);
                let mut aux = vec![0u8; 16];
                aux[0..4].copy_from_slice(&crate::target::elf::elf_hash(vname.as_bytes()).to_le_bytes());
                aux[6..8].copy_from_slice(&idx.to_le_bytes());
                aux[8..12].copy_from_slice(&name_off.to_le_bytes());
                auxes.push(aux);
            }
            let mut need = vec![0u8; 16];
            need[0..2].copy_from_slice(&1u16.to_le_bytes());
            need[2..4].copy_from_slice(&(vnames.len() as u16).to_le_bytes());
            need[4..8].copy_from_slice(&file_off.to_le_bytes());
            file_blobs.push((need, auxes));
        }
        let mut aux_off = file_blobs.len() * 16;
        let mut blob: Vec<u8> = Vec::new();
        for (i, (need, auxes)) in file_blobs.iter().enumerate() {
            let mut n = need.clone();
            n[8..12].copy_from_slice(&((aux_off - i * 16) as u32).to_le_bytes());
            n[12..16].copy_from_slice(&if i + 1 < file_blobs.len() { 16u32 } else { 0 }.to_le_bytes());
            blob.extend_from_slice(&n);
            aux_off += auxes.len() * 16;
        }
        for (_, auxes) in file_blobs.iter() {
            for (j, aux) in auxes.iter().enumerate() {
                let mut a = aux.clone();
                a[12..16].copy_from_slice(&if j + 1 < auxes.len() { 16u32 } else { 0 }.to_le_bytes());
                blob.extend_from_slice(&a);
            }
        }
        verneed = blob;
    }
    let dynstr_bytes = strtab.bytes().to_vec();
    let hash_bytes = crate::target::elf::sysv_hash(
        &std::iter::once(Vec::new()).chain(dynsym_names.iter().map(|n| n.as_bytes().to_vec())).collect::<Vec<_>>(),
    );
    let mut info = DynInfo::empty();
    info.needed = needed;
    info.dynsym_names = std::iter::once(String::new()).chain(dynsym_names).collect();
    info.dynsym_bind = dynsym_bind;
    info.dynsym_kind = dynsym_kind;
    info.dynsym_name_off = name_offs;
    info.dynstr_bytes = dynstr_bytes;
    info.hash_bytes = hash_bytes;
    info.versym = if ver_list.is_empty() { Vec::new() } else { versym };
    info.verneed = verneed;
    info.verneed_num = verneed_num;
    info.interp = Some("/lib64/ld-linux-x86-64.so.2".to_string());
    info.runpath = runpath.map(str::to_string);
    Ok(info)
}

fn fill_dyn_relocs(
    g: &Graph,
    layout: &Layout,
    db: &SymDb,
    dyn_info: &mut DynInfo,
    target: &dyn Target,
) -> Result<(), LinkError> {
    let index_of = |name: &str| -> Result<u32, LinkError> {
        dyn_info
            .dynsym_names
            .iter()
            .position(|n| n == name)
            .map(|i| i as u32)
            .ok_or_else(|| LinkError::Native(format!("dynamic symbol `{name}` missing from .dynsym")))
    };
    for (i, slot) in g.got.iter().enumerate() {
        let addr = layout.got_addr + i as u64 * 8;
        match &slot.key {
            graph::GotKey::Named(name) => {
                if slot.ifunc {
                    dyn_info.dyn_rel.push((addr, target.rel_irelative(), index_of(name)?, 0));
                } else if slot.dynamic {
                    dyn_info.dyn_rel.push((addr, target.rel_globdat(), index_of(name)?, 0));
                }
            }
            graph::GotKey::Local(_, _, _) => {}
        }
        let _ = db;
    }
    let mut n = 0u32;
    for slot in g.plt.iter() {
        if slot.ifunc {
            let gi = g.plt_got.get(&g.plt_index[&slot.name]).copied().ok_or_else(|| {
                LinkError::Native(format!("ifunc plt `{}` without got slot", slot.name))
            })?;
            let addr = layout.got_addr + gi as u64 * 8;
            dyn_info.plt_rel.push((addr, target.rel_irelative(), index_of(&slot.name)?, 0));
        } else {
            let addr = layout.got_plt_addr + (3 + n) as u64 * 8;
            dyn_info.plt_rel.push((addr, target.rel_jmpslot(), index_of(&slot.name)?, 0));
            n += 1;
        }
    }
    dyn_info.dyn_rel.sort();
    dyn_info.plt_rel.sort();
    Ok(())
}
