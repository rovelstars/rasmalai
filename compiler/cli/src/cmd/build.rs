use super::*;
use sysroot::target::{Arch as TargetArch, Libc as TargetLibc, Os as TargetOs};

fn profile_name(release: bool) -> &'static str {
    if release { "release" } else { "dev" }
}

fn build_key(
    input: &str,
    root: &std::path::Path,
    entry: &str,
    release: bool,
    opt_level: u8,
    target_triple: Option<&str>,
    debug: bool,
    jobs: usize,
    memory_cap: u64,
    prefer: frontend::native::NativePrefer,
    bundled: &[std::path::PathBuf],
    pack_digest: Option<&str>,
    runtime_sha256: &str,
    runtime_abi: u32,
) -> String {
    // Path-dep manifests and path-dep src trees (src/**/*.rnx, skipping
    // target/, .git/, .rnx-cache/) feed the fingerprint. Files outside a
    // path dep's src/ tree or without the .rnx extension are still ignored.
    let manifest = frontend::project::load_manifest(root)
        .ok()
        .flatten()
        .and_then(|m| m.project);
    let mut owned: Vec<Vec<u8>> = Vec::new();
    owned.push(std::fs::read(input).unwrap_or_default());
    owned.push(std::fs::read(root.join(frontend::project::MANIFEST_FILE)).unwrap_or_default());
    owned.push(std::fs::read(root.join("Project.deplock")).unwrap_or_default());
    owned.push(frontend::cache::hash_src_tree(root));
    if let Some(cfg) = &manifest {
        for (name, target) in &cfg.entries.bins {
            owned.push(name.as_bytes().to_vec());
            owned.push(std::fs::read(root.join(target)).unwrap_or_default());
        }
        if let Some(lib) = cfg.entries.lib.as_deref() {
            owned.push(lib.as_bytes().to_vec());
            owned.push(std::fs::read(root.join(lib)).unwrap_or_default());
        }
        if let Some(cabi) = cfg.entries.cabi.as_deref() {
            owned.push(cabi.as_bytes().to_vec());
            owned.push(std::fs::read(root.join(cabi)).unwrap_or_default());
        }
        for (name, spec) in &cfg.dependencies {
            let path = match spec {
                frontend::project::DependencySpec::Path { path, .. } => Some(path.clone()),
                frontend::project::DependencySpec::Alias { .. } => {
                    let scope = frontend::project::find_workspace_root(root)
                        .unwrap_or_else(|| root.to_path_buf());
                    match frontend::modules::resolve_alias_spec(
                        &scope,
                        cfg,
                        name,
                        &mut Vec::new(),
                    ) {
                        Ok((_, frontend::project::DependencySpec::Path { path, .. })) => {
                            Some(path)
                        }
                        _ => None,
                    }
                }
                _ => None,
            };
            if let Some(path) = path {
                owned.push(name.as_bytes().to_vec());
                let dep_root = root.join(path);
                owned.push(
                    std::fs::read(dep_root.join(frontend::project::MANIFEST_FILE))
                        .unwrap_or_default(),
                );
                owned.push(frontend::cache::hash_src_tree(&dep_root));
            }
        }
    }
    owned.push(entry.as_bytes().to_vec());
    owned.push(format!("jobs={jobs} memory-cap={memory_cap}").into_bytes());
    owned.push(format!("prefer={prefer:?}").into_bytes());
    let mut bundled: Vec<&std::path::PathBuf> = bundled.iter().collect();
    bundled.sort();
    for path in bundled {
        owned.push(path.display().to_string().into_bytes());
        owned.push(std::fs::read(path).unwrap_or_default());
    }
    if let Ok(vendor) = std::env::var("RNX_VENDOR_DIR") {
        if !vendor.is_empty() {
            let vendor_root = std::path::Path::new(&vendor);
            owned.push(vendor.as_bytes().to_vec());
            owned.push(frontend::cache::hash_src_tree(vendor_root));
        }
    }
    owned.push(sysroot::Sysroot::discover(None).identity().into_bytes());
    if let Some(digest) = pack_digest {
        owned.push(format!("target-pack={digest}").into_bytes());
    }
    owned.push(format!("runtime-sha256={runtime_sha256}").into_bytes());
    owned.push(format!("runtime-abi={runtime_abi}").into_bytes());
    let host = linker::host_triple().0;
    owned.extend(frontend::cache::toolchain_parts(
        release,
        opt_level,
        target_triple,
        &host,
        debug,
        env!("CARGO_PKG_VERSION"),
        cli::runtime_archive_hash(),
        llvm::LLVM_VERSION,
    ));
    let parts: Vec<&[u8]> = owned.iter().map(|b| b.as_slice()).collect();
    frontend::cache::fingerprint_hex(&parts)
}

