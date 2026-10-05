pub mod args;
pub mod audit;
pub mod check;
pub mod delta;
pub mod dev;
pub mod fmt;
pub mod mcp;
pub mod styles;
pub mod lint;
pub mod lsp;
pub mod publish;
pub mod repl;
pub mod report;
pub mod telemetry;
pub mod test_runner;
pub mod unpack;
pub mod watcher;

use diagnostics::{Code, Diagnostic};
use runtime::machine::{ExecError, Machine};

pub use runtime::value::Value;
use std::io::Read;
use std::io::IsTerminal;

pub struct CheckReport {
    pub errors: Vec<Diagnostic>,
    pub warnings: Vec<Diagnostic>,
}

pub fn check_source(src: &str) -> CheckReport {
    let mut module = match frontend::modules::ModuleGraph::from_source(src) {
        Ok(m) => m,
        Err(e) => {
            return CheckReport {
                errors: vec![e],
                warnings: Vec::new(),
            };
        }
    };
    let mut errors: Vec<Diagnostic> = Vec::new();
    errors.extend(frontend::desugar::desugar(&mut module));
    match lir::lower::lower(&module) {
        Ok(mut lm) => {
            lir::opt::optimize_lir(&mut lm, 1, "Main");
            errors.extend(lir::verify::verify(&lm));
        }
        Err(e) => errors.push(e),
    }
    let mut warnings: Vec<Diagnostic> = Vec::new();
    let mut kept: Vec<Diagnostic> = Vec::new();
    for d in frontend::semantic::check(&module) {
        if d.code.is_warning() {
            warnings.push(d);
        } else {
            kept.push(d);
        }
    }
    errors.extend(kept);
    errors.sort_by(|a, b| a.code.as_str().cmp(b.code.as_str()).then(a.message.cmp(&b.message)));
    errors.dedup_by(|a, b| a.code == b.code && a.message == b.message && a.span == b.span);
    warnings.sort_by(|a, b| a.code.as_str().cmp(b.code.as_str()).then(a.message.cmp(&b.message)));
    CheckReport { errors, warnings }
}

pub fn is_ok(report: &CheckReport) -> bool {
    report.errors.is_empty()
}

#[derive(Debug)]
pub enum RunOutcome {
    Value(Value),
    Thrown(Value),
    Uncaught {
        message: String,
        span: Option<diagnostics::Span>,
        func: Option<String>,
    },
    Fatal {
        message: String,
        span: Option<diagnostics::Span>,
        func: Option<String>,
    },
    Compile(Vec<Diagnostic>),
}

pub struct RunResult {
    pub outcome: RunOutcome,
    pub output: Vec<String>,
}

fn record_opt_subs(profiler: &mut frontend::profiler::PassProfiler, t: &lir::opt::OptTimings) {
    for (name, dur) in [
        ("opt_inline", t.inline),
        ("opt_escape", t.escape),
        ("opt_sroa", t.sroa),
        ("opt_licm", t.licm),
        ("opt_bce", t.bce),
        ("opt_arc", t.arc),
        ("opt_tco", t.tco),
        ("opt_fixpoint", t.fixpoint),
        ("opt_sweep", t.sweep),
        ("opt_deadfn", t.deadfn),
    ] {
        if !dur.is_zero() {
            profiler.record_sub("opt_pipeline", name, dur);
        }
    }
}

fn record_codegen_subs(
    profiler: &mut frontend::profiler::PassProfiler,
    t: &cranelift::jit::CodegenTimings,
) {
    for (name, dur) in [
        ("codegen_init", t.init),
        ("codegen_declare", t.declare),
        ("codegen_define", t.define),
        ("codegen_clif_lower", t.clif_lower),
        ("codegen_clif_backend", t.clif_backend),
        ("codegen_finalize", t.finalize),
    ] {
        profiler.record_sub("codegen", name, dur);
    }
    profiler.note("functions", t.fn_count);
}

pub fn resolve_entry<'a>(lowered: &lir::instr::Module, requested: &'a str) -> &'a str {
    match requested {
        "Main" if lowered.fn_id("Main").is_none() && lowered.fn_id("main").is_some() => "main",
        "main" if lowered.fn_id("main").is_none() && lowered.fn_id("Main").is_some() => "Main",
        _ => requested,
    }
}

pub fn run_source(src: &str, entry: &str, args: Vec<Value>) -> RunResult {
    let report = check_source(src);
    if !report.errors.is_empty() {
        return RunResult {
            outcome: RunOutcome::Compile(report.errors),
            output: Vec::new(),
        };
    }
    let mut cmdline = vec!["<main>".to_string()];
    cmdline.extend(args.iter().map(|v| v.display()));
    runtime::native::rnx_set_args(&cmdline);
    let mut module = frontend::modules::ModuleGraph::from_source(src).expect("checked");
    frontend::desugar::desugar(&mut module);
    let mut lowered = lir::lower::lower(&module).expect("checked");
    let entry = resolve_entry(&lowered, entry);
    lir::opt::optimize_lir(&mut lowered, 1, entry);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(lowered));
    let mut machine = Machine::new(leaked);
    let outcome = match machine.call(entry, args) {
        Ok(v) => RunOutcome::Value(v),
        Err(ExecError::Throw(v)) => RunOutcome::Thrown(v),
        Err(ExecError::Fatal(m)) => RunOutcome::Fatal {
            message: m,
            span: machine.error_span,
            func: machine.error_func.clone(),
        },
    };
    RunResult {
        outcome,
        output: machine.output.clone(),
    }
}

pub fn explain(code: &str) -> Option<(String, String)> {
    let c = Code::from_str(code)?;
    let fix = match c {
        Code::E005 => "use `.` for namespacing and member access; write explicit type arguments as `f<T>(x)`",
        Code::E105 => "arrow expressions do not use `fn`. Use `(x) => x * 2` or declare a named function `fn mul(x) { ... }`",
        Code::E107 => "break the import cycle; move shared items to a leaf module",
        Code::E108 => "read the message span; the message names the specific cause",
        Code::E109 => "move the `await` into an `async fn` or remove it",
        Code::E110 => "use `for x in range` or `while`/`do-while`; strided loops use `(a..b).stride(n)`",
        Code::E111 => "use `await` or `promise.wait()` instead of calling `poll()` directly",
        Code::E112 => "move execution logic into a function or class method in the imported file",
        Code::E201 => "wrap the call in `unsafe { ... }` or mark the caller `unsafe fn`",
        Code::E202 => "wrap address-of and pointer arithmetic in `unsafe { ... }`",
        Code::E203 => "access the member from inside its own class",
        Code::E204 => "instantiate the class with `new Class(...)`",
        Code::E205 => "change the value to match the declared type, or declare the variable as `Any`",
        Code::E206 => "add an initializer with `= <expr>`",
        Code::E302 => "break the cycle with `GenRef` on a back edge",
        Code::E303 => "declare the variable with `let` or check the spelling",
        Code::E304 => "return a value matching the declared return type",
        Code::E305 => "convert explicitly with `.asFast()` or `.asStrict()`",
        Code::E402 => "align the file schema with the declared record type",
        Code::E501 => "pass --token <token> or set RNX_TOKEN in your environment",
        Code::S101 => "approve the capability and re-lock, or remove the offending use",
        Code::S102 => "narrow the package code or deliberately widen the `[permissions]` ceiling",
        Code::S201 => "pass the delegated value through untouched or request the ambient grant",
        Code::S301 => "never target `Project.config`, `Project.deplock`, `.git`, or cache dirs from dependency code",
        Code::S401 => "add a covering `sys:exec:<name>` grant with explicit approval",
        Code::S501 => "split the package into smaller modules; never skip the scan",
        Code::W104 => "capture with `fn decay(this)`",
        Code::W201 => "correct the spelling or remove the entry",
        Code::W204 => "replace `pub`/`public` with `export`",
        Code::W108 => "make one side `GenRef` or add `#[Allow(CyclicReference)]`",
        Code::W109 => "set the field to `null` in `deinit` or a `clear*`/`close*`/`reset*` method",
    };
    Some((c.title().to_string(), fix.to_string()))
}

pub fn read_input(path: Option<&str>) -> Result<String, String> {
    match path {
        Some(p) => std::fs::read_to_string(p).map_err(|e| format!("{p}: {e}")),
        None => {
            let mut s = String::new();
            std::io::stdin()
                .read_to_string(&mut s)
                .map_err(|e| e.to_string())?;
            Ok(s)
        }
    }
}

pub fn build_object(src: &str, entry: &str, release: bool) -> Result<Vec<u8>, Vec<Diagnostic>> {
    let report = check_source(src);
    if !report.errors.is_empty() {
        return Err(report.errors);
    }
    let mut module = frontend::modules::ModuleGraph::from_source(src).expect("checked");
    frontend::desugar::desugar(&mut module);
    let mut lowered = lir::lower::lower(&module).expect("checked");
    let entry = resolve_entry(&lowered, entry);
    lir::opt::optimize_lir(&mut lowered, 1, entry);
    let opt = if release {
        llvm::codegen::OptLevel::Release
    } else {
        llvm::codegen::OptLevel::Dev
    };
    llvm::codegen::emit_object(&lowered, "rnx_module", entry, opt, None).map_err(|e| vec![e])
}

