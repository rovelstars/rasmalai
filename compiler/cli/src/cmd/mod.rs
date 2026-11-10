pub(super) fn activate_target(
    root: &std::path::Path,
    requested: Option<&str>,
) -> Result<(sysroot::target::Target, Vec<String>), diagnostics::Diagnostic> {
    let project_targets = match frontend::project::load_manifest(root) {
        Ok(Some(manifest)) => manifest
            .project
            .map(|cfg| cfg.targets)
            .unwrap_or_default(),
        Ok(None) => Vec::new(),
        Err(e) => return Err(e),
    };
    let target = sysroot::target::select_target(requested, &project_targets)
        .map_err(|e| diagnostics::Diagnostic::new(diagnostics::Code::E108, e.to_string()))?;
    sysroot::target::set_active_target(target.clone());
    Ok((target, project_targets))
}

pub(super) struct ResolvedPack {
    pub path: std::path::PathBuf,
    pub digest: Option<String>,
    pub pack: Option<sysroot::pack::Pack>,
}

pub(super) enum RuntimeBytes {
    Embedded,
    Packed(Vec<u8>),
}

pub(super) struct ResolvedRuntime {
    pub bytes: RuntimeBytes,
    pub sha256: String,
    pub abi: u32,
}

impl ResolvedRuntime {
    pub fn bytes(&self) -> &[u8] {
        match &self.bytes {
            RuntimeBytes::Embedded => runtime::archive::BYTES,
            RuntimeBytes::Packed(bytes) => bytes,
        }
    }

    pub fn archive_input(&self) -> linker::LinkInput {
        match &self.bytes {
            RuntimeBytes::Embedded => linker::LinkInput::ArchiveRefKnown {
                bytes: runtime::archive::BYTES,
                sha256: runtime::archive::SHA256,
            },
            RuntimeBytes::Packed(bytes) => linker::LinkInput::ArchiveBytes(bytes.clone()),
        }
    }

    pub fn external_input(&self) -> linker::LinkInput {
        match &self.bytes {
            RuntimeBytes::Embedded => linker::LinkInput::ArchiveRef(runtime::archive::BYTES),
            RuntimeBytes::Packed(bytes) => linker::LinkInput::ArchiveBytes(bytes.clone()),
        }
    }
}

fn pack_error(path: &std::path::Path, message: String) -> diagnostics::Diagnostic {
    diagnostics::Diagnostic::new(
        diagnostics::Code::E124,
        format!("target pack `{}`: {message}", path.display()),
    )
}

pub(super) fn resolve_runtime(
    pack: Option<&ResolvedPack>,
) -> Result<ResolvedRuntime, diagnostics::Diagnostic> {
    let embedded = || ResolvedRuntime {
        bytes: RuntimeBytes::Embedded,
        sha256: runtime::archive::SHA256.to_string(),
        abi: runtime::archive::ABI,
    };
    let Some(resolved) = pack else {
        return Ok(embedded());
    };
    let Some(manifest) = resolved.pack.as_ref() else {
        return Ok(embedded());
    };
    let file = manifest
        .resolve_file(&manifest.manifest.runtime.file)
        .map_err(|e| e.diagnostic())?;
    let bytes = std::fs::read(&file)
        .map_err(|e| pack_error(&resolved.path, format!("cannot read `{}`: {e}", file.display())))?;
    let sha256 = sysroot::pack::file_sha256(&file).map_err(|e| e.diagnostic())?;
    if let Some(expected) = manifest.manifest.runtime.sha256.as_deref()
        && expected != sha256
    {
        return Err(pack_error(
            &resolved.path,
            format!(
                "runtime archive `{}` hashes to {sha256}, but the manifest records {expected}",
                manifest.manifest.runtime.file
            ),
        ));
    }
    let actual = manifest.manifest.runtime.abi;
    if actual != runtime::archive::ABI {
        return Err(pack_error(
            &resolved.path,
            format!(
                "runtime ABI mismatch for target `{}`: expected {}, pack records {actual}",
                manifest.target.triple,
                runtime::archive::ABI
            ),
        ));
    }
    Ok(ResolvedRuntime {
        bytes: RuntimeBytes::Packed(bytes),
        sha256,
        abi: actual,
    })
}