fn project_name_invalid(name: &str) -> bool {
    if name.is_empty() || name.contains("..") || name.contains('\\') {
        return true;
    }
    if name.chars().any(|c| c.is_control()) {
        return true;
    }
    if name.starts_with('@') {
        return !frontend::project::is_package_name(name);
    }
    name.contains('/')
}

fn validate_project_name(root: &std::path::Path) {
    let name = frontend::project::load_manifest(root)
        .ok()
        .flatten()
        .and_then(|m| m.project.map(|p| p.name));
    if let Some(name) = name
        && project_name_invalid(&name)
    {
        println!(
            "{}",
            diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!("invalid project name `{name}` for build output"),
            )
        );
        std::process::exit(1);
    }
}

fn default_out_path(
    input: &str,
    root: &std::path::Path,
    release: bool,
    suffix: &str,
) -> std::path::PathBuf {
    let stem = frontend::project::load_manifest(root)
        .ok()
        .flatten()
        .and_then(|m| m.project.map(|p| p.name))
        .map(|n| cli::sanitize_lib_name(&n))
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| {
            std::path::Path::new(input)
                .file_stem()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_else(|| "a.out".to_string())
        });
    frontend::cache::project_cache_dir(root)
        .join("build")
        .join(profile_name(release))
        .join(format!("{stem}{suffix}"))
}

fn profile_artifact(root: &std::path::Path, release: bool, filename: &str) -> std::path::PathBuf {
    let dir = frontend::cache::project_cache_dir(root)
        .join("build")
        .join(profile_name(release));
    if let Err(e) = std::fs::create_dir_all(&dir) {
        eprintln!("error: cannot create {}: {e}", dir.display());
        std::process::exit(1);
    }
    dir.join(filename)
}

fn project_root_for(input: &str) -> std::path::PathBuf {
    let anchor = std::path::Path::new(input)
        .parent()
        .map(|p| p.to_path_buf())
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    match frontend::project::find_project_root(&anchor) {
        Some(root) => root,
        None => {
            if anchor.is_absolute() {
                anchor
            } else {
                std::env::current_dir()
                    .map(|cwd| cwd.join(anchor))
                    .unwrap_or_else(|_| std::path::PathBuf::from("."))
            }
        }
    }
}
fn env_flag_enabled(name: &str) -> bool {
    match std::env::var(name) {
        Ok(v) => matches!(v.trim().to_ascii_lowercase().as_str(), "1" | "true" | "yes" | "on"),
        Err(_) => false,
    }
}