pub struct FileProgram {
    pub lir: lir::instr::Module,
    pub warnings: Vec<diagnostics::Diagnostic>,
    pub profiler: frontend::profiler::PassProfiler,
    pub debug: Option<llvm::codegen::DebugInfo>,
    pub native_libs: Vec<String>,
}

#[derive(Clone, Debug, Default)]
pub struct CompileConfig {
    pub time_passes: bool,
    pub trace: Option<std::path::PathBuf>,
    pub perf_map: bool,
    pub debug: bool,
}

impl CompileConfig {
    fn profiler(&self) -> frontend::profiler::PassProfiler {
        frontend::profiler::PassProfiler::new(self.time_passes, self.trace.clone())
    }
}

pub fn write_perf_map(entries: &[(String, usize, usize)]) {
    let path = std::env::temp_dir().join(format!("perf-{}.map", std::process::id()));
    let mut text = String::new();
    for (name, addr, size) in entries {
        text.push_str(&format!("{addr:x} {size:x} {name}\n"));
    }
    match std::fs::OpenOptions::new().create(true).append(true).open(&path) {
        Ok(mut f) => {
            use std::io::Write;
            if let Err(e) = f.write_all(text.as_bytes()).and_then(|_| f.flush()) {
                eprintln!("warning: cannot write {}: {e}", path.display());
            } else {
                eprintln!("perf map: {}", path.display());
            }
        }
        Err(e) => eprintln!("warning: cannot write {}: {e}", path.display()),
    }
}

pub fn load_program(path: &str) -> Result<FileProgram, Vec<diagnostics::Diagnostic>> {
    load_program_opt(path, 1, "Main")
}

pub fn load_program_opt(
    path: &str,
    opt_level: u8,
    entry: &str,
) -> Result<FileProgram, Vec<diagnostics::Diagnostic>> {
    load_program_cfg(path, opt_level, entry, &CompileConfig::default())
}

pub fn load_program_cfg(
    path: &str,
    opt_level: u8,
    entry: &str,
    cfg: &CompileConfig,
) -> Result<FileProgram, Vec<diagnostics::Diagnostic>> {
    let mut profiler = cfg.profiler();
    profiler.start("lex_parse");
    let graph = frontend::modules::ModuleGraph::build_collecting(std::path::Path::new(path));
    let graph = match graph {
        Ok(g) => g,
        Err(errs) => return Err(errs),
    };
    let module = match graph.resolve() {
        Ok(m) => m,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    let mut total_lines = 0;
    for f in &graph.files {
        if let Ok(src) = std::fs::read_to_string(&f.path) {
            total_lines += src.lines().count();
        }
    }
    profiler.set_lines(total_lines);
    let debug = if cfg.debug {
        let file = graph
            .root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main.rnx".to_string());
        let dir = graph
            .root
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        let lines = frontend::modules::fn_lines(&graph)
            .into_iter()
            .map(|(k, v)| (k, (v.line, v.col)))
            .collect();
        Some(llvm::codegen::DebugInfo { file, dir, lines })
    } else {
        None
    };
    profiler.start("typecheck");
    let mut module = module;
    frontend::harness::strip_tests(&mut module);
    frontend::harness::strip_benches(&mut module);
    let mut errors = frontend::desugar::desugar(&mut module);
    let mut warnings = Vec::new();
    if errors.is_empty() {
        for d in frontend::semantic::check(&module) {
            if d.code.is_warning() {
                warnings.push(d);
            } else {
                errors.push(d);
            }
        }
    }
    profiler.stop();
    if !errors.is_empty() {
        return Err(errors);
    }
    profiler.start("lower");
    let lowered = match lir::lower::lower(&module) {
        Ok(l) => l,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    profiler.start("opt_pipeline");
    let mut lowered = lowered;
    let entry = resolve_entry(&lowered, entry);
    let opt_timings = lir::opt::optimize_lir_timed(&mut lowered, opt_level, entry);
    record_opt_subs(&mut profiler, &opt_timings);
    profiler.stop();
    profiler.start("verify");
    errors.extend(lir::verify::verify(&lowered));
    profiler.stop();
    if errors.is_empty() {
        errors.extend(graph.isolation_errors());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let native_libs = lir::instr::native_libs(&lowered);
    Ok(FileProgram { lir: lowered, warnings, profiler, debug, native_libs })
}

pub struct LibBuild {
    pub object: Vec<u8>,
    pub header: String,
    pub package: String,
    pub exports: Vec<frontend::header::ExportedFn>,
}

pub fn sanitize_lib_name(name: &str) -> String {
    name.chars()
        .map(|c| if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' { c } else { '_' })
        .collect()
}

pub fn package_name_for(path: &str) -> String {
    let p = std::path::Path::new(path);
    let start = if p.is_file() {
        p.parent().unwrap_or(std::path::Path::new(".")).to_path_buf()
    } else {
        p.to_path_buf()
    };
    if let Some(root) = frontend::project::find_project_root(&start) {
        if let Ok(Some(cfg)) = frontend::project::ProjectConfig::load_from_dir(&root) {
            if !cfg.name.is_empty() {
                return sanitize_lib_name(&cfg.name);
            }
        }
    }
    let stem = p
        .file_stem()
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "lib".to_string());
    sanitize_lib_name(&stem)
}

fn collect_exports(
    module: &frontend::ast::Module,
) -> Result<Vec<frontend::header::ExportedFn>, diagnostics::Diagnostic> {
    use diagnostics::{Code, Diagnostic};
    use frontend::header::{CABI, ExportedFn};
    let mut out = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for decl in &module.decls {
        let f = match &decl.node {
            frontend::ast::Decl::Fn(f) => f,
            _ => continue,
        };
        if f.access != frontend::ast::Access::Export || f.is_test {
            continue;
        }
        let short = f.name.rsplit('.').next().unwrap_or(&f.name).to_string();
        if !seen.insert(short.clone()) {
            return Err(Diagnostic::new(Code::E108, format!("duplicate C export `{short}`"))
                .with_span(decl.span));
        }
        if f.is_async {
            return Err(Diagnostic::new(
                Code::E108,
                format!("pub fn `{short}` is async and has no C ABI"),
            )
            .with_span(decl.span));
        }
        if f.throws {
            return Err(Diagnostic::new(
                Code::E108,
                format!("pub fn `{short}` cannot throw in --lib mode"),
            )
            .with_span(decl.span));
        }
        let mut params = Vec::with_capacity(f.params.len());
        for p in &f.params {
            let t = match &p.ty {
                Some(t) => t,
                None => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("pub fn `{short}` param `{}` needs a C-ABI type annotation", p.name),
                    )
                    .with_span(decl.span));
                }
            };
            match CABI::of_ast(t) {
                Some(CABI::Void) | None => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("pub fn `{short}` param `{}` has non-C-ABI type", p.name),
                    )
                    .with_span(decl.span));
                }
                Some(c) => params.push((p.name.clone(), c)),
            }
        }
        let ret = match &f.ret {
            Some(t) => match CABI::of_ast(t) {
                Some(CABI::Str) | None => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("pub fn `{short}` has non-C-ABI return type"),
                    )
                    .with_span(decl.span));
                }
                Some(c) => c,
            },
            None => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("pub fn `{short}` needs a C-ABI return type annotation"),
                )
                .with_span(decl.span));
            }
        };
        out.push(ExportedFn { name: short, params, ret });
    }
    Ok(out)
}

pub fn validate_target(triple: &str) -> Result<(), diagnostics::Diagnostic> {
    match triple {
        "x86_64-unknown-linux-gnu" | "aarch64-unknown-linux-gnu" => Ok(()),
        _ => Err(diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("unsupported target `{triple}`; expected x86_64-unknown-linux-gnu or aarch64-unknown-linux-gnu"),
        )),
    }
}

pub fn build_lib_files(
    path: &str,
    release: bool,
    opt_level: u8,
    target: Option<&str>,
) -> Result<LibBuild, Vec<diagnostics::Diagnostic>> {
    build_lib_files_cfg(path, release, opt_level, target, &CompileConfig::default())
}