pub(super) fn macho_link_inputs(
    pack: Option<&ResolvedPack>,
    runtime: &ResolvedRuntime,
    user: Vec<linker::LinkInput>,
) -> Result<Vec<linker::macho::MachInput>, diagnostics::Diagnostic> {
    let Some(bundle) = pack.and_then(|pack| pack.pack.as_ref()) else {
        return Err(diagnostics::Diagnostic::new(
            diagnostics::Code::E124,
            "a macOS target pack is required to link a Mach-O image",
        )
        .with_hint("run `rnx sysroot add aarch64-apple-darwin` or `x86_64-apple-darwin`"));
    };
    let read = |name: &str| -> Result<Vec<u8>, diagnostics::Diagnostic> {
        let path = bundle.resolve_file(name).map_err(|e| e.diagnostic())?;
        std::fs::read(&path).map_err(|e| {
            diagnostics::Diagnostic::new(
                diagnostics::Code::E124,
                format!(
                    "target pack `{}`: cannot read `{}`: {e}",
                    bundle.root.display(),
                    path.display()
                ),
            )
        })
    };
    let mut inputs = Vec::new();
    let link = &bundle.manifest.link;
    if !link.crt.is_empty() {
        let names = link.crt.get("static").ok_or_else(|| {
            diagnostics::Diagnostic::new(
                diagnostics::Code::E124,
                format!(
                    "target pack `{}`: link map has no `crt.static` entry",
                    bundle.root.display()
                ),
            )
        })?;
        for name in names {
            inputs.push(linker::macho::MachInput::Object(read(name)?));
        }
    }
    if let Some(names) = link.crt_end.get("static") {
        for name in names {
            inputs.push(linker::macho::MachInput::Object(read(name)?));
        }
    }
    for path in user {
        inputs.push(match path {
            linker::LinkInput::ObjectBytes(bytes) => linker::macho::MachInput::Object(bytes),
            linker::LinkInput::ObjectPath(path) => linker::macho::MachInput::ObjectPath(path),
            linker::LinkInput::ArchiveBytes(bytes) => linker::macho::MachInput::Library(bytes),
            linker::LinkInput::ArchiveRef(bytes) => {
                linker::macho::MachInput::Library(bytes.to_vec())
            }
            linker::LinkInput::ArchiveRefKnown { bytes, .. } => {
                linker::macho::MachInput::Library(bytes.to_vec())
            }
            linker::LinkInput::ArchivePath(path) => {
                linker::macho::MachInput::LibraryPath(path)
            }
            linker::LinkInput::Lib(name) => {
                return Err(diagnostics::Diagnostic::new(
                    diagnostics::Code::E108,
                    format!(
                        "native library `-l{name}` is not supported for Mach-O output; declare a .tbd stub in the target pack"
                    ),
                ));
            }
        });
    }
    for file in link.libs.values().flatten() {
        if file.ends_with(".tbd") {
            let bytes = read(file)?;
            let text = String::from_utf8_lossy(&bytes).into_owned();
            inputs.push(linker::macho::MachInput::Tbd(text, file.clone()));
        } else {
            inputs.push(linker::macho::MachInput::Library(read(file)?));
        }
    }
    if let Some(file) = link.builtins.as_deref() {
        inputs.push(linker::macho::MachInput::Library(read(file)?));
    }
    inputs.push(linker::macho::MachInput::Library(runtime.bytes().to_vec()));
    Ok(inputs)
}

