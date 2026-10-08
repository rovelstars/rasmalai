use super::*;

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
        for (name, spec) in &cfg.dependencies {
            if let frontend::project::DependencySpec::Path { path } = spec {
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

fn default_out_path(input: &str, root: &std::path::Path, release: bool) -> std::path::PathBuf {
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
        .join(stem)
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
pub(super) fn run_build(path: Option<std::path::PathBuf>, entry: String, release: bool, lib: bool, emit_obj: bool, target_triple: Option<String>, locked: bool, opt_level: String, time_passes: bool, trace: Option<std::path::PathBuf>, perf_map: bool, debug: bool, jobs: Option<usize>, memory_cap: Option<u64>, package: Option<String>, verbose: bool, quiet: bool) {
            let path = path.map(|p| p.to_string_lossy().into_owned());
            let opt_level = parse_opt_level(&opt_level);
            let target = cli::resolve_scope_target(path.as_deref(), package.as_deref())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            let input = target.entry;
            let root = project_root_for(&input);
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
            if let Some(ref t) = target_triple {
                if let Err(e) = cli::validate_target(t) {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
            let host = linker::host_triple().0;
            let cross = target_triple.as_deref().is_some_and(|t| t != host);
            if cross && !emit_obj {
                println!(
                    "{}",
                    diagnostics::Diagnostic::new(
                        diagnostics::Code::E108,
                        "cross targets need --emit-obj; linking runs on the host",
                    )
                );
                std::process::exit(1);
            }
            if locked {
                let locked_ok = match &target.scope_root {
                    Some(root) => cli::enforce_locked_at(root),
                    None => Err(cli::missing_lock()),
                };
                if let Err(e) = locked_ok {
                    println!("{e}");
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
            };
            if verbose {
                eprintln!("rnx: jobs={} memory-cap={} bytes", cfg.jobs, cfg.memory_cap);
            }
            if lib {
                let theme = diagnostics::theme::AuraTheme::active();
                let built = match cli::build_lib_files_cfg(&input, release, opt_level, target_triple.as_deref(), &cfg) {
                    Ok(b) => b,
                    Err(errs) => {
                        print!(
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
                    target_triple.as_deref(),
                    &cfg,
                ) {
                    Ok(b) => b,
                    Err(errs) => {
                        print!(
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
            let built = match cli::build_files_staged_cfg(&input, &entry, release, opt_level, target_triple.as_deref(), &cfg) {
                Ok(b) => b,
                Err(errs) => {
                    print!(
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
            let (out_path, pending_stamp) = {
                    let path = default_out_path(&input, &root, release);
                    if let Some(parent) = path.parent() {
                        if let Err(e) = std::fs::create_dir_all(parent) {
                            eprintln!("error: cannot create {}: {e}", parent.display());
                            std::process::exit(1);
                        }
                    }
                    let key = build_key(&input, &root, &entry, release, opt_level, target_triple.as_deref(), debug, cfg.jobs, cfg.memory_cap);
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
            let target = linker::host_triple();
            let link_start = std::time::Instant::now();
            let foreign_libs: Vec<linker::LinkInput> = built
                .native_libs
                .iter()
                .map(|l| linker::LinkInput::Lib(l.clone()))
                .collect();
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
            let native = match native_requested {
                Some(forced) => {
                    if forced && target.0 != "x86_64-unknown-linux-gnu" {
                        eprintln!("error: RNX_NATIVE_LINK=1 needs target x86_64-unknown-linux-gnu");
                        std::process::exit(1);
                    }
                    forced
                }
                None => target.0 == "x86_64-unknown-linux-gnu",
            };
            if native && target.0 != "x86_64-unknown-linux-gnu" {
                eprintln!("error: native link needs target x86_64-unknown-linux-gnu");
                std::process::exit(1);
            }
            let link_result = if native {
                if release {
                    let mut inputs = user_objects;
                    inputs.push(linker::LinkInput::ArchiveRef(runtime::archive::BYTES));
                    inputs.extend(foreign_libs);
                    linker::native::native_link_static(
                        &inputs,
                        std::path::Path::new(&out_path),
                        &linker::native::StaticOpts {
                            entry: "_start".to_string(),
                            icf: true,
                            target: target.0.clone(),
                            strip: true,
                        },
                    )
                    .map(|_| ())
                } else {
                    match linker::find_shared_runtime_dir() {
                        Some(lib_dir) => {
                            let mut dyn_libs = vec!["runtime_native".to_string()];
                            dyn_libs.extend(built.native_libs.iter().cloned());
                            linker::native::native_link_dynamic(
                                &user_objects,
                                std::path::Path::new(&out_path),
                                &linker::native::DynamicOpts {
                                    entry: "_start".to_string(),
                                    icf: false,
                                    target: target.0.clone(),
                                    lib_dirs: vec![lib_dir.clone()],
                                    libs: dyn_libs,
                                    runpath: Some(lib_dir.display().to_string()),
                                    strip: false,
                                },
                            )
                            .map(|_| ())
                        }
                        None => {
                            let mut inputs = user_objects;
                            inputs.push(linker::LinkInput::ArchiveRef(runtime::archive::BYTES));
                            inputs.extend(foreign_libs);
                            linker::native::native_link_static(
                                &inputs,
                                std::path::Path::new(&out_path),
                                &linker::native::StaticOpts {
                                    entry: "_start".to_string(),
                                    icf: false,
                                    target: target.0.clone(),
                                    strip: false,
                                },
                            )
                            .map(|_| ())
                        }
                    }
                }
            } else if release {
                let mut inputs = user_objects;
                inputs.push(linker::LinkInput::ArchiveRef(runtime::archive::BYTES));
                inputs.extend(foreign_libs);
                linker::link_executable(&inputs, std::path::Path::new(&out_path), &target, true)
            } else {
                match linker::find_shared_runtime_dir() {
                    Some(lib_dir) => {
                        let mut dyn_libs = vec!["runtime_native".to_string()];
                        dyn_libs.extend(built.native_libs.iter().cloned());
                        linker::link_executable_dynamic(
                            &user_objects,
                            std::path::Path::new(&out_path),
                            &target,
                            &linker::DynLink { lib_dir, libs: dyn_libs },
                        )
                    }
                    None => {
                        let mut inputs = user_objects;
                        inputs.push(linker::LinkInput::ArchiveRef(runtime::archive::BYTES));
                        inputs.extend(foreign_libs);
                        linker::link_executable(&inputs, std::path::Path::new(&out_path), &target, false)
                    }
                }
            };
            if let Err(e) = link_result {
                eprintln!("error: {e}");
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
                target_triple.as_deref().unwrap_or(target.0.as_str())
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

    fn write_manifest(dir: &std::path::Path, name: &str, deps: &str) {
        let text = format!(
            "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}{deps}}}\n"
        );
        std::fs::write(dir.join("Project.config"), text).unwrap();
    }

    #[test]
    fn build_key_tracks_path_dep_sources() {
        let root = std::env::temp_dir().join(format!("rnx-build-key-pathdep-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
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
        let key_before = build_key(&input, &root, "Main", false, 1, None, false, 1, 0);
        assert_eq!(key_before, build_key(&input, &root, "Main", false, 1, None, false, 1, 0));
        std::fs::write(&dep_src, "pub fn helper(): Int {\n    return 2;\n}\n").unwrap();
        let key_after = build_key(&input, &root, "Main", false, 1, None, false, 1, 0);
        assert_ne!(key_before, key_after);
        let _ = std::fs::remove_dir_all(&root);
    }
}