pub fn build_lib_files_cfg(
    path: &str,
    release: bool,
    opt_level: u8,
    target: Option<&str>,
    cfg: &CompileConfig,
) -> Result<LibBuild, Vec<diagnostics::Diagnostic>> {
    let opt_level = if release { opt_level.max(1) } else { opt_level };
    let mut profiler = cfg.profiler();
    profiler.start("lex_parse");
    let graph = frontend::modules::ModuleGraph::build_collecting(std::path::Path::new(path));
    let graph = match graph {
        Ok(g) => g,
        Err(errs) => return Err(errs),
    };
    let module = match graph.resolve() {
        Ok(m) => m,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    let mut total_lines = 0;
    for f in &graph.files {
        if let Ok(src) = std::fs::read_to_string(&f.path) {
            total_lines += src.lines().count();
        }
    }
    profiler.set_lines(total_lines);
    let debug = if cfg.debug {
        let file = graph
            .root
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_else(|| "main.rnx".to_string());
        let dir = graph
            .root
            .parent()
            .map(|p| p.to_string_lossy().into_owned())
            .unwrap_or_else(|| ".".to_string());
        let lines = frontend::modules::fn_lines(&graph)
            .into_iter()
            .map(|(k, v)| (k, (v.line, v.col)))
            .collect();
        Some(llvm::codegen::DebugInfo { file, dir, lines })
    } else {
        None
    };
    let mut module = module;
    profiler.start("typecheck");
    frontend::harness::strip_tests(&mut module);
    frontend::harness::strip_benches(&mut module);
    let exports = match collect_exports(&module) {
        Ok(e) => e,
        Err(e) => return Err(vec![e]),
    };
    let package = package_name_for(path);
    let mut errors = frontend::desugar::desugar(&mut module);
    if errors.is_empty() {
        let mut warnings = Vec::new();
        for d in frontend::semantic::check(&module) {
            if d.code.is_warning() {
                warnings.push(d);
            } else {
                errors.push(d);
            }
        }
        let _ = warnings;
    }
    profiler.stop();
    if !errors.is_empty() {
        return Err(errors);
    }
    profiler.start("lower");
    let lowered = match lir::lower::lower(&module) {
        Ok(l) => l,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    profiler.start("opt_pipeline");
    let mut lowered = lowered;
    let opt_timings = lir::opt::optimize_lir_lib_timed(&mut lowered, opt_level);
    record_opt_subs(&mut profiler, &opt_timings);
    profiler.stop();
    profiler.start("verify");
    errors.extend(lir::verify::verify(&lowered));
    profiler.stop();
    if errors.is_empty() {
        errors.extend(graph.isolation_errors());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    let opt =
        if release { llvm::codegen::OptLevel::Release } else { llvm::codegen::OptLevel::Dev };
    let debug = if release { None } else { debug.as_ref() };
    profiler.start("codegen");
    let object = llvm::codegen::emit_library_with_debug(&lowered, "rnx_module", opt, target, debug)
        .map_err(|e| vec![e]);
    profiler.stop();
    let object = object?;
    profiler.finish();
    let header = frontend::header::generate_c_header(&package, &exports);
    Ok(LibBuild { object, header, package, exports })
}

pub struct DocTargets {
    pub root: std::path::PathBuf,
    pub packages: Vec<(String, std::path::PathBuf, frontend::project::ProjectConfig)>,
    pub workspace: bool,
}

pub fn resolve_doc_targets(
    cwd: &std::path::Path,
    package: Option<&str>,
    no_deps: bool,
) -> Result<DocTargets, diagnostics::Diagnostic> {
    use diagnostics::{Code, Diagnostic};
    if let Some(ws_root) = frontend::project::find_workspace_root_strict(cwd) {
        let manifest = frontend::project::load_manifest(&ws_root)?.ok_or_else(|| {
            Diagnostic::new(Code::E108, "workspace has no Project.config".to_string())
        })?;
        let ws = manifest.workspace.clone().ok_or_else(|| {
            Diagnostic::new(Code::E108, "workspace has no [workspace] section".to_string())
        })?;
        let members = frontend::project::resolve_workspace_members(&ws_root, &ws)?;
        let mut packages = Vec::new();
        if let Some(name) = package {
            let (dir, cfg) = members.get(name).ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("unknown workspace member `{name}`"))
            })?;
            packages.push((name.to_string(), dir.clone(), cfg.clone()));
        } else {
            for (name, (dir, cfg)) in &members {
                packages.push((name.clone(), dir.clone(), cfg.clone()));
            }
        }
        if !no_deps {
            let mut extra: Vec<(String, std::path::PathBuf, frontend::project::ProjectConfig)> =
                Vec::new();
            for (_, dir, cfg) in &packages {
                for dep in cfg.dependencies.values() {
                    if let frontend::project::DependencySpec::Path { path } = dep {
                        let dep_dir = dir.join(path);
                        if let Ok(Some(dcfg)) =
                            frontend::project::ProjectConfig::load_from_dir(&dep_dir)
                        {
                            if !dcfg.name.is_empty()
                                && !packages.iter().any(|(n, _, _)| n == &dcfg.name)
                                && !extra.iter().any(|(n, _, _)| n == &dcfg.name)
                            {
                                extra.push((dcfg.name.clone(), dep_dir, dcfg));
                            }
                        }
                    }
                }
            }
            extra.sort_by(|a, b| a.0.cmp(&b.0));
            packages.extend(extra);
        }
        return Ok(DocTargets { root: ws_root, packages, workspace: true });
    }
    if let Some(proj_root) = frontend::project::find_project_root(cwd) {
        let cfg = frontend::project::ProjectConfig::load_from_dir(&proj_root)?.ok_or_else(|| {
            Diagnostic::new(Code::E108, "project has no Project.config".to_string())
        })?;
        if let Some(name) = package {
            if name != cfg.name {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("unknown workspace member `{name}`"),
                ));
            }
        }
        let mut packages = vec![(cfg.name.clone(), proj_root.clone(), cfg)];
        if !no_deps {
            let (_, dir, cfg) = &packages[0].clone();
            for dep in cfg.dependencies.values() {
                if let frontend::project::DependencySpec::Path { path } = dep {
                    let dep_dir = dir.join(path);
                    if let Ok(Some(dcfg)) = frontend::project::ProjectConfig::load_from_dir(&dep_dir) {
                        if !dcfg.name.is_empty() {
                            packages.push((dcfg.name.clone(), dep_dir, dcfg));
                        }
                    }
                }
            }
        }
        return Ok(DocTargets { root: proj_root, packages, workspace: false });
    }
    Err(Diagnostic::new(
        Code::E108,
        "no project found; run `rnx doc` inside a project or workspace".to_string(),
    ))
}

pub fn render_package_docs(
    name: &str,
    dir: &std::path::Path,
    cfg: &frontend::project::ProjectConfig,
    out_dir: &std::path::Path,
    include_private: bool,
) -> Result<(), diagnostics::Diagnostic> {
    let entry = cfg.main_path(dir);
    let graph = frontend::modules::ModuleGraph::build(&entry)?;
    let mut inputs = Vec::new();
    for f in &graph.files {
        if f.path.extension().is_some_and(|e| e == "rnx") && f.path.starts_with(dir) {
            let modname = if f.key.is_empty() { name.to_string() } else { f.key.clone() };
            inputs.push(frontend::doc::DocInput { name: modname, module: f.module.clone() });
        }
    }
    let manifest =
        frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
    frontend::doc::generate_package_docs(&manifest, &inputs, out_dir, include_private)
}

pub fn generate_docs(
    cwd: &std::path::Path,
    package: Option<&str>,
    no_deps: bool,
    include_private: bool,
) -> Result<std::path::PathBuf, diagnostics::Diagnostic> {
    let targets = resolve_doc_targets(cwd, package, no_deps)?;
    let out_root = targets.root.join("target").join("doc");
    if targets.packages.len() == 1 && !targets.workspace {
        let (name, dir, cfg) = &targets.packages[0];
        render_package_docs(name, dir, cfg, &out_root, include_private)?;
        return Ok(out_root.join("index.html"));
    }
    let mut members: Vec<(String, String, String)> = Vec::new();
    for (name, dir, cfg) in &targets.packages {
        let out_dir = out_root.join(name);
        render_package_docs(name, dir, cfg, &out_dir, include_private)?;
        members.push((name.clone(), cfg.version.clone(), cfg.description.clone()));
    }
    members.sort_by(|a, b| a.0.cmp(&b.0));
    std::fs::create_dir_all(&out_root).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write {}: {e}", out_root.display()),
        )
    })?;
    let index = frontend::doc::render_workspace_index(&members, "style.css");
    std::fs::write(out_root.join("index.html"), index).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write index: {e}"),
        )
    })?;
    std::fs::write(out_root.join("style.css"), frontend::doc::render_css()).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write style: {e}"),
        )
    })?;
    Ok(out_root.join("index.html"))
}

pub fn collect_package_docs(
    name: &str,
    dir: &std::path::Path,
    cfg: &frontend::project::ProjectConfig,
    include_private: bool,
) -> Result<Vec<frontend::doc::DocModuleDoc>, diagnostics::Diagnostic> {
    let entry = cfg.main_path(dir);
    let graph = frontend::modules::ModuleGraph::build(&entry)?;
    let mut docs = Vec::new();
    for f in &graph.files {
        if f.path.extension().is_some_and(|e| e == "rnx") && f.path.starts_with(dir) {
            let modname = if f.key.is_empty() { name.to_string() } else { f.key.clone() };
            docs.push(frontend::doc::collect_module(&modname, &f.module, include_private));
        }
    }
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(docs)
}