pub(super) fn wasm_link_inputs(
    pack: Option<&ResolvedPack>,
    runtime: &ResolvedRuntime,
    user: Vec<linker::LinkInput>,
) -> Result<Vec<linker::wasm::WasmInput>, diagnostics::Diagnostic> {
    let Some(bundle) = pack.and_then(|pack| pack.pack.as_ref()) else {
        return Err(diagnostics::Diagnostic::new(
            diagnostics::Code::E124,
            "a WASI target pack is required to link a wasm module",
        )
        .with_hint(
            "run `rnx sysroot add --local <pack-dir> wasm32-unknown-wasi` with a pack built from compiler/runtime/wasi",
        ));
    };
    let read = |name: &str| -> Result<std::path::PathBuf, diagnostics::Diagnostic> {
        bundle.resolve_file(name).map_err(|e| e.diagnostic())
    };
    let mut inputs = Vec::new();
    for input in user {
        inputs.push(match input {
            linker::LinkInput::ObjectBytes(bytes) => linker::wasm::WasmInput::Object(bytes),
            linker::LinkInput::ObjectPath(path) => linker::wasm::WasmInput::ObjectPath(path),
            linker::LinkInput::ArchiveBytes(bytes) => linker::wasm::WasmInput::Archive(bytes),
            linker::LinkInput::ArchiveRef(bytes) => {
                linker::wasm::WasmInput::Archive(bytes.to_vec())
            }
            linker::LinkInput::ArchiveRefKnown { bytes, .. } => {
                linker::wasm::WasmInput::Archive(bytes.to_vec())
            }
            linker::LinkInput::ArchivePath(path) => linker::wasm::WasmInput::ArchivePath(path),
            linker::LinkInput::Lib(name) => {
                return Err(diagnostics::Diagnostic::new(
                    diagnostics::Code::E108,
                    format!(
                        "native library `-l{name}` is not supported for wasm output; declare an archive in the target pack"
                    ),
                ));
            }
        });
    }
    let link = &bundle.manifest.link;
    for group in [&link.crt, &link.crt_end] {
        for paths in group.values() {
            for name in paths {
                inputs.push(linker::wasm::WasmInput::ArchivePath(read(name)?));
            }
        }
    }
    for name in link.libs.values().flatten() {
        inputs.push(linker::wasm::WasmInput::ArchivePath(read(name)?));
    }
    if let Some(name) = link.builtins.as_deref() {
        inputs.push(linker::wasm::WasmInput::ArchivePath(read(name)?));
    }
    inputs.push(linker::wasm::WasmInput::Archive(runtime.bytes().to_vec()));
    Ok(inputs)
}