pub(super) fn run_build(path: Option<std::path::PathBuf>, entry: String, release: bool, lib: bool, lib_prefix: Option<String>, emit_obj: bool, target_triple: Option<String>, locked: bool, opt_level: String, time_passes: bool, trace: Option<std::path::PathBuf>, perf_map: bool, debug: bool, jobs: Option<usize>, memory_cap: Option<u64>, package: Option<String>, context: Option<std::path::PathBuf>, deny_perf_lints: Vec<String>, show_perf_notes: bool, fix: bool, fix_force: bool, fix_keep_backup: bool, expand_macros: bool, verbose: bool, quiet: bool, prefer: frontend::native::NativePrefer) {
            let path = path.map(|p| p.to_string_lossy().into_owned());
            let opt_level = parse_opt_level(&opt_level);
            let deny_raw: Vec<String> = if deny_perf_lints.is_empty() {
                std::env::var("RNX_DENY_PERF_LINTS").map(|v| vec![v]).unwrap_or_default()
            } else {
                deny_perf_lints
            };
            let deny = match cli::report::parse_deny_perf(&deny_raw) {
                Ok(d) => d,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(2);
                }
            };
            if !deny.is_empty() && (lib || emit_obj) {
                eprintln!("error: --deny-perf-lints is only supported for executable builds");
                std::process::exit(2);
            }
            if lib_prefix.is_some() && !lib {
                eprintln!("error: --lib-prefix needs --lib");
                std::process::exit(2);
            }
            if fix && (lib || emit_obj) {
                eprintln!("error: --fix is only supported for executable builds");
                std::process::exit(2);
            }
            if fix && deny.denies("P201") {
                eprintln!("error: --fix cannot be combined with --deny-perf-lints for P201 (run `build --fix` first, then `build --deny-perf-lints`)");
                std::process::exit(2);
            }
            let show_notes = show_perf_notes || env_flag_enabled("RNX_SHOW_PERF_NOTES");
            let mut target = cli::resolve_scope_target(path.as_deref(), package.as_deref())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if let Some(dir) = context.as_deref() {
                match resolve_context_root(dir) {
                    Some(root) => target.scope_root = Some(root),
                    None => {
                        eprintln!("error: --context {} has no Project.config", dir.display());
                        std::process::exit(1);
                    }
                }
            }
            let input = target.entry;
            let root = project_root_for(&input);
            let scope_root = target.scope_root.clone().unwrap_or_else(|| root.clone());
            validate_project_name(&root);
            if verbose {
                eprintln!("rnx: building {input} ({})", if release { "release" } else { "dev" });
            }
            let security_issues =
                frontend::security::verify_entry(std::path::Path::new(&input));
            if !security_issues.is_empty() {
                eprint!("{}", cli::report::render_security_issues(&security_issues));
                std::process::exit(1);
            }
            let (selected_target, project_targets) =
                match super::activate_target(&root, target_triple.as_deref()) {
                    Ok(target) => target,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
            let compile_target_owned = (target_triple.is_some() || !project_targets.is_empty())
                .then(|| selected_target.triple.clone());
            let compile_target = compile_target_owned.as_deref();
            let host_target = linker::host_target().ok();
            let host = host_target
                .as_ref()
                .map(|target| target.triple.clone())
                .unwrap_or_else(|| linker::host_triple().0);
            let effective = selected_target.triple.as_str();
            let cross = effective != host;
            let host_linkable_musl = selected_target.libc == TargetLibc::Musl
                && selected_target.arch == TargetArch::X86_64
                && host_target.as_ref().is_some_and(|target| {
                    target.arch == TargetArch::X86_64 && target.os == TargetOs::Linux
                });
            let object_coff = selected_target.object == sysroot::target::ObjectFormat::Coff;
            let object_macho = selected_target.object == sysroot::target::ObjectFormat::MachO;
            let object_wasm = selected_target.object == sysroot::target::ObjectFormat::Wasm;
            if lib && object_wasm {
                eprintln!(
                    "{}",
                    diagnostics::Diagnostic::new(
                        diagnostics::Code::E108,
                        "`--lib` is not supported for wasm targets; build a command module instead",
                    )
                );
                std::process::exit(1);
            }
            let native_cross = (selected_target.arch == TargetArch::Aarch64
                && selected_target.os == TargetOs::Linux)
                || object_coff
                || object_macho
                || object_wasm;
            if cross && !host_linkable_musl && !native_cross && !emit_obj {
                eprintln!(
                    "{}",
                    diagnostics::Diagnostic::new(
                        diagnostics::Code::E108,
                        "cross targets need --emit-obj; linking runs on the host",
                    )
                );
                std::process::exit(1);
            }
            let resolved_pack = if emit_obj {
                None
            } else {
                match super::resolve_target_pack(&root, &selected_target, &host) {
                    Ok(pack) => pack,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                }
            };
            let resolved_runtime = match super::resolve_runtime(resolved_pack.as_ref()) {
                Ok(runtime) => runtime,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            let link_sysroot = match &resolved_pack {
                Some(pack) => sysroot::Sysroot::custom(pack.path.clone()),
                None => sysroot::Sysroot::discover(None),
            };
            let pack_selected = resolved_pack
                .as_ref()
                .is_some_and(|pack| pack.pack.is_some());
            let runtime_abi = resolved_pack
                .as_ref()
                .and_then(|pack| pack.pack.as_ref())
                .map(|pack| pack.manifest.runtime.abi)
                .unwrap_or(runtime::archive::ABI);
            if let Some(pack) = &resolved_pack {
                if verbose {
                    eprintln!("rnx: target pack {}", pack.path.display());
                }
                unsafe {
                    std::env::set_var("RNX_TARGET_PACK", &pack.path);
                    match pack.digest.as_deref() {
                        Some(digest) => std::env::set_var("RNX_TARGET_PACK_DIGEST", digest),
                        None => std::env::remove_var("RNX_TARGET_PACK_DIGEST"),
                    }
                }
            }
            if locked {
                let locked_ok = match &target.scope_root {
                    Some(root) => cli::enforce_locked_at_with(root, prefer),
                    None => Err(cli::missing_lock()),
                };
                if let Err(e) = locked_ok {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
            if perf_map {
                eprintln!("warning: --perf-map applies to JIT execution; ignored for `build`");
            }
            let cfg = cli::CompileConfig {
                time_passes,
                trace,
                perf_map: false,
                debug,
                jobs: cli::resolve_jobs(jobs),
                memory_cap: cli::resolve_memory_cap(memory_cap),
                expand_macros,
            };
            if verbose {
                eprintln!("rnx: jobs={} memory-cap={} bytes", cfg.jobs, cfg.memory_cap);
            }
            if fix {
                let req = cli::fix::FixRequest {
                    input: input.clone(),
                    root: root.clone(),
                    entry: entry.clone(),
                    release,
                    opt_level,
                    package: package.clone(),
                    locked,
                    fix_force,
                    fix_keep_backup,
                    cfg: cfg.clone(),
                };
                match cli::fix::run_fix(&req) {
                    cli::fix::FixOutcome::Done => {}
                    cli::fix::FixOutcome::Exit(code) => std::process::exit(code),
                }
            }
            if lib {
                let theme = diagnostics::theme::AuraTheme::active();
                let mut input = input.clone();
                let anchor = std::path::Path::new(&input)
                    .parent()
                    .map(|p| p.to_path_buf())
                    .unwrap_or_else(|| std::path::PathBuf::from("."));
                if let Some(root) = frontend::project::find_project_root(&anchor)
                    && let Ok(Some(cfg)) = frontend::project::ProjectConfig::load_from_dir(&root)
                {
                    let surface = cfg
                        .cabi_path(&root)
                        .or_else(|| cfg.entries.lib.as_deref().map(|l| root.join(l)));
                    if let Some(surface) = surface {
                        input = surface.to_string_lossy().into_owned();
                    }
                }
                if let Some(prefix) = lib_prefix.as_deref()
                    && !frontend::header::is_valid_prefix(prefix)
                {
                    eprintln!(
                        "{}",
                        diagnostics::Diagnostic::new(
                            diagnostics::Code::E108,
                            format!("invalid --lib-prefix `{prefix}`; use [A-Za-z_][A-Za-z0-9_]*"),
                        )
                    );
                    std::process::exit(1);
                }
                let built = match cli::build_lib_files_cfg(&input, release, opt_level, compile_target, lib_prefix.as_deref().unwrap_or(""), &cfg) {
                    Ok(b) => b,
                    Err(errs) => {
                        eprint!(
                            "{}",
                            cli::report::render_compile_errors(
                                &theme,
                                std::path::Path::new(&input),
                                &errs
                            )
                        );
                        std::process::exit(1);
                    }
                };
                for warning in &built.warnings {
                    eprintln!("{warning}");
                }
                if emit_obj {
                    let obj_path = profile_artifact(&root, release, &format!("lib{}.o", built.package));
                    if let Err(e) = std::fs::write(&obj_path, &built.object) {
                        eprintln!("error: write {}: {e}", obj_path.display());
                        std::process::exit(1);
                    }
                    let header_path = obj_path.with_extension("h");
                    if let Err(e) = std::fs::write(&header_path, &built.header) {
                        eprintln!("error: header {}: {e}", header_path.display());
                        std::process::exit(1);
                    }
                    println!("artifact: {}", obj_path.display());
                    return;
                }
                let out_path = profile_artifact(&root, release, &format!("lib{}.a", built.package));
                if let Err(e) = linker::bundle_static_library(
                    &built.object,
                    &runtime::archive::BYTES,
                    std::path::Path::new(&out_path),
                ) {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
                let header_path = std::path::Path::new(&out_path).with_extension("h");
                if let Err(e) = std::fs::write(&header_path, &built.header) {
                    eprintln!("error: header {}: {e}", header_path.display());
                    std::process::exit(1);
                }
                println!("artifact: {}", out_path.display());
                return;
            }
            if emit_obj && !lib {
                let theme = diagnostics::theme::AuraTheme::active();
                let bytes = match cli::build_files_cfg(
                    &input,
                    &entry,
                    release,
                    opt_level,
                    compile_target,
                    &cfg,
                ) {
                    Ok(b) => b,
                    Err(errs) => {
                        eprint!(
                            "{}",
                            cli::report::render_compile_errors(
                                &theme,
                                std::path::Path::new(&input),
                                &errs
                            )
                        );
                        std::process::exit(1);
                    }
                };
                let obj_path = profile_artifact(&root, release, &format!("{}.o", cli::package_name_for(&input)));
                if let Err(e) = std::fs::write(&obj_path, &bytes) {
                    eprintln!("error: write {}: {e}", obj_path.display());
                    std::process::exit(1);
                }
                println!("artifact: {}", obj_path.display());
                return;
            }
            let wall = std::time::Instant::now();
            let theme = diagnostics::theme::AuraTheme::active();
            let built = match cli::build_files_staged_cfg(&input, &entry, release, opt_level, compile_target, &cfg) {
                Ok(b) => b,
                Err(errs) => {
                    eprint!(
                        "{}",
                        cli::report::render_compile_errors(
                            &theme,
                            std::path::Path::new(&input),
                            &errs
                        )
                    );
                    std::process::exit(1);
                }
            };
            let perf_diags: Vec<diagnostics::Diagnostic> = built
                .perf_diags
                .iter()
                .filter(|d| d.code.is_perf_lint())
                .cloned()
                .collect();
            if !perf_diags.is_empty() || !deny.is_empty() {
                let rendered = cli::report::render_perf_stream(
                    &theme,
                    std::path::Path::new(&input),
                    &perf_diags,
                    &deny,
                    show_notes,
                );
                if !rendered.text.is_empty() {
                    eprint!("{}", rendered.text);
                }
                if !rendered.denied.is_empty() {
                    let mut codes: Vec<String> = rendered
                        .denied
                        .iter()
                        .map(|d| d.code.as_str().to_string())
                        .collect();
                    codes.sort();
                    codes.dedup();
                    eprintln!(
                        "error: build denied by --deny-perf-lints ({}: {} diagnostic(s))",
                        codes.join(", "),
                        rendered.denied.len()
                    );
                    std::process::exit(1);
                }
            }
            let link_plan = match frontend::native::resolve_native_links(&scope_root, prefer, &built.native_libs) {
                Ok(plan) => plan,
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            };
            let bundled_paths: Vec<std::path::PathBuf> = link_plan
                .iter()
                .filter_map(|n| n.archive.clone())
                .collect();
            let (out_path, pending_stamp) = {
                    let path = default_out_path(
                        &input,
                        &root,
                        release,
                        if object_wasm { ".wasm" } else { "" },
                    );
                    if let Some(parent) = path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            eprintln!("error: cannot create {}: {e}", parent.display());
                            std::process::exit(1);
                        }
                    }
                    let key = build_key(&input, &root, &entry, release, opt_level, compile_target, debug, cfg.jobs, cfg.memory_cap, prefer, &bundled_paths, resolved_pack.as_ref().and_then(|pack| pack.digest.as_deref()), &resolved_runtime.sha256, runtime_abi);
                    let stamp = match path.file_name().map(|s| s.to_string_lossy().into_owned()) {
                        Some(stem) => path.with_file_name(format!("{stem}.fingerprint")),
                        None => path.with_file_name(".fingerprint"),
                    };
                    let fresh = std::fs::read_to_string(&stamp).map(|s| s.trim() == key).unwrap_or(false);
                    if fresh && path.is_file() {
                        println!("artifact: {} (fresh)", path.display());
                        return;
                    }
                    (path.display().to_string(), Some((stamp, key)))
            };
            let target = linker::TargetTriple(effective.to_string());
            let link_start = std::time::Instant::now();
            let foreign_libs: Vec<linker::LinkInput> = link_plan
                .iter()
                .map(|n| match &n.archive {
                    Some(path) => linker::LinkInput::ArchivePath(path.clone()),
                    None => linker::LinkInput::Lib(n.lib.clone()),
                })
                .collect();
            let bundled_archives: Vec<linker::LinkInput> = link_plan
                .iter()
                .filter_map(|n| n.archive.clone().map(linker::LinkInput::ArchivePath))
                .collect();
            let system_lib_names: Vec<String> = link_plan
                .iter()
                .filter(|n| n.archive.is_none())
                .map(|n| n.lib.clone())
                .collect();
            let mut system_lib_dirs: Vec<std::path::PathBuf> = link_plan
                .iter()
                .filter(|n| n.archive.is_none())
                .flat_map(|n| n.lib_dirs.iter().cloned())
                .collect();
            system_lib_dirs.sort();
            system_lib_dirs.dedup();
            let user_objects: Vec<linker::LinkInput> = built
                .objects
                .into_iter()
                .map(linker::LinkInput::ObjectBytes)
                .collect();
            let native_requested = match std::env::var_os("RNX_NATIVE_LINK") {
                Some(v) if v == "0" => Some(false),
                Some(v) if v == "1" => Some(true),
                Some(_) => Some(false),
                None => None,
            };
            let native_x86 = selected_target.arch == TargetArch::X86_64
                && selected_target.os == TargetOs::Linux;
            let native_arm = selected_target.arch == TargetArch::Aarch64
                && selected_target.os == TargetOs::Linux;
            let native = match native_requested {
                Some(forced) => {
                    if forced && !native_x86 && !native_arm && !object_coff && !object_macho && !object_wasm {
                        eprintln!("error: RNX_NATIVE_LINK=1 needs a target with a native backend");
                        std::process::exit(1);
                    }
                    forced || native_arm || object_coff || object_macho || object_wasm
                }
                None => native_x86 || native_arm || object_coff || object_macho || object_wasm,
            };
            if native && !native_x86 && !native_arm && !object_coff && !object_macho && !object_wasm {
                eprintln!("error: native link needs a target with a native backend");
                std::process::exit(1);
            }
            let musl = selected_target.libc == TargetLibc::Musl;
            // Incremental-link cache: project-local memo of the archive
            // pull order. Cold builds (no plan file yet) take the exact
            // uncached path; the plan only ever accelerates rebuilds.
            // Construction here is in-memory only (one stat for the
            // rewrite gate, no hashing, no reads).
            let link_cache = || linker::plan::CacheCtx {
                dir: frontend::cache::project_cache_dir(&root)
                    .join("link")
                    .join(profile_name(release)),
                toolchain: format!(
                    "rnx={} llvm={} rt={} release={} opt={} target={} debug={} sysroot={} pack={} abi={}",
                    env!("CARGO_PKG_VERSION"),
                    llvm::LLVM_VERSION,
                    cli::runtime_archive_hash(),
                    release,
                    opt_level,
                    effective,
                    debug,
                    link_sysroot.identity(),
                    resolved_pack
                        .as_ref()
                        .and_then(|pack| pack.digest.as_deref())
                        .unwrap_or("none"),
                    resolved_runtime.abi,
                ),
                allow_write: std::path::Path::new(&out_path).is_file(),
            };
            let link_result = if object_macho {
                let mut user_objects = user_objects;
                user_objects.extend(foreign_libs);
                let inputs = match super::macho_link_inputs(
                    resolved_pack.as_ref(),
                    &resolved_runtime,
                    user_objects,
                ) {
                    Ok(inputs) => inputs,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
                linker::macho::link(&inputs, std::path::Path::new(&out_path), "start").map(|_| ())
            } else if object_wasm {
                let inputs = match super::wasm_link_inputs(
                    resolved_pack.as_ref(),
                    &resolved_runtime,
                    user_objects,
                ) {
                    Ok(inputs) => inputs,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
                linker::wasm::link(&inputs, std::path::Path::new(&out_path))
            } else if object_coff {
                let inputs = match super::pe_link_inputs(
                    resolved_pack.as_ref(),
                    &resolved_runtime,
                    user_objects,
                ) {
                    Ok(inputs) => inputs,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
                linker::pe::link(
                    &inputs,
                    std::path::Path::new(&out_path),
                    selected_target.pe_entry(),
                    selected_target.pe_subsystem(),
                )
                .map(|_| ())
            } else if musl {
                let mut inputs = user_objects;
                inputs.push(resolved_runtime.archive_input());
                inputs.extend(foreign_libs);
                linker::native::native_link_fully_static_in(
                    &inputs,
                    std::path::Path::new(&out_path),
                    &linker::native::StaticOpts {
                        entry: "_start".to_string(),
                        icf: release,
                        target: target.0.clone(),
                        strip: release,
                        lib_dirs: system_lib_dirs.clone(),
                        runpath: Some(system_lib_dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(":")),
                    },
                    &link_sysroot,
                )
                .map(|_| ())
            } else if native {
                if release {
                    let mut inputs = user_objects;
                    inputs.push(resolved_runtime.archive_input());
                    inputs.extend(foreign_libs);
                    let cache = link_cache();
                    linker::native::native_link_static_cached_in(
                        &inputs,
                        std::path::Path::new(&out_path),
                        &linker::native::StaticOpts {
                            entry: "_start".to_string(),
                            icf: true,
                            target: target.0.clone(),
                            strip: true,
                            lib_dirs: system_lib_dirs.clone(),
                            runpath: Some(system_lib_dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(":")),
                        },
                        &cache,
                        &link_sysroot,
                    )
                    .map(|_| ())
                } else {
                    match linker::find_shared_runtime_dir().filter(|_| !pack_selected) {
                        Some(lib_dir) => {
                            let mut dyn_libs = vec!["runtime_native".to_string()];
                            dyn_libs.extend(system_lib_names.iter().cloned());
                            let mut lib_dirs = vec![lib_dir.clone()];
                            lib_dirs.extend(system_lib_dirs.iter().cloned());
                            let mut runpath = vec![lib_dir.display().to_string()];
                            runpath.extend(system_lib_dirs.iter().map(|d| d.display().to_string()));
                            let mut dyn_objects = user_objects;
                            dyn_objects.extend(bundled_archives);
                            linker::native::native_link_dynamic_in(
                                &dyn_objects,
                                std::path::Path::new(&out_path),
                                &linker::native::DynamicOpts {
                                    entry: "_start".to_string(),
                                    icf: false,
                                    target: target.0.clone(),
                                    lib_dirs,
                                    libs: dyn_libs,
                                    runpath: Some(runpath.join(":")),
                                    strip: false,
                                },
                                &link_sysroot,
                            )
                            .map(|_| ())
                        }
                        None => {
                            let mut inputs = user_objects;
                            inputs.push(resolved_runtime.archive_input());
                            inputs.extend(foreign_libs);
                            let cache = link_cache();
                            linker::native::native_link_static_cached_in(
                                &inputs,
                                std::path::Path::new(&out_path),
                                &linker::native::StaticOpts {
                                    entry: "_start".to_string(),
                                    icf: false,
                                    target: target.0.clone(),
                                    strip: false,
                                    lib_dirs: system_lib_dirs.clone(),
                                    runpath: Some(system_lib_dirs.iter().map(|d| d.display().to_string()).collect::<Vec<_>>().join(":")),
                                },
                                &cache,
                                &link_sysroot,
                            )
                            .map(|_| ())
                        }
                    }
                }
            } else if release {
                let mut inputs = user_objects;
                inputs.push(resolved_runtime.external_input());
                inputs.extend(foreign_libs);
                linker::link_executable(&inputs, std::path::Path::new(&out_path), &target, true)
            } else {
                match linker::find_shared_runtime_dir().filter(|_| !pack_selected) {
                    Some(lib_dir) => {
                        let mut dyn_libs = vec!["runtime_native".to_string()];
                        dyn_libs.extend(system_lib_names.iter().cloned());
                        let mut dyn_objects = user_objects;
                        dyn_objects.extend(bundled_archives);
                        linker::link_executable_dynamic(
                            &dyn_objects,
                            std::path::Path::new(&out_path),
                            &target,
                            &linker::DynLink {
                                lib_dir,
                                lib_dirs: system_lib_dirs.clone(),
                                libs: dyn_libs,
                            },
                        )
                    }
                    None => {
                        let mut inputs = user_objects;
                        inputs.push(resolved_runtime.external_input());
                        inputs.extend(foreign_libs);
                        linker::link_executable(&inputs, std::path::Path::new(&out_path), &target, false)
                    }
                }
            };
            if let Err(e) = link_result {
                match &e {
                    linker::LinkError::Pack(_) => eprintln!(
                        "{}",
                        diagnostics::Diagnostic::new(diagnostics::Code::E124, e.to_string())
                    ),
                    linker::LinkError::Unsupported(_) => eprintln!(
                        "{}",
                        diagnostics::Diagnostic::new(diagnostics::Code::E108, e.to_string())
                    ),
                    _ => eprintln!("error: {e}"),
                }
                std::process::exit(1);
            }
            if let Some((stamp, key)) = pending_stamp {
                if let Err(e) = std::fs::write(&stamp, format!("{key}\n")) {
                    eprintln!("warning: cannot write {}: {e}", stamp.display());
                }
            }
            let link_ms = link_start.elapsed().as_secs_f64() * 1000.0;
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                let perms = std::fs::Permissions::from_mode(0o755);
                if let Err(e) = std::fs::set_permissions(&out_path, perms) {
                    eprintln!("error: chmod {out_path}: {e}");
                    std::process::exit(1);
                }
            }
            let total_ms = wall.elapsed().as_secs_f64() * 1000.0;
            let size = std::fs::metadata(&out_path).map(|m| m.len()).unwrap_or(0);
            let mut steps: Vec<cli::telemetry::BusStep> = built
                .stages
                .iter()
                .map(|(name, ms)| cli::telemetry::BusStep {
                    detail: bus_detail(name).to_string(),
                    name: bus_name(name).to_string(),
                    duration_ms: *ms,
                })
                .collect();
            steps.push(cli::telemetry::BusStep {
                name: "link".to_string(),
                detail: if release {
                    "stripped executable link".to_string()
                } else {
                    "executable link".to_string()
                },
                duration_ms: link_ms,
            });
            let mode = if release { "release" } else { "dev" };
            let title = format!(
                "build: {} [{mode}] ({})",
                cli::package_name_for(&input),
                effective
            );
            if quiet {
                println!("artifact: {out_path}");
            } else {
                print!(
                    "{}",
                    cli::telemetry::render_bus(
                        &theme,
                        &title,
                        &steps,
                        &out_path,
                        size,
                        total_ms,
                        cli::telemetry::peak_mb(),
                    )
                );
                println!("artifact: {out_path}");
            }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn project_name_gate() {
        assert!(project_name_invalid(""));
        assert!(project_name_invalid(".."));
        assert!(project_name_invalid("a..b"));
        assert!(project_name_invalid("a/b"));
        assert!(project_name_invalid("/lead"));
        assert!(project_name_invalid("a\\b"));
        assert!(project_name_invalid("a\x07b"));
        assert!(project_name_invalid("a\nb"));
        assert!(project_name_invalid("@lonely"));
        assert!(project_name_invalid("@/noname"));
        assert!(!project_name_invalid("app"));
        assert!(!project_name_invalid("sample_project"));
        assert!(!project_name_invalid("my-app"));
        assert!(!project_name_invalid("@acme/widget"));
    }

    #[test]
    fn built_in_targets_validate() {
        for triple in sysroot::target::accepted_triples() {
            assert!(cli::validate_target(triple).is_ok(), "{triple}");
        }
        assert!(cli::validate_target("x86_64-pc-windows-msvc").is_err());
        assert!(cli::validate_target("riscv64gc-unknown-linux-gnu").is_err());
    }

    fn write_manifest(dir: &std::path::Path, name: &str, deps: &str) {
        let text = format!(
            "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}{deps}}}\n"
        );
        std::fs::write(dir.join("Project.config"), text).unwrap();
    }

    #[test]
    fn build_key_tracks_path_dep_sources() {
        let root = crate::scratch::dir("build");
        let dep = root.join("libs").join("mydep");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(dep.join("src")).unwrap();
        write_manifest(
            &root,
            "app",
            ",\n    dependencies: {\n        mydep: { path: \"libs/mydep\" }\n    }\n",
        );
        write_manifest(&dep, "mydep", "");
        let main = root.join("src").join("main.rnx");
        std::fs::write(&main, "fn Main(): Int {\n    return 0;\n}\n").unwrap();
        let dep_src = dep.join("src").join("lib.rnx");
        std::fs::write(&dep_src, "pub fn helper(): Int {\n    return 1;\n}\n").unwrap();
        let input = main.to_string_lossy().into_owned();
        let key_before = build_key(&input, &root, "Main", false, 1, None, false, 1, 0, frontend::native::NativePrefer::None, &[], None, runtime::archive::SHA256, runtime::archive::ABI);
        assert_eq!(key_before, build_key(&input, &root, "Main", false, 1, None, false, 1, 0, frontend::native::NativePrefer::None, &[], None, runtime::archive::SHA256, runtime::archive::ABI));
        std::fs::write(&dep_src, "pub fn helper(): Int {\n    return 2;\n}\n").unwrap();
        let key_after = build_key(&input, &root, "Main", false, 1, None, false, 1, 0, frontend::native::NativePrefer::None, &[], None, runtime::archive::SHA256, runtime::archive::ABI);
        assert_ne!(key_before, key_after);
    }
}
