use crate::delta::{self, Delta};
use crate::watcher::Watcher;
use cranelift::jit::Jit;
use diagnostics::Diagnostic;
use frontend::ast as A;
use frontend::semantic::ScopeCache;
use frontend::unsafe_taint::TaintCache;
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::{Duration, Instant};

pub enum DevEvent {
    Swapped { names: Vec<String>, elapsed_ms: u128 },
    Restarted { reason: String },
    Broken { count: usize },
}

#[derive(Debug, Clone, serde::Serialize)]
pub struct HotReport {
    pub status: String,
    pub swapped_functions: Vec<String>,
    pub compile_time_ms: u128,
    pub diagnostics: Vec<String>,
}

pub enum DevCommand {
    Rebuild { file: Option<PathBuf>, reply: mpsc::Sender<HotReport> },
}

fn render_plain(entry_file: &Path, errs: &[Diagnostic]) -> Vec<String> {
    let theme = diagnostics::theme::AuraTheme::plain();
    crate::report::render_compile_errors(&theme, entry_file, errs)
        .to_string()
        .lines()
        .filter(|l| !l.trim().is_empty())
        .map(|l| l.to_string())
        .collect()
}

pub fn hot_report(event: &DevEvent, diags: &[Diagnostic], entry_file: &Path, elapsed_ms: u128) -> HotReport {
    match event {
        DevEvent::Swapped { names, .. } => HotReport {
            status: if names.is_empty() { "no_change".to_string() } else { "swapped".to_string() },
            swapped_functions: names.clone(),
            compile_time_ms: elapsed_ms,
            diagnostics: Vec::new(),
        },
        DevEvent::Restarted { reason } => HotReport {
            status: "restarted".to_string(),
            swapped_functions: Vec::new(),
            compile_time_ms: elapsed_ms,
            diagnostics: vec![format!("structural change in {reason}; session restarted")],
        },
        DevEvent::Broken { .. } => HotReport {
            status: "error".to_string(),
            swapped_functions: Vec::new(),
            compile_time_ms: elapsed_ms,
            diagnostics: render_plain(entry_file, diags),
        },
    }
}

const DEV_SPARE_SLOTS: usize = 4096;

enum DevUpdate {
    Swap {
        module: A::Module,
        lowered: std::sync::Arc<lir::instr::Module>,
        names: Vec<String>,
        elapsed_ms: u128,
        detected: Instant,
    },
    Restart {
        reason: String,
        module: A::Module,
        lowered: std::sync::Arc<lir::instr::Module>,
    },
    Broken {
        count: usize,
    },
}

struct DevRestart {
    reason: String,
    module: A::Module,
    lowered: std::sync::Arc<lir::instr::Module>,
}

struct DevState {
    jit: Jit,
    module: A::Module,
    entry_file: PathBuf,
    entry: String,
    entry_done: bool,
    updates: mpsc::Receiver<DevUpdate>,
    pending_restart: Option<DevRestart>,
}

thread_local! {
    static ACTIVE: std::cell::RefCell<Option<std::sync::Arc<std::sync::Mutex<DevState>>>> =
        const { std::cell::RefCell::new(None) };
}

unsafe extern "C" fn dev_install_callback() -> i64 {
    ACTIVE.with(|slot| {
        let state = slot.borrow().clone();
        let Some(state) = state else {
            return 0;
        };
        let mut state = state.lock().unwrap_or_else(|e| e.into_inner());
        state.drain()
    })
}

impl DevState {
    fn apply_swap(
        &mut self,
        module: &A::Module,
        lowered: &lir::instr::Module,
        names: &[String],
    ) -> Result<(Vec<String>, u128), Diagnostic> {
        let started = Instant::now();
        let mut done = Vec::new();
        for name in names {
            let Some(slot) = self.jit.slot_of(name) else {
                continue;
            };
            self.jit
                .hot_swap_function(slot, lowered)
                .map_err(|e| Diagnostic::new(diagnostics::Code::E108, format!("swap `{name}`: {e:?}")))?;
            done.push(name.clone());
        }
        self.module = module.clone();
        if std::env::var_os("RNX_DEV_TRACE").is_some() {
            eprintln!(
                "[rnx dev] install {}: {}ms",
                done.join(", "),
                started.elapsed().as_millis()
            );
        }
        Ok((done, started.elapsed().as_millis()))
    }