pub fn generate_docs_json(
    cwd: &std::path::Path,
    package: Option<&str>,
    no_deps: bool,
    include_private: bool,
) -> Result<std::path::PathBuf, diagnostics::Diagnostic> {
    let targets = resolve_doc_targets(cwd, package, no_deps)?;
    let out_root = targets.root.join("target").join("doc");
    std::fs::create_dir_all(&out_root).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write {}: {e}", out_root.display()),
        )
    })?;
    let mut all: Vec<frontend::doc::DocModuleDoc> = Vec::new();
    for (name, dir, cfg) in &targets.packages {
        all.extend(collect_package_docs(name, dir, cfg, include_private)?);
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    let out_path = out_root.join("api.json");
    std::fs::write(&out_path, frontend::doc::modules_to_json(&all)).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write {}: {e}", out_path.display()),
        )
    })?;
    Ok(out_path)
}

pub fn generate_stdlib_docs_json(
    out_dir: &std::path::Path,
) -> Result<std::path::PathBuf, diagnostics::Diagnostic> {
    docgen::generate_stdlib_docs_json(out_dir)
}

pub struct PackTargets {
    pub root: std::path::PathBuf,
    pub packages: Vec<(String, std::path::PathBuf, frontend::project::ProjectConfig)>,
}

pub fn resolve_pack_targets(
    cwd: &std::path::Path,
    package: Option<&str>,
) -> Result<PackTargets, diagnostics::Diagnostic> {
    use diagnostics::{Code, Diagnostic};
    if let Some(ws_root) = frontend::project::find_workspace_root_strict(cwd) {
        let manifest = frontend::project::load_manifest(&ws_root)?.ok_or_else(|| {
            Diagnostic::new(Code::E108, "workspace has no Project.config".to_string())
        })?;
        let ws = manifest.workspace.clone().ok_or_else(|| {
            Diagnostic::new(Code::E108, "workspace has no [workspace] section".to_string())
        })?;
        let members = frontend::project::resolve_workspace_members(&ws_root, &ws)?;
        let mut packages = Vec::new();
        if let Some(name) = package {
            let (dir, cfg) = members.get(name).ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("unknown workspace member `{name}`"))
            })?;
            packages.push((name.to_string(), dir.clone(), cfg.clone()));
        } else {
            for (name, (dir, cfg)) in &members {
                packages.push((name.clone(), dir.clone(), cfg.clone()));
            }
        }
        return Ok(PackTargets { root: ws_root, packages });
    }
    if let Some(proj_root) = frontend::project::find_project_root(cwd) {
        let cfg = frontend::project::ProjectConfig::load_from_dir(&proj_root)?.ok_or_else(|| {
            Diagnostic::new(Code::E108, "project has no Project.config".to_string())
        })?;
        if let Some(name) = package {
            if name != cfg.name {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("unknown workspace member `{name}`"),
                ));
            }
        }
        return Ok(PackTargets {
            root: proj_root.clone(),
            packages: vec![(cfg.name.clone(), proj_root, cfg)],
        });
    }
    Err(Diagnostic::new(
        Code::E108,
        "no project found; run `rnx pack` inside a project or workspace".to_string(),
    ))
}

pub fn pack_targets(
    targets: &PackTargets,
    out_dir: &std::path::Path,
    gzip: bool,
) -> Result<Vec<(String, std::path::PathBuf)>, diagnostics::Diagnostic> {
    let mut done = Vec::new();
    for (name, dir, cfg) in &targets.packages {
        let manifest =
            frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
        let (tar, _) = if gzip {
            frontend::pack::pack_package_gz(dir, &manifest, out_dir)?
        } else {
            frontend::pack::pack_package(dir, &manifest, out_dir)?
        };
        done.push((name.clone(), tar));
    }
    done.sort();
    Ok(done)
}

pub fn check_files(path: &str) -> CheckReport {
    match frontend::check::check_package(std::path::Path::new(path)) {
        Ok(warnings) => CheckReport {
            errors: Vec::new(),
            warnings,
        },
        Err(errors) => CheckReport {
            errors,
            warnings: Vec::new(),
        },
    }
}

pub fn check_files_opt(path: &str, opt_level: u8) -> CheckReport {
    match load_program_opt(path, opt_level, "Main") {
        Ok(prog) => CheckReport {
            errors: Vec::new(),
            warnings: prog.warnings,
        },
        Err(errors) => CheckReport {
            errors,
            warnings: Vec::new(),
        },
    }
}

pub fn run_files(path: &str, entry: &str, args: Vec<Value>) -> RunResult {
    run_files_opt(path, entry, args, 1)
}

pub fn run_files_opt(
    path: &str,
    entry: &str,
    args: Vec<Value>,
    opt_level: u8,
) -> RunResult {
    run_files_cfg(path, entry, args, opt_level, TestBackend::Interpreter, &CompileConfig::default())
}

fn jit_args(args: &[Value]) -> Result<Vec<i64>, String> {
    let mut out = Vec::with_capacity(args.len());
    for v in args {
        match v {
            Value::Int(n) => out.push(*n),
            other => {
                return Err(format!(
                    "jit backends pass integer args only, got `{}`",
                    other.display()
                ))
            }
        }
    }
    Ok(out)
}

fn uncaught_payload(e: &diagnostics::Diagnostic) -> Option<&str> {
    e.message.strip_prefix("uncaught error: ")
}

pub fn run_files_cfg(
    path: &str,
    entry: &str,
    args: Vec<Value>,
    opt_level: u8,
    backend: TestBackend,
    cfg: &CompileConfig,
) -> RunResult {
    struct AssertStrictGuard {
        prev: bool,
    }
    impl AssertStrictGuard {
        fn enter() -> Self {
            let prev = runtime::native::assert_strict();
            runtime::native::rnx_set_assert_strict(true);
            AssertStrictGuard { prev }
        }
    }
    impl Drop for AssertStrictGuard {
        fn drop(&mut self) {
            runtime::native::rnx_set_assert_strict(self.prev);
        }
    }
    let _assert_strict = AssertStrictGuard::enter();
    let mut prog = match load_program_cfg(path, opt_level, entry, cfg) {
        Ok(p) => p,
        Err(errors) => {
            return RunResult {
                outcome: RunOutcome::Compile(errors),
                output: Vec::new(),
            };
        }
    };
    let mut cmdline = vec![path.to_string()];
    cmdline.extend(args.iter().map(|v| v.display()));
    runtime::native::rnx_set_args(&cmdline);
    let entry = resolve_entry(&prog.lir, entry);
    let entry_params = prog
        .lir
        .fn_id(entry)
        .map(|id| prog.lir.functions[id].params.len())
        .unwrap_or(0);
    if cfg.perf_map && backend == TestBackend::Interpreter {
        eprintln!("warning: --perf-map needs a JIT backend (cranelift|llvm); ignored");
    }
    prog.profiler.start("codegen");
    let (outcome, output) = match backend {
        TestBackend::Interpreter => {
            if std::env::var_os("RNX_DEBUG_LIR").is_some() {
                for f in &prog.lir.functions {
                    if f.name.contains("gradeMult") {
                        eprintln!("dbg lir fn {}", f.name);
                        for (bi, b) in f.blocks.iter().enumerate() {
                            for ins in &b.instrs {
                                eprintln!("dbg lir b{bi} {ins:?}");
                            }
                        }
                    }
                }
            }
            let leaked: &'static lir::instr::Module = Box::leak(Box::new(prog.lir));
            let mut machine = Machine::new(leaked);
            let outcome = match machine.call(entry, args) {
                Ok(v) => RunOutcome::Value(v),
                Err(ExecError::Throw(v)) => RunOutcome::Uncaught {
                    message: v.display(),
                    span: machine.error_span,
                    func: machine.error_func.clone(),
                },
                Err(ExecError::Fatal(m)) => RunOutcome::Fatal {
                    message: m,
                    span: machine.error_span,
                    func: machine.error_func.clone(),
                },
            };
            (outcome, machine.output.clone())
        }
        TestBackend::Cranelift => {
            let iargs = if entry_params == 0 {
                Vec::new()
            } else {
                match jit_args(&args) {
                    Ok(a) => a,
                    Err(m) => {
                        prog.profiler.stop();
                        return RunResult {
                            outcome: RunOutcome::Fatal {
                                message: m,
                                span: None,
                                func: None,
                            },
                            output: Vec::new(),
                        };
                    }
                }
            };
            let mut jit = match cranelift::jit::Jit::compile_with_timings(&prog.lir) {
                Ok((j, t)) => {
                    record_codegen_subs(&mut prog.profiler, &t);
                    j
                }
                Err(e) => {
                    prog.profiler.stop();
                    return RunResult {
                        outcome: RunOutcome::Compile(vec![e]),
                        output: Vec::new(),
                    };
                }
            };
            if cfg.perf_map {
                write_perf_map(&jit.perf_entries());
            }
            let outcome = match jit.call(entry, &iargs) {
                Ok(v) => RunOutcome::Value(Value::Int(v)),
                Err(e) => match uncaught_payload(&e) {
                    Some(message) => RunOutcome::Uncaught {
                        message: message.to_string(),
                        span: None,
                        func: None,
                    },
                    None => RunOutcome::Fatal {
                        message: e.message.clone(),
                        span: None,
                        func: None,
                    },
                },
            };
            (outcome, Vec::new())
        }
        TestBackend::Llvm => {
            if !args.is_empty() && entry_params > 0 {
                prog.profiler.stop();
                return RunResult {
                    outcome: RunOutcome::Fatal {
                        message: "llvm backend passes no args to the entry".to_string(),
                        span: None,
                        func: None,
                    },
                    output: Vec::new(),
                };
            }
            let outcome = if cfg.perf_map {
                match llvm::codegen::execute_with_map(&prog.lir, entry) {
                    Ok((v, entries)) => {
                        write_perf_map(&entries);
                        RunOutcome::Value(Value::Int(v))
                    }
                    Err(e) => match uncaught_payload(&e) {
                        Some(message) => RunOutcome::Uncaught {
                            message: message.to_string(),
                            span: None,
                            func: None,
                        },
                        None => RunOutcome::Compile(vec![e]),
                    },
                }
            } else {
                match llvm::codegen::execute(&prog.lir, entry) {
                    Ok(v) => RunOutcome::Value(Value::Int(v)),
                    Err(e) => match uncaught_payload(&e) {
                        Some(message) => RunOutcome::Uncaught {
                            message: message.to_string(),
                            span: None,
                            func: None,
                        },
                        None => RunOutcome::Compile(vec![e]),
                    },
                }
            };
            (outcome, Vec::new())
        }
    };
    prog.profiler.stop();
    prog.profiler.finish();
    RunResult { outcome, output }
}