pub(super) fn pe_link_inputs(
    pack: Option<&ResolvedPack>,
    runtime: &ResolvedRuntime,
    user: Vec<linker::LinkInput>,
) -> Result<Vec<linker::pe::PeInput>, diagnostics::Diagnostic> {
    let Some(bundle) = pack.and_then(|pack| pack.pack.as_ref()) else {
        return Err(diagnostics::Diagnostic::new(
            diagnostics::Code::E124,
            "a PE target pack is required to link a PE image",
        )
        .with_hint("run `rnx sysroot add x86_64-pc-windows-gnu` or `rnx sysroot add x86_64-unknown-uefi`"));
    };
    let read = |name: &str| -> Result<Vec<u8>, diagnostics::Diagnostic> {
        let path = bundle.resolve_file(name).map_err(|e| e.diagnostic())?;
        std::fs::read(&path).map_err(|e| {
            diagnostics::Diagnostic::new(
                diagnostics::Code::E124,
                format!(
                    "target pack `{}`: cannot read `{}`: {e}",
                    bundle.root.display(),
                    path.display()
                ),
            )
        })
    };
    let mut inputs = Vec::new();
    let link = &bundle.manifest.link;
    if !link.crt.is_empty() {
        let names = link.crt.get("static").ok_or_else(|| {
            diagnostics::Diagnostic::new(
                diagnostics::Code::E124,
                format!(
                    "target pack `{}`: link map has no `crt.static` entry",
                    bundle.root.display()
                ),
            )
        })?;
        for name in names {
            inputs.push(linker::pe::PeInput::Object(read(name)?));
        }
    }
    if let Some(names) = link.crt_end.get("static") {
        for name in names {
            inputs.push(linker::pe::PeInput::Object(read(name)?));
        }
    }
    for path in user {
        inputs.push(match path {
            linker::LinkInput::ObjectBytes(bytes) => linker::pe::PeInput::Object(bytes),
            linker::LinkInput::ObjectPath(path) => linker::pe::PeInput::ObjectPath(path),
            linker::LinkInput::ArchiveBytes(bytes) => linker::pe::PeInput::Library(bytes),
            linker::LinkInput::ArchiveRef(bytes) => {
                linker::pe::PeInput::Library(bytes.to_vec())
            }
            linker::LinkInput::ArchiveRefKnown { bytes, .. } => {
                linker::pe::PeInput::Library(bytes.to_vec())
            }
            linker::LinkInput::ArchivePath(path) => linker::pe::PeInput::LibraryPath(path),
            linker::LinkInput::Lib(name) => {
                return Err(diagnostics::Diagnostic::new(
                    diagnostics::Code::E108,
                    format!(
                        "native library `-l{name}` is not supported for PE output; declare an import library in the target pack"
                    ),
                ));
            }
        });
    }
    for file in link.libs.values().flatten() {
        inputs.push(linker::pe::PeInput::Library(read(file)?));
    }
    if let Some(file) = link.builtins.as_deref() {
        inputs.push(linker::pe::PeInput::Library(read(file)?));
    }
    inputs.push(linker::pe::PeInput::Library(runtime.bytes().to_vec()));
    Ok(inputs)
}

fn load_pack(path: &std::path::Path) -> Result<sysroot::pack::Pack, diagnostics::Diagnostic> {
    sysroot::pack::Pack::load(path).map_err(|e| e.diagnostic())
}

pub(super) fn resolve_target_pack(
    root: &std::path::Path,
    target: &sysroot::target::Target,
    host: &str,
) -> Result<Option<ResolvedPack>, diagnostics::Diagnostic> {
    if let Some(custom) = frontend::sysroot::configured_root() {
        sysroot::Sysroot::custom(custom.clone())
            .validate()
            .map_err(|e| {
                diagnostics::Diagnostic::new(
                    diagnostics::Code::E124,
                    format!("target pack `{}`: {e}", custom.display()),
                )
            })?;
        let pack = if sysroot::pack::read_manifest(&custom).is_ok() {
            Some(load_pack(&custom)?)
        } else {
            None
        };
        let digest = pack
            .as_ref()
            .map(|pack| pack.manifest.integrity.digest.clone());
        return Ok(Some(ResolvedPack {
            path: custom,
            digest,
            pack,
        }));
    }
    let store = sysroot::store::Store::default_store();
    let host_target = target.triple == host;
    let pin = frontend::deplock::ProjectDepLock::load(root)
        .ok()
        .flatten()
        .and_then(|lock| lock.sysroots.get(&target.triple).cloned());
    let installed = if let Some(pin) = pin {
        Some(store.verify_digest(&target.triple, &pin).map_err(|e| {
            e.with_hint(format!(
                "run `rnx sysroot add {}`, then `rnx lock` to record the installed digest",
                target.triple
            ))
        })?)
    } else if host_target {
        // D23: the embedded archive is the host fast path. An installed but
        // unpinned host pack must not silently replace it.
        None
    } else {
        store.current(&target.triple)
    };
    if let Some(installed) = installed {
        let pack = load_pack(&installed.path)?;
        return Ok(Some(ResolvedPack {
            path: installed.path,
            digest: Some(installed.digest),
            pack: Some(pack),
        }));
    }
    if host_target {
        return Ok(None);
    }
    if sysroot::store::fetch_disabled() {
        return Err(diagnostics::Diagnostic::new(
            diagnostics::Code::E124,
            format!("no target pack installed for `{}`", target.triple),
        )
        .with_hint(format!(
            "run `rnx sysroot add {}` (network access is disabled by --offline or CI)",
            target.triple
        )));
    }
    let bare = frontend::oci::sysroot_oci();
    let tag = frontend::oci::sysroot_tag(env!("CARGO_PKG_VERSION"), &target.triple);
    let download = frontend::oci::download_sysroot(&bare, &tag, &target.triple, None)?;
    let installed = store.install_from_dir(&download.dir, Some(&download.digest));
    let _ = std::fs::remove_dir_all(&download.dir);
    let installed = installed
        .map_err(|e| e.with_hint(format!("run `rnx sysroot add {}`", target.triple)))?;
    let pack = load_pack(&installed.path)?;
    Ok(Some(ResolvedPack {
        path: installed.path,
        digest: Some(installed.digest),
        pack: Some(pack),
    }))
}