    /// Drain queued updates while the app thread is at a safe point.
    fn drain(&mut self) -> i64 {
        let mut applied = false;
        loop {
            let update = match self.updates.try_recv() {
                Ok(update) => update,
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => break,
            };
            match update {
                DevUpdate::Swap { module, lowered, names, elapsed_ms, detected } => {
                    match self.apply_swap(&module, &lowered, &names) {
                        Ok((done, install_ms)) => {
                            if !done.is_empty() {
                                eprintln!(
                                    "[rnx dev] hot swapped {} (build {elapsed_ms}ms, install {install_ms}ms, detect->apply {}ms)",
                                    done.join(", "),
                                    detected.elapsed().as_millis()
                                );
                                applied = true;
                            }
                        }
                        Err(e) => {
                            let reason = format!("swap failed: {e:?}");
                            let theme = diagnostics::theme::AuraTheme::active();
                            eprint!(
                                "{}",
                                crate::report::render_compile_errors(&theme, &self.entry_file, &[e])
                            );
                            self.pending_restart = Some(DevRestart {
                                reason,
                                module,
                                lowered,
                            });
                            runtime::native::dev::request_restart();
                            return 2;
                        }
                    }
                }
                DevUpdate::Restart { reason, module, lowered } => {
                    self.pending_restart = Some(DevRestart { reason, module, lowered });
                    runtime::native::dev::request_restart();
                    return 2;
                }
                DevUpdate::Broken { count } => {
                    eprintln!("[rnx dev] {count} error(s); keeping last valid build");
                }
            }
        }
        if applied { 1 } else { 0 }
    }
}

pub struct DevRunner {
    entry_file: PathBuf,
    entry: String,
    no_rerun: bool,
    state: std::sync::Arc<std::sync::Mutex<DevState>>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
    compiler: Option<std::thread::JoinHandle<()>>,
}

fn dev_timing(label: &str, step: &mut Instant) {
    if std::env::var_os("RNX_DEV_TRACE").is_some() {
        eprintln!("[rnx dev] build {label}: {}ms", step.elapsed().as_millis());
        *step = Instant::now();
    }
}

struct BuiltModule {
    graph: frontend::modules::ModuleGraph,
    module: A::Module,
    files: Vec<PathBuf>,
}

fn build_module(path: &Path, reuse_deps: bool) -> Result<BuiltModule, Vec<Diagnostic>> {
    let mut step = Instant::now();
    let graph = if reuse_deps {
        frontend::modules::ModuleGraph::build_collecting_reuse_deps(path)?
    } else {
        frontend::modules::ModuleGraph::build_collecting(path)?
    };
    dev_timing("graph", &mut step);
    if std::env::var_os("RNX_DEV_TRACE").is_some() {
        eprintln!("[rnx dev] graph files: {}", graph.files.len());
    }
    let mut module = graph.resolve().map_err(|e| vec![e])?;
    dev_timing("resolve", &mut step);
    let files: Vec<PathBuf> = graph.files.iter().map(|f| f.path.clone()).collect();
    frontend::harness::strip_tests(&mut module);
    frontend::harness::strip_benches(&mut module);
    dev_timing("strip", &mut step);
    let errors = frontend::desugar::desugar(&mut module);
    dev_timing("desugar", &mut step);
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok(BuiltModule { graph, module, files })
}

fn check_built(
    built: &BuiltModule,
    path: &Path,
    mut taint: Option<(&mut TaintCache, Option<&std::collections::BTreeSet<String>>)>,
    mut scope: Option<(&mut ScopeCache, Option<&std::collections::BTreeSet<String>>)>,
) -> Vec<Diagnostic> {
    let mut step = Instant::now();
    let decl_files = frontend::modules::merged_decl_files(&built.graph);
    let surface = crate::cabi_surface_for(path);
    let diags = match taint.take() {
        Some((cache, changed)) => frontend::semantic::check_with_files_and_surface_cached(
            &built.module,
            &decl_files,
            surface.as_deref(),
            cache,
            scope
                .take()
                .map(|(scope, _)| scope)
                .expect("scope cache passed with taint cache"),
            changed,
        ),
        None => frontend::semantic::check_with_files_and_surface(
            &built.module,
            &decl_files,
            surface.as_deref(),
        ),
    };
    dev_timing("check", &mut step);
    let mut errors: Vec<Diagnostic> = diags
        .into_iter()
        .filter(|d| !d.code.is_warning())
        .collect();
    errors.extend(frontend::security::verify_runtime_imports(&built.graph));
    dev_timing("security", &mut step);
    errors
}