pub fn build_files(path: &str, entry: &str, release: bool) -> Result<Vec<u8>, Vec<Diagnostic>> {
    build_files_opt(path, entry, release, 1, None)
}

pub fn build_files_opt(
    path: &str,
    entry: &str,
    release: bool,
    opt_level: u8,
    target: Option<&str>,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    build_files_cfg(path, entry, release, opt_level, target, &CompileConfig::default())
}

pub fn build_files_cfg(
    path: &str,
    entry: &str,
    release: bool,
    opt_level: u8,
    target: Option<&str>,
    cfg: &CompileConfig,
) -> Result<Vec<u8>, Vec<Diagnostic>> {
    Ok(build_files_staged_cfg(path, entry, release, opt_level, target, cfg)?.bytes)
}

pub struct BuildOutput {
    pub bytes: Vec<u8>,
    pub stages: Vec<(String, f64)>,
    pub native_libs: Vec<String>,
}

pub fn build_files_staged_cfg(
    path: &str,
    entry: &str,
    release: bool,
    opt_level: u8,
    target: Option<&str>,
    cfg: &CompileConfig,
) -> Result<BuildOutput, Vec<Diagnostic>> {
    let opt_level = if release { opt_level.max(1) } else { opt_level };
    let mut prog = load_program_cfg(path, opt_level, entry, cfg)?;
    let entry = resolve_entry(&prog.lir, entry);
    let opt = if release {
        llvm::codegen::OptLevel::Release
    } else {
        llvm::codegen::OptLevel::Dev
    };
    let debug = if release { None } else { prog.debug.as_ref() };
    prog.profiler.start("codegen");
    let out = llvm::codegen::emit_object_with_debug(
        &prog.lir,
        "rnx_module",
        entry,
        opt,
        target,
        debug,
    )
    .map_err(|e| vec![e]);
    prog.profiler.stop();
    let out = out?;
    prog.profiler.finish();
    let stages = prog
        .profiler
        .records()
        .iter()
        .map(|(name, _, dur_ns)| (name.to_string(), *dur_ns as f64 / 1_000_000.0))
        .collect();
    Ok(BuildOutput { bytes: out, stages, native_libs: prog.native_libs })
}


pub fn init_project(name: &str, cwd: &std::path::Path) -> Result<std::path::PathBuf, String> {
    if name.is_empty() || name == "." || name == ".." || name.contains(['/', '\\']) {
        return Err(format!("invalid project name `{name}`"));
    }
    let root = cwd.join(name);
    if root.exists() {
        let non_empty = std::fs::read_dir(&root)
            .map(|mut d| d.next().is_some())
            .unwrap_or(true);
        if non_empty {
            return Err(format!(
                "`{}` already exists and is not empty",
                root.display()
            ));
        }
    }
    std::fs::create_dir_all(root.join("src"))
        .map_err(|e| format!("cannot create {}: {e}", root.display()))?;
    let manifest = frontend::project::Manifest {
        project: Some(frontend::project::ProjectConfig {
            name: name.to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            engine: String::new(),
            entries: frontend::project::Entries {
                main: frontend::project::DEFAULT_ENTRY.to_string(),
                lib: None,
                docs: None,
                bins: std::collections::BTreeMap::new(),
            },
            registry: None,
            registries: std::collections::BTreeMap::new(),
            dependencies: std::collections::BTreeMap::new(),
            permissions: None,
        }),
        workspace: None,
    };
    let config = frontend::project::manifest_to_rnx(&manifest);
    std::fs::write(
        root.join(frontend::project::MANIFEST_FILE),
        config,
    )
    .map_err(|e| format!("cannot write Project.config: {e}"))?;
    let main = format!(
        "fn Main(): Int {{\n    print(\"Hello from {name}!\");\n    return 0;\n}}\n"
    );
    std::fs::write(root.join(frontend::project::DEFAULT_ENTRY), main)
        .map_err(|e| format!("cannot write src/main.rnx: {e}"))?;
    ensure_gitignore_entry(&root.join(".gitignore"))
        .map_err(|e| format!("cannot write .gitignore: {e}"))?;
    Ok(root)
}

fn ensure_gitignore_entry(path: &std::path::Path) -> std::io::Result<()> {
    const LINE: &str = ".rnx-cache/";
    match std::fs::read_to_string(path) {
        Ok(existing) => {
            if existing.lines().any(|line| line.trim() == LINE) {
                return Ok(());
            }
            let mut out = existing;
            if !out.is_empty() && !out.ends_with('\n') {
                out.push('\n');
            }
            out.push_str(".rnx-cache/\n");
            std::fs::write(path, out)
        }
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            std::fs::write(path, ".rnx-cache/\n")
        }
        Err(e) => Err(e),
    }
}

pub struct ScopeTarget {
    pub scope_root: Option<std::path::PathBuf>,
    pub entry: String,
    pub member: Option<String>,
}

fn no_manifest() -> Diagnostic {
    Diagnostic::new(Code::E108, "No file specified and no Project.config found")
}

fn scope_of(start: &std::path::Path) -> Result<std::path::PathBuf, Diagnostic> {
    frontend::project::find_workspace_root(&start).ok_or_else(no_manifest)
}

pub fn resolve_member(
    ws_root: &std::path::Path,
    name: &str,
) -> Result<(std::path::PathBuf, frontend::project::ProjectConfig), Diagnostic> {
    let unknown = || Diagnostic::new(Code::E108, format!("unknown workspace member `{name}`"));
    let manifest = frontend::project::load_manifest(ws_root)?.ok_or_else(unknown)?;
    let ws = manifest.workspace.ok_or_else(unknown)?;
    let members = frontend::project::resolve_workspace_members(ws_root, &ws)?;
    members.get(name).cloned().ok_or_else(unknown)
}