fn cwd_path() -> std::path::PathBuf {
    std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."))
}

fn project_root_for_path(path: Option<&std::path::Path>) -> std::path::PathBuf {
    let start = match path {
        Some(p) if p.is_file() => p.parent().unwrap_or(std::path::Path::new(".")).to_path_buf(),
        Some(p) => p.to_path_buf(),
        None => cwd_path(),
    };
    frontend::project::find_project_root(&start).unwrap_or(start)
}

pub(super) fn target_selection_input(
    command: &cli::args::Command,
) -> Option<(std::path::PathBuf, Option<String>)> {
    match command {
        cli::args::Command::Build { path, target, .. } => {
            Some((project_root_for_path(path.as_deref()), target.clone()))
        }
        cli::args::Command::Run { path, .. } => {
            Some((project_root_for_path(path.as_deref()), None))
        }
        cli::args::Command::Check { paths, .. } => {
            let path = paths.first().map(std::path::PathBuf::as_path);
            Some((project_root_for_path(path), None))
        }
        _ => None,
    }
}

pub(super) fn resolve_context_root(dir: &std::path::Path) -> Option<std::path::PathBuf> {
    frontend::project::find_workspace_root(dir)
        .or_else(|| frontend::project::find_project_root(dir))
        .or_else(|| {
            if dir.join(frontend::project::MANIFEST_FILE).is_file() {
                Some(dir.to_path_buf())
            } else {
                None
            }
        })
}

fn parse_opt_level(s: &str) -> u8 {
    match s {
        "0" => 0,
        "1" => 1,
        _ => {
            eprintln!("opt level must be 0 or 1");
            std::process::exit(2);
        }
    }
}

fn open_browser(path: &std::path::Path) -> std::io::Result<()> {
    #[cfg(target_os = "macos")]
    let prog = "open";
    #[cfg(not(target_os = "macos"))]
    let prog = "xdg-open";
    std::process::Command::new(prog).arg(path).spawn().map(|_| ())
}

fn backend_name(backend: &cli::TestBackend) -> &'static str {
    match backend {
        cli::TestBackend::Interpreter => "interpreter",
        cli::TestBackend::Cranelift => "cranelift",
        cli::TestBackend::Llvm => "llvm",
    }
}

fn bus_name(stage: &str) -> &str {
    match stage {
        "lex_parse" => "parse",
        "typecheck" => "typecheck",
        "lower" => "ir-lower",
        "opt_pipeline" => "opt",
        "verify" => "verify",
        "codegen" => "codegen",
        other => other,
    }
}

fn bus_detail(stage: &str) -> &str {
    match stage {
        "lex_parse" => "lex & parse sources",
        "typecheck" => "soundness & borrow analysis",
        "lower" => "lir lowering",
        "opt_pipeline" => "optimization pipeline",
        "verify" => "ir verification",
        "codegen" => "native object emission",
        _ => "",
    }
}

