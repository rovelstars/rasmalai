use super::*;

pub(super) fn run_test(filter: Option<String>, package: Option<String>, opt_level: String, backend: String, exact: bool, nocapture: bool, verbose: bool, quiet: bool, no_color: bool) {
            let opt_level = parse_opt_level(&opt_level);
            let backend = match backend.as_str() {
                "interpreter" => cli::TestBackend::Interpreter,
                "cranelift" => cli::TestBackend::Cranelift,
                "llvm" => cli::TestBackend::Llvm,
                other => {
                    eprintln!("unknown backend `{other}`");
                    std::process::exit(2);
                }
            };
            let targets = cli::test_targets(package.as_deref()).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            if verbose {
                eprintln!("rnx: testing {} target(s)", targets.len());
            }
            if !cli::test_runner::is_worker() {
                let mut code = 0;
                let theme = diagnostics::theme::AuraTheme::active();
                for (pkg_root, _) in &targets {
                    let req = cli::test_runner::TargetRequest {
                        pkg_root: pkg_root.clone(),
                        filter: filter.clone(),
                        opt_level: opt_level.to_string(),
                        backend: backend_name(&backend).to_string(),
                        exact,
                        no_color,
                        quiet,
                    };
                    if nocapture {
                        if cli::test_runner::run_nocapture(&req, pkg_root) != 0 {
                            code = 1;
                        }
                    } else if cli::test_runner::run_captured(&theme, &req, pkg_root) != 0 {
                        code = 1;
                    }
                }
                std::process::exit(code);
            }
            let mut code = 0;
            let theme = diagnostics::theme::AuraTheme::active();
            for (pkg_root, _) in &targets {
                let lowered = match cli::load_test_program_opt(
                    pkg_root,
                    filter.as_deref(),
                    exact,
                    opt_level,
                ) {
                    Ok(l) => l,
                    Err(errs) => {
                        print!(
                            "{}",
                            cli::report::render_compile_errors(&theme, pkg_root, &errs)
                        );
                        std::process::exit(1);
                    }
                };
                match cli::run_test_harness(lowered, backend) {
                    cli::TestOutcome::Completed { output, failed } => {
                        for line in &output {
                            if quiet && !line.starts_with("tests:") {
                                continue;
                            }
                            println!("{}", cli::telemetry::colorize_test_line(&theme, line));
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