pub fn resolve_scope_target(
    path: Option<&str>,
    package: Option<&str>,
) -> Result<ScopeTarget, Diagnostic> {
    if let Some(p) = path {
        let start = {
            let p = std::path::Path::new(p);
            if p.is_file() {
                p.parent().unwrap_or(std::path::Path::new(".")).to_path_buf()
            } else {
                p.to_path_buf()
            }
        };
        let scope_root = frontend::project::find_workspace_root(&start);
        return Ok(ScopeTarget {
            scope_root,
            entry: p.to_string(),
            member: None,
        });
    }
    let cwd = std::env::current_dir()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read working directory: {e}")))?;
    if let Some(name) = package {
        let ws_root = frontend::project::find_workspace_root_strict(&cwd).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("unknown workspace member `{name}`"))
        })?;
        let (member_root, cfg) = resolve_member(&ws_root, name)?;
        let entry = cfg.main_path(&member_root).to_string_lossy().into_owned();
        return Ok(ScopeTarget {
            scope_root: Some(ws_root),
            entry,
            member: Some(name.to_string()),
        });
    }
    let scope_root = scope_of(&cwd)?;
    let manifest = frontend::project::load_manifest(&scope_root)?.ok_or_else(no_manifest)?;
    if manifest.workspace.is_none() {
        let cfg = manifest.project.ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("{}: missing [project] section", scope_root.display()))
        })?;
        let entry = cfg.main_path(&scope_root).to_string_lossy().into_owned();
        return Ok(ScopeTarget {
            scope_root: Some(scope_root),
            entry,
            member: None,
        });
    }
    let nearest = frontend::project::find_project_root(&cwd).ok_or_else(no_manifest)?;
    if nearest != scope_root {
        let cfg =
            frontend::project::ProjectConfig::load_from_dir(&nearest)?.ok_or_else(no_manifest)?;
        let entry = cfg.main_path(&nearest).to_string_lossy().into_owned();
        return Ok(ScopeTarget {
            scope_root: Some(scope_root),
            entry,
            member: Some(cfg.name),
        });
    }
    match manifest.project {
        Some(cfg) => {
            let entry = cfg.main_path(&scope_root).to_string_lossy().into_owned();
            Ok(ScopeTarget {
                scope_root: Some(scope_root),
                entry,
                member: None,
            })
        }
        None => {
            let ws = manifest.workspace.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("{}: missing [project] section", scope_root.display()))
            })?;
            let members = frontend::project::resolve_workspace_members(&scope_root, &ws)?;
            let mut names: Vec<&str> = members.keys().map(|s| s.as_str()).collect();
            names.sort();
            Err(Diagnostic::new(
                Code::E108,
                format!(
                    "multiple packages in workspace, specify -p <package> ({})",
                    names.join(", ")
                ),
            ))
        }
    }
}

pub fn lock_scope(path: Option<&str>, package: Option<&str>) -> Result<usize, Diagnostic> {
    let scope_root = match (path, package) {
        (Some(p), _) => {
            let target = resolve_scope_target(Some(p), None)?;
            target.scope_root.ok_or_else(no_manifest)?
        }
        (None, Some(name)) => {
            let target = resolve_scope_target(None, Some(name))?;
            target.scope_root.ok_or_else(no_manifest)?
        }
        (None, None) => {
            let cwd = std::env::current_dir().map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot read working directory: {e}"))
            })?;
            scope_of(&cwd)?
        }
    };
    let manifest = frontend::project::load_manifest(&scope_root)?.ok_or_else(no_manifest)?;
    let lock = match manifest.workspace {
        Some(ws) => {
            let members = frontend::project::resolve_workspace_members(&scope_root, &ws)?;
            frontend::deplock::ProjectDepLock::resolve_workspace(&scope_root, &members)?
        }
        None => {
            let cfg = manifest.project.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("{}: missing [project] section", scope_root.display()))
            })?;
            frontend::deplock::ProjectDepLock::resolve(&scope_root, &cfg)?
        }
    };
    let count = lock.packages.len();
    lock.write(&scope_root)?;
    Ok(count)
}

pub fn missing_lock() -> Diagnostic {
    Diagnostic::new(Code::E108, "Project.deplock not found, run 'rnx lock'")
}

pub fn enforce_locked_at(scope_root: &std::path::Path) -> Result<(), Diagnostic> {
    let manifest = frontend::project::load_manifest(scope_root)?.ok_or_else(no_manifest)?;
    let lock = frontend::deplock::ProjectDepLock::load(scope_root)?.ok_or_else(missing_lock)?;
    match manifest.workspace {
        Some(ws) => {
            let members = frontend::project::resolve_workspace_members(scope_root, &ws)?;
            frontend::deplock::verify_workspace_lock(scope_root, &members, &lock)
        }
        None => {
            let cfg = manifest.project.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("{}: missing [project] section", scope_root.display()))
            })?;
            frontend::deplock::verify_lock(scope_root, &cfg, &lock)
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum TestBackend {
    Interpreter,
    Cranelift,
    Llvm,
}

pub enum TestOutcome {
    Completed { output: Vec<String>, failed: i64 },
    Compile(Vec<Diagnostic>),
    Runtime(String),
}

pub fn discover_test_files(pkg_root: &std::path::Path) -> Result<Vec<std::path::PathBuf>, Diagnostic> {
    let mut out: Vec<std::path::PathBuf> = Vec::new();
    for sub in ["src", "tests"] {
        let base = pkg_root.join(sub);
        if !base.is_dir() {
            continue;
        }
        let mut stack = vec![base];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<std::path::PathBuf> = Vec::new();
            let read = std::fs::read_dir(&dir).map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", dir.display()))
            })?;
            for entry in read {
                let entry = entry.map_err(|e| {
                    Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", dir.display()))
                })?;
                entries.push(entry.path());
            }
            entries.sort();
            for path in entries {
                let name = path
                    .file_name()
                    .map(|s| s.to_string_lossy().into_owned())
                    .unwrap_or_default();
                if path.is_dir() {
                    if !name.starts_with('.') {
                        stack.push(path);
                    }
                } else if name.ends_with(".rnx") {
                    out.push(path);
                }
            }
        }
    }
    out.sort();
    Ok(out)
}

pub fn test_targets(
    package: Option<&str>,
) -> Result<Vec<(std::path::PathBuf, frontend::project::ProjectConfig)>, Diagnostic> {
    let cwd = std::env::current_dir()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read working directory: {e}")))?;
    if let Some(name) = package {
        let ws_root = frontend::project::find_workspace_root_strict(&cwd).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("unknown workspace member `{name}`"))
        })?;
        let (root, cfg) = resolve_member(&ws_root, name)?;
        return Ok(vec![(root, cfg)]);
    }
    let scope_root = scope_of(&cwd)?;
    let manifest = frontend::project::load_manifest(&scope_root)?.ok_or_else(no_manifest)?;
    if manifest.workspace.is_none() {
        let cfg = manifest.project.ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("{}: missing [project] section", scope_root.display()))
        })?;
        return Ok(vec![(scope_root, cfg)]);
    }
    let nearest = frontend::project::find_project_root(&cwd).ok_or_else(no_manifest)?;
    if nearest != scope_root {
        let cfg =
            frontend::project::ProjectConfig::load_from_dir(&nearest)?.ok_or_else(no_manifest)?;
        return Ok(vec![(nearest, cfg)]);
    }
    match manifest.project {
        Some(cfg) => Ok(vec![(scope_root, cfg)]),
        None => {
            let ws = manifest.workspace.ok_or_else(no_manifest)?;
            let members = frontend::project::resolve_workspace_members(&scope_root, &ws)?;
            let mut names: Vec<&str> = members.keys().map(|s| s.as_str()).collect();
            names.sort();
            Err(Diagnostic::new(
                Code::E108,
                format!(
                    "multiple packages in workspace, specify -p <package> ({})",
                    names.join(", ")
                ),
            ))
        }
    }
}

pub fn load_test_program(
    pkg_root: &std::path::Path,
    filter: Option<&str>,
) -> Result<lir::instr::Module, Vec<Diagnostic>> {
    load_test_program_opt(pkg_root, filter, false, 1)
}

pub fn load_test_program_opt(
    pkg_root: &std::path::Path,
    filter: Option<&str>,
    exact: bool,
    opt_level: u8,
) -> Result<lir::instr::Module, Vec<Diagnostic>> {
    Ok(load_test_program_cfg(pkg_root, filter, exact, opt_level, &CompileConfig::default())?.0)
}

