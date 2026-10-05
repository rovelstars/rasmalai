use super::*;

fn profile_name(release: bool) -> &'static str {
    if release { "release" } else { "dev" }
}

fn build_key(
    input: &str,
    root: &std::path::Path,
    release: bool,
    opt_level: u8,
    target_triple: Option<&str>,
) -> String {
    let entry_bytes = std::fs::read(input).unwrap_or_default();
    let manifest_bytes = std::fs::read(root.join(frontend::project::MANIFEST_FILE)).unwrap_or_default();
    let deplock_bytes = std::fs::read(root.join("Project.deplock")).unwrap_or_default();
    let flags = format!("release={release} opt={opt_level} target={}", target_triple.unwrap_or("host"));
    frontend::cache::fingerprint_hex(&[
        entry_bytes.as_slice(),
        manifest_bytes.as_slice(),
        deplock_bytes.as_slice(),
        flags.as_bytes(),
    ])
}

fn default_out_path(input: &str, root: &std::path::Path, release: bool) -> std::path::PathBuf {
    let project_name = frontend::project::load_manifest(root)
        .ok()
        .flatten()
        .and_then(|m| m.project.map(|p| p.name));
    let stem = project_name.unwrap_or_else(|| {
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
pub(super) fn run_build(path: Option<std::path::PathBuf>, entry: String, release: bool, lib: bool, emit_obj: bool, target_triple: Option<String>, locked: bool, opt_level: String, time_passes: bool, trace: Option<std::path::PathBuf>, perf_map: bool, debug: bool, package: Option<String>, verbose: bool, quiet: bool) {
            let path = path.map(|p| p.to_string_lossy().into_owned());
            let opt_level = parse_opt_level(&opt_level);
            let target = cli::resolve_scope_target(path.as_deref(), package.as_deref())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            let input = target.entry;
            let root = project_root_for(&input);
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
            };
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
                    let key = build_key(&input, &root, release, opt_level, target_triple.as_deref());
                    let stamp = path.with_file_name(".fingerprint");
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
            let link_result = if release {
                let mut inputs = vec![
                    linker::LinkInput::ObjectBytes(built.bytes),
                    linker::LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()),
                ];
                inputs.extend(foreign_libs);
                linker::link_executable(&inputs, std::path::Path::new(&out_path), &target, true)
            } else {
                match linker::find_shared_runtime_dir() {
                    Some(lib_dir) => {
                        let mut dyn_libs = vec!["runtime_native".to_string()];
                        dyn_libs.extend(built.native_libs.iter().cloned());
                        linker::link_executable_dynamic(
                            &[linker::LinkInput::ObjectBytes(built.bytes)],
                            std::path::Path::new(&out_path),
                            &target,
                            &linker::DynLink { lib_dir, libs: dyn_libs },
                        )
                    }
                    None => {
                        let mut inputs = vec![
                            linker::LinkInput::ObjectBytes(built.bytes),
                            linker::LinkInput::ArchiveBytes(runtime::archive::BYTES.to_vec()),
                        ];
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