mod add;
mod audit;
mod bench;
mod bindgen;
mod build;
mod cache;
mod check;
mod completions;
mod cputier;
mod dev;
mod doc;
mod doctor;
mod explain;
pub(super) mod fetch;
pub(super) mod download;

mod fmt;
mod init;
mod lint;
mod lock;
mod lsp;
mod mcp;
mod pack;
mod publish;
mod repl;
mod run;
mod setup;
mod sysroots;
mod target;
mod task;
mod test;
mod unpack;
mod unpublish;
mod update;
mod vendor;
mod x;

use self::add::run_add;
use self::audit::run_audit;
use self::bench::run_bench;
use self::bindgen::run_bindgen;
use self::build::run_build;
use self::cache::{run_cache, run_clean};
use self::check::run_check;
use self::completions::run_completions;
use self::cputier::run_cpu_tier;
use self::dev::run_dev;
use self::doc::run_doc;
use self::doctor::run_doctor;
use self::explain::run_explain;
use self::fetch::run_fetch;
use self::download::run_download;
use self::fmt::run_fmt;
use self::init::run_init;
use self::lint::run_lint;
use self::lock::run_lock;
use self::lsp::run_lsp;
use self::mcp::run_mcp;
use self::pack::run_pack;
use self::publish::run_publish;
use self::repl::run_repl;
use self::run::run_run;
use self::setup::run_setup;
use self::sysroots::run_sysroot;
use self::target::run_target;
use self::task::run_task;
use self::test::run_test;
use self::unpack::run_unpack;
use self::unpublish::run_unpublish;
use self::update::run_update;
use self::vendor::run_vendor;
use self::x::run_x;

