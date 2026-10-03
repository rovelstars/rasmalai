use super::*;
use cli::RunOutcome;
use runtime::value::Value;

pub(super) fn run_run(path: Option<std::path::PathBuf>, entry: String, package: Option<String>, locked: bool, opt_level: String, backend: String, time_passes: bool, trace: Option<std::path::PathBuf>, perf_map: bool, debug: bool, rest: Vec<String>) {
            let path = path.map(|p| p.to_string_lossy().into_owned());
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
            let args: Vec<Value> = rest
                .iter()
                .map(|a| match a.parse::<i64>() {
                    Ok(n) => Value::Int(n),
                    Err(_) => Value::Str(a.clone()),
                })
                .collect();
            let target = cli::resolve_scope_target(path.as_deref(), package.as_deref())
                .unwrap_or_else(|e| {
                    eprintln!("{e}");
                    std::process::exit(1);
                });
            if locked {
                let locked_ok = match &target.scope_root {
                    Some(root) => cli::enforce_locked_at(root),
                    None => Err(cli::missing_lock()),
                };
                if let Err(e) = locked_ok {
                    println!("{e}");
                    std::process::exit(1);
                }
                let ceiling_issues =
                    frontend::security::verify_entry(std::path::Path::new(&target.entry));
                if !ceiling_issues.is_empty() {
                    eprint!("{}", cli::report::render_security_issues(&ceiling_issues));
                    std::process::exit(1);
                }
                cli::apply_locked_spawn_gate(target.scope_root.as_deref());
            } else {
                runtime::native::rnx_clear_spawn_restriction();
            }
            if debug {
                eprintln!("warning: -g applies to native `rnx build` outputs; ignored for `run`");
            }
            let cfg = cli::CompileConfig {
                time_passes,
                trace,
                perf_map,
                debug: false,
            };
            let out = cli::run_files_cfg(&target.entry, &entry, args, opt_level, backend, &cfg);
            for line in &out.output {
                if line.ends_with('\n') {
                    print!("{line}");
                } else {
                    println!("{line}");
                }
            }
            let theme = diagnostics::theme::AuraTheme::active();
            match out.outcome {
                RunOutcome::Value(v) => {
                    if let cli::Value::Int(n) = v {
                        std::process::exit(n as i32);
                    }
                }
                RunOutcome::Thrown(v) => {
                    eprint!(
                        "{}",
                        cli::report::render_uncaught(&format!("{}", v.display()), &[])
                    );
                    std::process::exit(1);
                }
                RunOutcome::Uncaught { message, span, func } => {
                    let trace = cli::report::uncaught_trace(
                        std::path::Path::new(&target.entry),
                        span,
                        func.as_deref(),
                    );
                    eprint!("{}", cli::report::render_uncaught(&message, &trace));
                    std::process::exit(1);
                }
                RunOutcome::Fatal { message: m, span, func } => {
                    eprint!(
                        "{}",
                        cli::report::render_runtime_error(
                            &theme,
                            &format!("fatal: {m}"),
                            std::path::Path::new(&target.entry),
                            span,
                            func.as_deref()
                        )
                    );
                    std::process::exit(1);
                }
                RunOutcome::Compile(errs) => {
                    print!(
                        "{}",
                        cli::report::render_compile_errors(
                            &theme,
                            std::path::Path::new(&target.entry),
                            &errs
                        )
                    );
                    std::process::exit(1);
                }
            }
}
