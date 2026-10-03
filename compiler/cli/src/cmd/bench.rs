
pub(super) fn run_bench(package: Option<String>, filter: Option<String>, backend: String, no_release: bool, time_passes: bool, trace: Option<std::path::PathBuf>, perf_map: bool, debug: bool) {
            let release = !no_release;
            let backend = match backend.as_str() {
                "interpreter" => cli::TestBackend::Interpreter,
                "cranelift" => cli::TestBackend::Cranelift,
                "llvm" => cli::TestBackend::Llvm,
                other => {
                    eprintln!("unknown backend `{other}`");
                    std::process::exit(2);
                }
            };
            if debug {
                eprintln!("warning: -g applies to native `rnx build` outputs; ignored for `bench`");
            }
            let cfg = cli::CompileConfig {
                time_passes,
                trace,
                perf_map,
                debug: false,
            };
            let opt_level = if release { 1 } else { 0 };
            let targets = cli::test_targets(package.as_deref()).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let mut code = 0;
            let theme = diagnostics::theme::AuraTheme::active();
            for (pkg_root, _) in &targets {
                let (lowered, mut profiler) = match cli::load_bench_program_cfg(pkg_root, filter.as_deref(), opt_level, &cfg) {
                    Ok(l) => l,
                    Err(errs) => {
                        print!(
                            "{}",
                            cli::report::render_compile_errors(&theme, pkg_root, &errs)
                        );
                        std::process::exit(1);
                    }
                };
                match cli::run_test_harness_cfg(lowered, backend, &cfg, &mut profiler) {
                    cli::TestOutcome::Completed { output, failed } => {
                        for line in &output {
                            if line.ends_with('\n') {
                                print!("{line}");
                            } else {
                                println!("{line}");
                            }
                        }
                        if failed != 0 {
                            code = 1;
                        }
                    }
                    cli::TestOutcome::Compile(errs) => {
                        print!(
                            "{}",
                            cli::report::render_compile_errors(&theme, pkg_root, &errs)
                        );
                        std::process::exit(1);
                    }
                    cli::TestOutcome::Runtime(m) => {
                        eprintln!("error: {m}");
                        std::process::exit(1);
                    }
                }
            }
            std::process::exit(code);
}