pub fn load_test_program_cfg(
    pkg_root: &std::path::Path,
    filter: Option<&str>,
    exact: bool,
    opt_level: u8,
    cfg: &CompileConfig,
) -> Result<(lir::instr::Module, frontend::profiler::PassProfiler), Vec<Diagnostic>> {
    let mut profiler = cfg.profiler();
    profiler.start("lex_parse");
    let config = frontend::project::ProjectConfig::load_from_dir(pkg_root)
        .map_err(|e| vec![e])?
        .ok_or_else(|| {
            vec![Diagnostic::new(
                Code::E108,
                format!("{}: missing [project] section", pkg_root.display()),
            )]
        })?;
    let entry = config.main_path(pkg_root);
    let extra = discover_test_files(pkg_root).map_err(|e| vec![e])?;
    let graph = frontend::modules::ModuleGraph::build_collecting_extra(&entry, &extra)?;
    let module = match graph.resolve() {
        Ok(m) => m,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    let mut total_lines = 0;
    for f in &graph.files {
        if let Ok(src) = std::fs::read_to_string(&f.path) {
            total_lines += src.lines().count();
        }
    }
    profiler.set_lines(total_lines);
    profiler.start("typecheck");
    let mut module = module;
    let display_files: Vec<(String, String)> = graph
        .files
        .iter()
        .map(|f| {
            let display = f
                .path
                .strip_prefix(pkg_root)
                .unwrap_or(&f.path)
                .to_string_lossy()
                .replace('\\', "/");
            (f.key.clone(), display)
        })
        .collect();
    if let Err(e) =
        frontend::harness::generate_test_harness(&mut module, filter, exact, &display_files)
    {
        return Err(vec![e]);
    }
    let mut errors = frontend::desugar::desugar(&mut module);
    if errors.is_empty() {
        for d in frontend::semantic::check(&module) {
            if !d.code.is_warning() {
                errors.push(d);
            }
        }
    }
    profiler.stop();
    if !errors.is_empty() {
        return Err(errors);
    }
    profiler.start("lower");
    let lowered = match lir::lower::lower(&module) {
        Ok(l) => l,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    profiler.start("opt_pipeline");
    let mut lowered = lowered;
    let opt_timings = lir::opt::optimize_lir_timed(&mut lowered, opt_level, "Main");
    record_opt_subs(&mut profiler, &opt_timings);
    profiler.stop();
    profiler.start("verify");
    errors.extend(lir::verify::verify(&lowered));
    profiler.stop();
    if errors.is_empty() {
        errors.extend(graph.isolation_errors());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok((lowered, profiler))
}

pub fn load_bench_program_opt(
    pkg_root: &std::path::Path,
    filter: Option<&str>,
    opt_level: u8,
) -> Result<lir::instr::Module, Vec<Diagnostic>> {
    Ok(load_bench_program_cfg(pkg_root, filter, opt_level, &CompileConfig::default())?.0)
}

pub fn load_bench_program_cfg(
    pkg_root: &std::path::Path,
    filter: Option<&str>,
    opt_level: u8,
    cfg: &CompileConfig,
) -> Result<(lir::instr::Module, frontend::profiler::PassProfiler), Vec<Diagnostic>> {
    let mut profiler = cfg.profiler();
    profiler.start("lex_parse");
    let config = frontend::project::ProjectConfig::load_from_dir(pkg_root)
        .map_err(|e| vec![e])?
        .ok_or_else(|| {
            vec![Diagnostic::new(
                Code::E108,
                format!("{}: missing [project] section", pkg_root.display()),
            )]
        })?;
    let entry = config.main_path(pkg_root);
    let extra = discover_test_files(pkg_root).map_err(|e| vec![e])?;
    let graph = frontend::modules::ModuleGraph::build_collecting_extra(&entry, &extra)?;
    let module = match graph.resolve() {
        Ok(m) => m,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    let mut total_lines = 0;
    for f in &graph.files {
        if let Ok(src) = std::fs::read_to_string(&f.path) {
            total_lines += src.lines().count();
        }
    }
    profiler.set_lines(total_lines);
    profiler.start("typecheck");
    let mut module = module;
    frontend::harness::strip_tests(&mut module);
    let benches = match frontend::harness::collect_benches(&module, filter) {
        Ok(b) => b,
        Err(e) => return Err(vec![e]),
    };
    if let Err(e) = frontend::bench::generate_bench_harness(&mut module, &benches) {
        return Err(vec![e]);
    }
    let mut errors = frontend::desugar::desugar(&mut module);
    if errors.is_empty() {
        for d in frontend::semantic::check(&module) {
            if !d.code.is_warning() {
                errors.push(d);
            }
        }
    }
    profiler.stop();
    if !errors.is_empty() {
        return Err(errors);
    }
    profiler.start("lower");
    let lowered = match lir::lower::lower(&module) {
        Ok(l) => l,
        Err(e) => return Err(vec![e]),
    };
    profiler.stop();
    profiler.start("opt_pipeline");
    let mut lowered = lowered;
    let opt_timings = lir::opt::optimize_lir_timed(&mut lowered, opt_level, "Main");
    record_opt_subs(&mut profiler, &opt_timings);
    profiler.stop();
    profiler.start("verify");
    errors.extend(lir::verify::verify(&lowered));
    profiler.stop();
    if errors.is_empty() {
        errors.extend(graph.isolation_errors());
    }
    if !errors.is_empty() {
        return Err(errors);
    }
    Ok((lowered, profiler))
}

pub fn run_test_harness(lowered: lir::instr::Module, backend: TestBackend) -> TestOutcome {
    run_test_harness_cfg(
        lowered,
        backend,
        &CompileConfig::default(),
        &mut frontend::profiler::PassProfiler::disabled(),
    )
}

pub fn run_test_harness_cfg(
    lowered: lir::instr::Module,
    backend: TestBackend,
    cfg: &CompileConfig,
    profiler: &mut frontend::profiler::PassProfiler,
) -> TestOutcome {
    if cfg.perf_map && backend == TestBackend::Interpreter {
        eprintln!("warning: --perf-map needs a JIT backend (cranelift|llvm); ignored");
    }
    profiler.start("codegen");
    let outcome = match backend {
        TestBackend::Interpreter => {
            let leaked: &'static lir::instr::Module = Box::leak(Box::new(lowered));
            let mut machine = runtime::machine::Machine::new(leaked);
            match machine.call("Main", Vec::new()) {
                Ok(v) => match v {
                    runtime::value::Value::Int(f) => TestOutcome::Completed {
                        output: machine.output.clone(),
                        failed: f,
                    },
                    other => TestOutcome::Runtime(format!(
                        "test harness returned non-integer `{}`",
                        other.display()
                    )),
                },
                Err(runtime::machine::ExecError::Throw(v)) => {
                    TestOutcome::Runtime(format!("uncaught throw: {}", v.display()))
                }
                Err(runtime::machine::ExecError::Fatal(m)) => TestOutcome::Runtime(m),
            }
        }
        TestBackend::Cranelift => {
            let mut jit = match cranelift::jit::Jit::compile_with_timings(&lowered) {
                Ok((j, t)) => {
                    record_codegen_subs(profiler, &t);
                    j
                }
                Err(e) => {
                    profiler.stop();
                    return TestOutcome::Compile(vec![e]);
                }
            };
            if cfg.perf_map {
                write_perf_map(&jit.perf_entries());
            }
            match jit.call("Main", &[]) {
                Ok(f) => TestOutcome::Completed {
                    output: Vec::new(),
                    failed: f,
                },
                Err(e) => TestOutcome::Runtime(e.message.clone()),
            }
        }
        TestBackend::Llvm => {
            if cfg.perf_map {
                match llvm::codegen::execute_with_map(&lowered, "Main") {
                    Ok((f, entries)) => {
                        write_perf_map(&entries);
                        TestOutcome::Completed {
                            output: Vec::new(),
                            failed: f,
                        }
                    }
                    Err(e) => TestOutcome::Runtime(e.message.clone()),
                }
            } else {
                match llvm::codegen::execute(&lowered, "Main") {
                    Ok(f) => TestOutcome::Completed {
                        output: Vec::new(),
                        failed: f,
                    },
                    Err(e) => TestOutcome::Runtime(e.message.clone()),
                }
            }
        }
    };
    profiler.stop();
    profiler.finish();
    outcome
}

fn fetch_scope_root(package: Option<&str>) -> Result<std::path::PathBuf, Diagnostic> {
    match package {
        Some(name) => {
            let target = resolve_scope_target(None, Some(name))?;
            target.scope_root.ok_or_else(no_manifest)
        }
        None => {
            let cwd = std::env::current_dir().map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot read working directory: {e}"))
            })?;
            scope_of(&cwd)
        }
    }
}

pub fn fetch_scope(
    package: Option<&str>,
) -> Result<Vec<(String, String, String)>, Diagnostic> {
    let scope_root = fetch_scope_root(package)?;
    frontend::fetch::fetch_all_git_deps(&scope_root)
}

pub fn vendor_scope(package: Option<&str>) -> Result<Vec<String>, Diagnostic> {
    let scope_root = fetch_scope_root(package)?;
    frontend::fetch::vendor_all(&scope_root)
}

pub enum AddResult {
    Added(String),
    Failed(Diagnostic),
    Headless(String),
}

pub fn add_package(
    package: &str,
    path: Option<&std::path::Path>,
    accept_caps: Option<&str>,
    accept_all_caps: bool,
) -> AddResult {
    let cwd = match std::env::current_dir() {
        Ok(d) => d,
        Err(e) => {
            return AddResult::Failed(Diagnostic::new(
                Code::E108,
                format!("cannot read working directory: {e}"),
            ));
        }
    };
    let Some(root) = frontend::project::find_project_root(&cwd) else {
        return AddResult::Failed(Diagnostic::new(
            Code::E108,
            "no Project.config found; run `rnx init` first",
        ));
    };
    let Some(rel) = path else {
        return AddResult::Failed(Diagnostic::new(
            Code::E108,
            "registry add is not supported yet; pass `--path <dir>` with a local package",
        ));
    };
    let dep_dir = if rel.is_absolute() {
        rel.to_path_buf()
    } else {
        cwd.join(rel)
    };
    let dep_dir = match std::fs::canonicalize(&dep_dir) {
        Ok(d) => d,
        Err(_) => {
            return AddResult::Failed(Diagnostic::new(
                Code::E108,
                format!("dependency directory `{}` does not exist", rel.display()),
            ));
        }
    };
    let analysis = match frontend::security::analyze_dep_dir(&dep_dir) {
        Ok(a) => a,
        Err(e) => return AddResult::Failed(e),
    };
    if analysis.name != package {
        return AddResult::Failed(Diagnostic::new(
            Code::E108,
            format!(
                "package name mismatch: requested `{package}`, manifest says `{}`",
                analysis.name,
            ),
        ));
    }
    let tier = analysis.tier.to_string();
    if !analysis.capabilities.is_empty() {
        if accept_all_caps {
            // Approved below.
        } else if let Some(list) = accept_caps {
            let accepted: std::collections::BTreeSet<String> = list
                .split(',')
                .map(|s| s.trim().to_string())
                .filter(|s| !s.is_empty())
                .collect();
            let missing: Vec<&String> = analysis
                .capabilities
                .iter()
                .filter(|c| !accepted.contains(*c))
                .collect();
            if !missing.is_empty() {
                let missing = missing
                    .iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                return AddResult::Failed(Diagnostic::new(
                    Code::E108,
                    format!("`--accept-caps` does not cover deduced {missing}"),
                ));
            }
        } else if std::io::stdin().is_terminal() {
            println!("package `{package}` (tier {tier}) requests:");
            for c in &analysis.capabilities {
                println!("  {c}");
            }
            match prompt_grant(&analysis.traces) {
                GrantAnswer::Yes => {}
                GrantAnswer::No => {
                    return AddResult::Failed(Diagnostic::new(
                        Code::E108,
                        format!("add of `{package}` aborted; Project.deplock unchanged"),
                    ));
                }
            }
        } else {
            let caps = analysis.capabilities.join(",");
            return AddResult::Headless(format!(
                "error[S101]: Package '{package}' requires unapproved capabilities: [{}]\nTo approve non-interactively, re-run with:\n  rnx add {package} --path {} --accept-caps=\"{caps}\"",
                analysis.capabilities.join(", "),
                dep_dir.display(),
            ));
        }
    }
    if let Err(e) = record_dependency(&root, &dep_dir, &analysis.name) {
        return AddResult::Failed(e);
    }
    match record_lock_entry(&root, &dep_dir, &analysis) {
        Ok(()) => AddResult::Added(format!(
            "Added {package} v{} (tier {tier})",
            analysis.version,
        )),
        Err(e) => AddResult::Failed(e),
    }
}

enum GrantAnswer {
    Yes,
    No,
}

fn prompt_grant(traces: &[frontend::capabilities::CallChain]) -> GrantAnswer {
    use std::io::{BufRead, Write};
    let stdin = std::io::stdin();
    let mut lines = stdin.lock().lines();
    loop {
        print!("Do you want to grant these capabilities? [y/N/inspect] ");
        let _ = std::io::stdout().flush();
        let answer = lines.next().and_then(|l| l.ok()).unwrap_or_default();
        let answer = answer.trim().to_ascii_lowercase();
        match answer.as_str() {
            "y" | "yes" => return GrantAnswer::Yes,
            "inspect" => {
                for t in traces {
                    println!("  {} (delegated: {})", t.capability, t.is_delegated);
                    for n in &t.nodes {
                        println!("    {}:{}:{} {}", n.file, n.line, n.col, n.symbol);
                    }
                }
            }
            _ => return GrantAnswer::No,
        }
    }
}

fn record_dependency(
    root: &std::path::Path,
    dep_dir: &std::path::Path,
    name: &str,
) -> Result<(), Diagnostic> {
    let manifest_path = root.join(frontend::project::MANIFEST_FILE);
    let mut manifest =
        frontend::project::load_manifest(root)?.ok_or_else(|| {
            Diagnostic::new(Code::E108, "project has no Project.config".to_string())
        })?;
    let rel = pathdiff_relative(root, dep_dir);
    let project = manifest.project.as_mut().ok_or_else(|| {
        Diagnostic::new(Code::E108, "Project.config has no `project` object".to_string())
    })?;
    match project.dependencies.get(name) {
        Some(frontend::project::DependencySpec::Path { path })
            if path.to_string_lossy().replace('\\', "/") == rel =>
        {
            return Ok(())
        }
        Some(_) => {
            return Err(Diagnostic::new(
                Code::E108,
                format!("dependency `{name}` already declared with a different source"),
            ));
        }
        None => {}
    }
    project.dependencies.insert(
        name.to_string(),
        frontend::project::DependencySpec::Path { path: std::path::PathBuf::from(&rel) },
    );
    let out = frontend::project::manifest_to_rnx(&manifest);
    std::fs::write(&manifest_path, out).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", manifest_path.display()))
    })?;
    Ok(())
}