fn finish_lowered(
    mut lowered: lir::instr::Module,
    built: &BuiltModule,
    entry: &str,
    mut step: Instant,
) -> Result<lir::instr::Module, Vec<Diagnostic>> {
    // Dev builds must not inline: inlined callees bypass the dispatch table, so swaps would stay invisible.
    lir::opt::optimize_lir(&mut lowered, 0, entry);
    dev_timing("opt", &mut step);
    let jobs = std::env::var("RNX_JOBS")
        .ok()
        .and_then(|value| value.parse::<usize>().ok())
        .or_else(|| std::thread::available_parallelism().ok().map(|n| n.get()))
        .unwrap_or(1)
        .clamp(1, 8);
    let verrs = lir::verify::verify_parallel(&lowered, jobs);
    if !verrs.is_empty() {
        return Err(verrs);
    }
    dev_timing("verify", &mut step);
    let iso = built.graph.isolation_errors();
    if !iso.is_empty() {
        return Err(iso);
    }
    dev_timing("isolation", &mut step);
    Ok(lowered)
}

fn lower_built(built: &BuiltModule, entry: &str) -> Result<lir::instr::Module, Vec<Diagnostic>> {
    let mut step = Instant::now();
    let lowered = lir::lower::lower(&built.module).map_err(|e| vec![e])?;
    dev_timing("lower", &mut step);
    if std::env::var_os("RNX_DEV_TRACE").is_some() {
        eprintln!("[rnx dev] lowered functions: {}", lowered.functions.len());
    }
    finish_lowered(lowered, built, entry, step)
}