pub(super) fn dispatch(
    command: cli::args::Command,
    verbose: bool,
    quiet: bool,
    no_color: bool,
    prefer: frontend::native::NativePrefer,
) {
    match command {
            cli::args::Command::Repl => {
                run_repl();
            }
            cli::args::Command::Dev { path, entry, no_rerun, mcp } => {
                run_dev(path, entry, no_rerun, mcp);
            }
            cli::args::Command::Check {
                paths,
                package,
                json,
                expand_macros,
            } => {
                run_check(paths, package, json, expand_macros, verbose, quiet);
            }
            cli::args::Command::Run {
                path,
                entry,
                package,
                context,
                locked,
                opt_level,
                backend,
                time_passes,
                trace,
                perf_map,
                debug,
                jobs,
                memory_cap,
                expand_macros,
                args: rest,
            } => {
                run_run(path, entry, package, context, locked, opt_level, backend, time_passes, trace, perf_map, debug, jobs, memory_cap, expand_macros, verbose, rest, prefer);
            }
            cli::args::Command::Explain { code } => {
                run_explain(code);
            }
            cli::args::Command::CpuTier => {
                run_cpu_tier();
            }
            cli::args::Command::Build {
                path,
                entry,
                release,
                lib,
                lib_prefix,
                emit_obj,
                target: target_triple,
                locked,
                opt_level,
                time_passes,
                trace,
                perf_map,
                debug,
                jobs,
                memory_cap,
                package,
                context,
                deny_perf_lints,
                show_perf_notes,
                fix,
                fix_force,
                fix_keep_backup,
                expand_macros,
            } => {
                run_build(path, entry, release, lib, lib_prefix, emit_obj, target_triple, locked, opt_level, time_passes, trace, perf_map, debug, jobs, memory_cap, package, context, deny_perf_lints, show_perf_notes, fix, fix_force, fix_keep_backup, expand_macros, verbose, quiet, prefer);
            }
            cli::args::Command::Init { name } => {
                run_init(name);
            }
            cli::args::Command::Lsp => {
                run_lsp();
            }
            cli::args::Command::Lint {
                paths,
                package,
                sarif,
                json,
                deny_warnings,
                fix,
            } => {
                run_lint(paths, package, sarif, json, deny_warnings, fix);
            }
            cli::args::Command::Fmt { paths, check, diff } => {
                run_fmt(paths, check, diff);
            }
            cli::args::Command::Lock { package, target } => {
                run_lock(package, target, prefer);
            }
            cli::args::Command::Update { packages, major, package } => {
                run_update(packages, major, package, prefer);
            }
            cli::args::Command::Fetch { package } => {
                run_fetch(package);
            }
            cli::args::Command::Install { package } => {
                run_lock(package.clone(), None, prefer);
                run_fetch(package);
            }
            cli::args::Command::X { package, bin, yes, args } => {
                run_x(package, bin, yes, args);
            }
            cli::args::Command::Download { packages, local, registry } => {
                run_download(packages, local, registry);
            }
            cli::args::Command::Doctor { repair_std, registry } => {
                run_doctor(repair_std, registry);
            }
            cli::args::Command::Doc {
                package,
                open,
                no_deps,
                all,
                json,
                out_dir,
                stdlib,
            } => {
                run_doc(package, open, no_deps, all, json, out_dir, stdlib, quiet);
            }
            cli::args::Command::Pack {
                package,
                out_dir,
                gzip,
            } => {
                run_pack(package, out_dir, gzip);
            }
            cli::args::Command::Publish { registry, token, artifact } => {
                run_publish(registry, token, artifact);
            }
            cli::args::Command::Unpublish { target, registry, token } => {
                run_unpublish(target, registry, token);
            }
            cli::args::Command::Unpack { archive, out_dir } => {
                run_unpack(archive, out_dir);
            }
            cli::args::Command::Vendor { packages, dir } => {
                run_vendor(packages, dir);
            }
            cli::args::Command::Audit {
                path,
                json,
                export_manifest,
                licenses,
                deny_license,
                registry,
            } => {
                run_audit(path, json, export_manifest, licenses, deny_license, registry);
            }
            cli::args::Command::Add { package, path, accept_caps, accept_all_caps } => {
                run_add(package, path, accept_caps, accept_all_caps);
            }
            cli::args::Command::Bench {                package,
                filter,
                backend,
                release: _,
                no_release,
                time_passes,
                trace,
                perf_map,
                debug,
                jobs,
                memory_cap,
            } => {
                run_bench(package, filter, backend, no_release, time_passes, trace, perf_map, debug, jobs, memory_cap, verbose);
            }
            cli::args::Command::Test {
                filter,
                package,
                opt_level,
                backend,
                exact,
                nocapture,
                jobs,
                memory_cap,
            } => {
                run_test(filter, package, opt_level, backend, exact, nocapture, jobs, memory_cap, verbose, quiet, no_color);
            }
            cli::args::Command::Task { name, package, all, explain, allow, force, watch } => {
                run_task(name, package, all, explain, allow, force, watch, prefer);
            }
            cli::args::Command::Setup { editor } => {
                run_setup(editor);
            }
            cli::args::Command::Completions { shell } => {
                run_completions(shell);
            }
            cli::args::Command::Mcp => {
                run_mcp();
            }
            cli::args::Command::Bindgen {
                header,
                native_lib,
                pkg_name,
                out,
                target,
                prefix,
                skip,
                include,
                case,
                rename,
                upstream_name,
                check,
            } => {
                run_bindgen(header, native_lib, pkg_name, out, target, prefix, skip, include, case, rename, upstream_name, check);
            }
            cli::args::Command::Target { action } => {
                run_target(action);
            }
            cli::args::Command::Sysroot { action } => {
                run_sysroot(action);
            }
            cli::args::Command::Cache { action } => {
                run_cache(action);
            }
            cli::args::Command::Clean { package } => {
                run_clean(package);
            }
    }
}