fn pathdiff_relative(root: &std::path::Path, dir: &std::path::Path) -> String {
    if let Ok(rel) = dir.strip_prefix(root) {
        return rel.to_string_lossy().replace('\\', "/");
    }
    let mut r = root.components().peekable();
    let mut d = dir.components().peekable();
    while r.peek() == d.peek() {
        if r.peek().is_none() {
            break;
        }
        r.next();
        d.next();
    }
    let mut out = String::new();
    for _ in r {
        out.push_str("../");
    }
    for c in d {
        out.push_str(&c.as_os_str().to_string_lossy().replace('\\', "/"));
        out.push('/');
    }
    out.trim_end_matches('/').to_string()
}

fn record_lock_entry(
    root: &std::path::Path,
    dep_dir: &std::path::Path,
    analysis: &frontend::security::DepAnalysis,
) -> Result<(), Diagnostic> {
    let mut lock = frontend::deplock::ProjectDepLock::load(root)?.unwrap_or(
        frontend::deplock::ProjectDepLock {
            version: frontend::deplock::LOCK_VERSION,
            packages: Vec::new(),
        },
    );
    let checksum = frontend::checksum::compute_package_checksum(dep_dir)?;
    let rel = pathdiff_relative(root, dep_dir);
    lock.packages.retain(|p| p.name != analysis.name);
    lock.packages.push(frontend::deplock::LockedPackage {
        name: analysis.name.clone(),
        version: analysis.version.clone(),
        source: format!("path:{rel}"),
        checksum,
        dependencies: Vec::new(),
        tier: analysis.tier.to_string(),
        capabilities: analysis.capabilities.clone(),
    });
    ensure_root_entry(root, &mut lock)?;
    lock.packages.sort_by(|a, b| a.name.cmp(&b.name));
    lock.version = frontend::deplock::LOCK_VERSION;
    lock.write(root)
}

fn ensure_root_entry(
    root: &std::path::Path,
    lock: &mut frontend::deplock::ProjectDepLock,
) -> Result<(), Diagnostic> {
    if lock.packages.iter().any(|p| p.source == "root") {
        return Ok(());
    }
    let config = frontend::project::ProjectConfig::load_from_dir(root)?.ok_or_else(|| {
        Diagnostic::new(Code::E108, "no Project.config found; run `rnx init` first")
    })?;
    let checksum = frontend::checksum::compute_package_checksum(root)?;
    lock.packages.push(frontend::deplock::LockedPackage {
        name: config.name,
        version: config.version,
        source: "root".to_string(),
        checksum,
        dependencies: Vec::new(),
        tier: "pure".to_string(),
        capabilities: Vec::new(),
    });
    Ok(())
}

pub fn apply_locked_spawn_gate(scope_root: Option<&std::path::Path>) {
    let Some(root) = scope_root else {
        runtime::native::rnx_clear_spawn_restriction();
        return;
    };
    let mut allowed: Vec<String> = Vec::new();
    if let Ok(Some(lock)) = frontend::deplock::ProjectDepLock::load(root) {
        for pkg in &lock.packages {
            for cap in &pkg.capabilities {
                if let Some(bin) = cap.strip_prefix("sys:exec:") {
                    let bin = bin.trim();
                    if !bin.is_empty() && !allowed.iter().any(|b| b == bin) {
                        allowed.push(bin.to_string());
                    }
                }
            }
        }
    }
    let csv = allowed.join(",");
    runtime::native::rnx_set_spawn_allowlist(csv.as_ptr(), csv.len());
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> std::path::PathBuf {
        let dir =
            std::env::temp_dir().join(format!("rnx-gitignore-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn gitignore_missing_file_is_created() {
        let dir = scratch("missing");
        let path = dir.join(".gitignore");
        ensure_gitignore_entry(&path).unwrap();
        assert_eq!(std::fs::read_to_string(&path).unwrap(), ".rnx-cache/\n");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gitignore_existing_content_survives() {
        let dir = scratch("append");
        let path = dir.join(".gitignore");
        std::fs::write(&path, "target/\n*.log").unwrap();
        ensure_gitignore_entry(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "target/\n*.log\n.rnx-cache/\n"
        );
        ensure_gitignore_entry(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "target/\n*.log\n.rnx-cache/\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn gitignore_existing_entry_left_untouched() {
        let dir = scratch("untouched");
        let path = dir.join(".gitignore");
        std::fs::write(&path, "target/\n.rnx-cache/\n").unwrap();
        ensure_gitignore_entry(&path).unwrap();
        assert_eq!(
            std::fs::read_to_string(&path).unwrap(),
            "target/\n.rnx-cache/\n"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