enum BodyRef<'a> {
    Fn(&'a A::FnBody),
    Block(&'a A::Block),
}

fn patch_expanded(
    expanded: &mut A::Module,
    new: &A::Module,
    changed: &std::collections::BTreeSet<String>,
) -> bool {
    let mut classes: std::collections::BTreeMap<&str, &A::Decl> = std::collections::BTreeMap::new();
    let mut fns: std::collections::BTreeMap<&str, &A::FnDecl> = std::collections::BTreeMap::new();
    for decl in &new.decls {
        match &decl.node {
            A::Decl::Fn(f) => {
                fns.insert(f.name.as_str(), f);
            }
            A::Decl::Class { name, .. } | A::Decl::Struct { name, .. } => {
                classes.insert(name.as_str(), &decl.node);
            }
            _ => {}
        }
    }
    let mut bodies: std::collections::BTreeMap<String, BodyRef<'_>> =
        std::collections::BTreeMap::new();
    for name in changed {
        if let Some((class, member)) = name.rsplit_once('.') {
            let Some(decl) = classes.get(class) else {
                return false;
            };
            let members = match decl {
                A::Decl::Class { extends, with, members, .. } => {
                    if extends.is_some() || !with.is_empty() {
                        return false;
                    }
                    members
                }
                A::Decl::Struct { members, .. } => members,
                _ => return false,
            };
            let found = members.iter().find_map(|m| match (&m.node, member) {
                (A::ClassMember::Method(f), _) if f.name == member => Some(BodyRef::Fn(&f.body)),
                (A::ClassMember::Init { body, .. }, "init") => Some(BodyRef::Block(body)),
                (A::ClassMember::Deinit(body), "deinit") => Some(BodyRef::Block(body)),
                (A::ClassMember::OnReload { body, .. }, "onReload") => Some(BodyRef::Block(body)),
                _ => None,
            });
            let Some(body) = found else {
                return false;
            };
            bodies.insert(name.clone(), body);
        } else {
            let Some(f) = fns.get(name.as_str()) else {
                return false;
            };
            bodies.insert(name.clone(), BodyRef::Fn(&f.body));
        }
    }
    let mut patched = 0usize;
    for decl in &mut expanded.decls {
        match &mut decl.node {
            A::Decl::Fn(f) => {
                if let Some(BodyRef::Fn(body)) = bodies.remove(&f.name) {
                    f.body = body.clone();
                    patched += 1;
                }
            }
            A::Decl::Class { name, members, .. } | A::Decl::Struct { name, members, .. } => {
                let prefix = format!("{name}.");
                if !changed.iter().any(|key| key.starts_with(&prefix)) {
                    continue;
                }
                for member in members {
                    match &mut member.node {
                        A::ClassMember::Method(f) => {
                            let key = format!("{name}.{}", f.name);
                            if let Some(BodyRef::Fn(body)) = bodies.remove(&key) {
                                f.body = body.clone();
                                patched += 1;
                            }
                        }
                        A::ClassMember::Init { body, .. } => {
                            let key = format!("{name}.init");
                            if let Some(BodyRef::Block(next)) = bodies.remove(&key) {
                                *body = next.clone();
                                patched += 1;
                            }
                        }
                        A::ClassMember::Deinit(body) => {
                            let key = format!("{name}.deinit");
                            if let Some(BodyRef::Block(next)) = bodies.remove(&key) {
                                *body = next.clone();
                                patched += 1;
                            }
                        }
                        A::ClassMember::OnReload { body, .. } => {
                            let key = format!("{name}.onReload");
                            if let Some(BodyRef::Block(next)) = bodies.remove(&key) {
                                *body = next.clone();
                                patched += 1;
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    patched == changed.len()
}


fn lower_expanded_plain(
    expanded: &A::Module,
    built: &BuiltModule,
    entry: &str,
) -> Result<lir::instr::Module, Vec<Diagnostic>> {
    let mut step = Instant::now();
    let lowered = lir::lower::lower_expanded(expanded).map_err(|e| vec![e])?;
    dev_timing("lower", &mut step);
    finish_lowered(lowered, built, entry, step)
}

fn lower_expanded_owned(
    expanded: &A::Module,
    built: &BuiltModule,
    entry: &str,
    prev: lir::instr::Module,
    changed: &std::collections::BTreeSet<String>,
) -> Result<lir::instr::Module, Vec<Diagnostic>> {
    let mut step = Instant::now();
    let lowered = lir::lower::lower_expanded_incremental(expanded, prev, changed)
        .map_err(|e| vec![e])?;
    dev_timing("lower", &mut step);
    finish_lowered(lowered, built, entry, step)
}

pub fn build_program(
    path: &Path,
    entry: &str,
) -> Result<(A::Module, lir::instr::Module, Vec<PathBuf>), Vec<Diagnostic>> {
    let built = build_module(path, false)?;
    let errors = check_built(&built, path, None, None);
    if !errors.is_empty() {
        return Err(errors);
    }
    let lowered = lower_built(&built, entry)?;
    Ok((built.module, lowered, built.files))
}

fn build_program_cached(
    path: &Path,
    entry: &str,
    taint: &mut TaintCache,
    scope: &mut ScopeCache,
) -> Result<(A::Module, lir::instr::Module, Vec<PathBuf>, A::Module), Vec<Diagnostic>> {
    let built = build_module(path, false)?;
    let errors = check_built(&built, path, Some((taint, None)), Some((scope, None)));
    if !errors.is_empty() {
        return Err(errors);
    }
    let expanded = lir::lower::expand(&built.module).map_err(|e| vec![e])?;
    let lowered = lower_expanded_plain(&expanded, &built, entry)?;
    Ok((built.module, lowered, built.files, expanded))
}

type FileStamp = (PathBuf, Option<(u128, u64)>);

fn snapshot(files: &[PathBuf]) -> Vec<FileStamp> {
    let mut out: Vec<FileStamp> = files
        .iter()
        .map(|path| {
            let stamp = std::fs::metadata(path).ok().and_then(|meta| {
                let ticks = meta
                    .modified()
                    .ok()?
                    .duration_since(std::time::UNIX_EPOCH)
                    .ok()?
                    .as_nanos();
                Some((ticks, meta.len()))
            });
            (path.clone(), stamp)
        })
        .collect();
    out.sort();
    out
}

fn compile_loop(
    entry_file: PathBuf,
    entry: String,
    mut files: Vec<PathBuf>,
    mut last: A::Module,
    mut expanded: A::Module,
    mut last_lir: Option<std::sync::Arc<lir::instr::Module>>,
    mut taint: TaintCache,
    mut scope: ScopeCache,
    updates: mpsc::Sender<DevUpdate>,
    stop: std::sync::Arc<std::sync::atomic::AtomicBool>,
) {
    let mut watcher = match Watcher::new(&files) {
        Ok(watcher) => watcher,
        Err(err) => {
            eprintln!("[rnx dev] {err}");
            return;
        }
    };
    let mut stamps = snapshot(&files);
    while !stop.load(std::sync::atomic::Ordering::Acquire) {
        // Notify is the fast path. The stat check catches saves that arrive
        // while a build is running, or that an editor performs as an atomic
        // rename the watcher does not report as a relevant path.
        let event = watcher.poll(Duration::from_millis(20));
        let current_stamps = snapshot(&files);
        let mut changed_paths: std::collections::BTreeSet<PathBuf> = std::collections::BTreeSet::new();
        if let Some(path) = event.as_ref() {
            changed_paths.insert(path.clone());
        }
        for (old, new) in stamps.iter().zip(current_stamps.iter()) {
            if old != new {
                changed_paths.insert(new.0.clone());
            }
        }
        if event.is_none() && current_stamps == stamps {
            continue;
        }
        let detected = Instant::now();
        if std::env::var_os("RNX_DEV_TRACE").is_some() {
            eprintln!("[rnx dev] change detected");
        }
        while watcher.poll(Duration::from_millis(5)).is_some() {}
        let started = Instant::now();
        if std::env::var_os("RNX_DEV_TRACE").is_some() {
            eprintln!("[rnx dev] build start");
        }
        let module = match build_module(&entry_file, true) {
            Ok(module) => module,
            Err(errs) => {
                let theme = diagnostics::theme::AuraTheme::active();
                eprint!(
                    "{}",
                    crate::report::render_compile_errors(&theme, &entry_file, &errs)
                );
                let _ = updates.send(DevUpdate::Broken { count: errs.len() });
                taint.clear();
                stamps = snapshot(&files);
                continue;
            }
        };
        let decl_files = frontend::modules::merged_decl_files(&module.graph);
        let changed_paths = (!changed_paths.is_empty()).then_some(&changed_paths);
        let delta_started = Instant::now();
        let delta = delta::classify(&last, &module.module, Some(&decl_files), changed_paths);
        if std::env::var_os("RNX_DEV_TRACE").is_some() {
            eprintln!("[rnx dev] delta: {}ms", delta_started.elapsed().as_millis());
        }
        let changed: Option<std::collections::BTreeSet<String>> = match &delta {
            Delta::Pure { swapped } => Some(swapped.iter().cloned().collect()),
            Delta::Structural { .. } => {
                taint.clear();
                scope.clear();
                None
            }
        };
        let errors = check_built(
            &module,
            &entry_file,
            Some((&mut taint, changed.as_ref())),
            Some((&mut scope, changed.as_ref())),
        );
        if !errors.is_empty() {
            let theme = diagnostics::theme::AuraTheme::active();
            eprint!(
                "{}",
                crate::report::render_compile_errors(&theme, &entry_file, &errors)
            );
            let _ = updates.send(DevUpdate::Broken { count: errors.len() });
            taint.clear();
            scope.clear();
            stamps = snapshot(&files);
            continue;
        }
        let patch_started = Instant::now();
        let mut owned = None;
        let mut patched = false;
        if let (Delta::Pure { .. }, Some(changed)) = (&delta, &changed)
            && patch_expanded(&mut expanded, &module.module, changed)
        {
            patched = true;
            if let Some(prev) = last_lir.take() {
                if let Ok(prev) = std::sync::Arc::try_unwrap(prev) {
                    owned = Some(prev);
                }
            }
        }
        if std::env::var_os("RNX_DEV_TRACE").is_some() {
            eprintln!("[rnx dev] patch: {}ms", patch_started.elapsed().as_millis());
        }
        let lowered = if patched {
            match (owned, &changed) {
                (Some(prev), Some(changed)) => {
                    lower_expanded_owned(&expanded, &module, &entry, prev, changed)
                }
                _ => lower_expanded_plain(&expanded, &module, &entry),
            }
        } else {
            match lir::lower::expand(&module.module) {
                Ok(next) => {
                    let result = lower_expanded_plain(&next, &module, &entry);
                    expanded = next;
                    result
                }
                Err(e) => Err(vec![e]),
            }
        };
        let lowered = match lowered {
            Ok(lowered) => lowered,
            Err(errs) => {
                let theme = diagnostics::theme::AuraTheme::active();
                eprint!(
                    "{}",
                    crate::report::render_compile_errors(&theme, &entry_file, &errs)
                );
                let _ = updates.send(DevUpdate::Broken { count: errs.len() });
                last_lir = None;
                if let Ok(valid) = lir::lower::expand(&last) {
                    expanded = valid;
                }
                taint.clear();
                scope.clear();
                stamps = snapshot(&files);
                continue;
            }
        };
        let elapsed_ms = started.elapsed().as_millis();
        let lowered = std::sync::Arc::new(lowered);
        match delta {
            Delta::Pure { swapped } => {
                let _ = updates.send(DevUpdate::Swap {
                    module: module.module.clone(),
                    lowered: lowered.clone(),
                    names: swapped,
                    elapsed_ms,
                    detected,
                });
            }
            Delta::Structural { name: reason } => {
                let _ = updates.send(DevUpdate::Restart {
                    reason,
                    module: module.module.clone(),
                    lowered: lowered.clone(),
                });
                runtime::native::dev::request_restart();
            }
        }
        last_lir = Some(lowered);
        last = module.module;
        files = module.files;
        stamps = snapshot(&files);
        let _ = watcher.set_files(&files);
    }
}

impl DevRunner {
    pub fn new(
        entry_file: &Path,
        entry: &str,
        no_rerun: bool,
        prefer: frontend::native::NativePrefer,
    ) -> Result<Self, Vec<Diagnostic>> {
        // The app loop blocks on host events when idle. A dev session needs
        // a bounded wait so the frame-boundary safepoint runs.
        unsafe {
            std::env::set_var("RNX_DEV", "1");
        }
        let mut taint = TaintCache::new();
        let mut scope = ScopeCache::new();
        let (module, lowered, files, expanded) =
            build_program_cached(entry_file, entry, &mut taint, &mut scope)?;
        let native_libs = lir::instr::native_libs(&lowered);
        crate::set_native_search_paths(
            &entry_file.to_string_lossy(),
            prefer,
            &native_libs,
        )
        .map_err(|e| vec![e])?;
        let jit = Jit::compile_hot_reserve(&lowered, DEV_SPARE_SLOTS).map_err(|e| vec![e])?;
        let (tx, rx) = mpsc::channel();
        let state = std::sync::Arc::new(std::sync::Mutex::new(DevState {
            jit,
            module: module.clone(),
            entry_file: entry_file.to_path_buf(),
            entry: entry.to_string(),
            entry_done: false,
            updates: rx,
            pending_restart: None,
        }));
        runtime::native::dev::clear_restart();
        runtime::native::dev::install(dev_install_callback);
        let stop = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let compiler = {
            let entry_file = entry_file.to_path_buf();
            let entry = entry.to_string();
            let stop = stop.clone();
            let last_lir = Some(std::sync::Arc::new(lowered));
            std::thread::spawn(move || {
                compile_loop(entry_file, entry, files, module, expanded, last_lir, taint, scope, tx, stop)
            })
        };
        Ok(DevRunner {
            entry_file: entry_file.to_path_buf(),
            entry: entry.to_string(),
            no_rerun,
            state,
            stop,
            compiler: Some(compiler),
        })
    }

    pub fn run_entry(&mut self) -> Result<i64, Diagnostic> {
        ACTIVE.with(|slot| *slot.borrow_mut() = Some(self.state.clone()));
        runtime::native::dev::install(dev_install_callback);
        let (addr, epoch) = {
            let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            state.jit.entry_addr(&state.entry, 0)?
        };
        // The Jit borrow is dropped before the call so the app loop can
        // install updates through the dev callback while this is running.
        let result = unsafe { Jit::invoke(addr, epoch, &[]) };
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry_done = true;
        Ok(result)
    }

    pub fn poll(&mut self, timeout: Duration) -> Option<DevEvent> {
        let deadline = Instant::now() + timeout;
        loop {
            let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
            if let Some(restart) = state.pending_restart.take() {
                drop(state);
                return Some(self.restart(restart.reason, restart.module, restart.lowered));
            }
            match state.updates.try_recv() {
                Ok(DevUpdate::Swap { module, lowered, names, elapsed_ms, detected }) => {
                    let result = state.apply_swap(&module, &lowered, &names);
                    drop(state);
                    return Some(match result {
                        Ok((names, install_ms)) => {
                            if std::env::var_os("RNX_DEV_TRACE").is_some() {
                                eprintln!(
                                    "[rnx dev] detect->apply {}ms",
                                    detected.elapsed().as_millis()
                                );
                            }
                            DevEvent::Swapped {
                                names,
                                elapsed_ms: elapsed_ms + install_ms,
                            }
                        }
                        Err(e) => {
                            let reason = format!("swap failed: {e:?}");
                            let theme = diagnostics::theme::AuraTheme::active();
                            eprint!(
                                "{}",
                                crate::report::render_compile_errors(&theme, &self.entry_file, &[e])
                            );
                            self.restart(reason, module, lowered)
                        }
                    });
                }
                Ok(DevUpdate::Restart { reason, module, lowered }) => {
                    drop(state);
                    return Some(self.restart(reason, module, lowered));
                }
                Ok(DevUpdate::Broken { count }) => {
                    drop(state);
                    return Some(DevEvent::Broken { count });
                }
                Err(mpsc::TryRecvError::Empty) => {
                    drop(state);
                }
                Err(mpsc::TryRecvError::Disconnected) => {
                    drop(state);
                    return None;
                }
            }
            if Instant::now() >= deadline {
                return None;
            }
            std::thread::sleep(Duration::from_millis(5));
        }
    }

    fn restart(&mut self, reason: String, module: A::Module, lowered: std::sync::Arc<lir::instr::Module>) -> DevEvent {
        match Jit::compile_hot_reserve(&lowered, DEV_SPARE_SLOTS) {
            Ok(jit) => {
                let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                state.jit = jit;
                state.module = module;
                state.entry_done = false;
                state.pending_restart = None;
                drop(state);
                runtime::native::dev::clear_restart();
                DevEvent::Restarted { reason }
            }
            Err(e) => {
                let theme = diagnostics::theme::AuraTheme::active();
                eprint!(
                    "{}",
                    crate::report::render_compile_errors(&theme, &self.entry_file, &[e])
                );
                DevEvent::Broken { count: 1 }
            }
        }
    }

    pub fn rebuild(&mut self) -> (DevEvent, Vec<Diagnostic>) {
        match build_program(&self.entry_file, &self.entry) {
            Err(errs) => {
                let count = errs.len();
                (DevEvent::Broken { count }, errs)
            }
            Ok((module, lowered, _files)) => {
                let delta = {
                    let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                    delta::classify(&state.module, &module, None, None)
                };
                match delta {
                    Delta::Pure { swapped } => {
                        let started = Instant::now();
                        let mut done = Vec::new();
                        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
                        for name in &swapped {
                            let Some(slot) = state.jit.slot_of(name) else {
                                continue;
                            };
                            match state.jit.hot_swap_function(slot, &lowered) {
                                Ok(()) => done.push(name.clone()),
                                Err(e) => {
                                    drop(state);
                                    eprintln!("[rnx dev] swap of `{name}` failed ({e:?}); restarting");
                                    return (
                                        self.restart(
                                            format!("swap of `{name}` failed"),
                                            module,
                                            std::sync::Arc::new(lowered),
                                        ),
                                        Vec::new(),
                                    );
                                }
                            }
                        }
                        state.module = module;
                        let elapsed_ms = started.elapsed().as_millis();
                        (DevEvent::Swapped { names: done, elapsed_ms }, Vec::new())
                    }
                    Delta::Structural { name: reason } => (
                        self.restart(reason, module, std::sync::Arc::new(lowered)),
                        Vec::new(),
                    ),
                }
            }
        }
    }

    pub fn should_rerun(&self) -> bool {
        let done = self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry_done;
        done && !self.no_rerun
    }

    pub fn entry_name(&self) -> &str {
        &self.entry
    }

    pub fn entry_file(&self) -> &Path {
        &self.entry_file
    }
}

impl Drop for DevRunner {
    fn drop(&mut self) {
        self.stop.store(true, std::sync::atomic::Ordering::Release);
        runtime::native::dev::uninstall();
        ACTIVE.with(|slot| *slot.borrow_mut() = None);
        if let Some(compiler) = self.compiler.take() {
            let _ = compiler.join();
        }
    }
}

fn rerun(runner: &mut DevRunner) {
    let entry = runner.entry_name().to_string();
    match runner.run_entry() {
        Ok(v) => eprintln!("[rnx dev] {entry}() -> {v}"),
        Err(e) => eprintln!("[rnx dev] entry failed: {e:?}"),
    }
}

pub fn run_watch_loop(mut runner: DevRunner, commands: Option<mpsc::Receiver<DevCommand>>) {
    eprintln!("[rnx dev] watching {}", runner.entry_file().display());
    rerun(&mut runner);
    loop {
        if let Some(rx) = commands.as_ref() {
            match rx.try_recv() {
                Ok(DevCommand::Rebuild { file, reply }) => {
                    if let Some(path) = file.as_ref() {
                        if !path.exists() {
                            let _ = reply.send(HotReport {
                                status: "error".to_string(),
                                swapped_functions: Vec::new(),
                                compile_time_ms: 0,
                                diagnostics: vec![format!("file `{}` does not exist", path.display())],
                            });
                            continue;
                        }
                    }
                    let started = Instant::now();
                    let entry_file = runner.entry_file().to_path_buf();
                    let (event, diags) = runner.rebuild();
                    let elapsed_ms = started.elapsed().as_millis();
                    if let DevEvent::Broken { .. } = event {
                        let theme = diagnostics::theme::AuraTheme::active();
                        eprint!(
                            "{}",
                            crate::report::render_compile_errors(&theme, &entry_file, &diags)
                        );
                    } else if runner.should_rerun() {
                        rerun(&mut runner);
                    }
                    let _ = reply.send(hot_report(&event, &diags, &entry_file, elapsed_ms));
                    continue;
                }
                Err(mpsc::TryRecvError::Disconnected) => break,
                Err(mpsc::TryRecvError::Empty) => {}
            }
        }
        match runner.poll(Duration::from_millis(50)) {
            None => {}
            Some(DevEvent::Swapped { names, elapsed_ms }) => {
                eprintln!("[rnx dev] hot swapped {} (build {elapsed_ms}ms)", names.join(", "));
                if runner.should_rerun() {
                    rerun(&mut runner);
                }
            }
            Some(DevEvent::Restarted { reason }) => {
                eprintln!("[rnx dev] structural change detected in {reason} -> restarting");
                rerun(&mut runner);
            }
            Some(DevEvent::Broken { .. }) => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;

    #[test]
    fn hot_report_maps_all_events() {
        let file = Path::new("main.rnx");
        let swapped = hot_report(
            &DevEvent::Swapped { names: vec!["f".to_string()], elapsed_ms: 3 },
            &[],
            file,
            3,
        );
        assert_eq!(swapped.status, "swapped");
        assert_eq!(swapped.swapped_functions, vec!["f".to_string()]);
        let idle = hot_report(&DevEvent::Swapped { names: vec![], elapsed_ms: 1 }, &[], file, 1);
        assert_eq!(idle.status, "no_change");
        let restarted = hot_report(&DevEvent::Restarted { reason: "g".to_string() }, &[], file, 5);
        assert_eq!(restarted.status, "restarted");
        assert!(restarted.diagnostics.join(" ").contains('g'));
        let broken = hot_report(&DevEvent::Broken { count: 2 }, &[], file, 7);
        assert_eq!(broken.status, "error");
    }
}
