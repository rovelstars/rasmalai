
pub(super) fn run_dev(path: Option<std::path::PathBuf>, entry: String, no_rerun: bool, mcp: bool, screenshot: Option<std::path::PathBuf>, prefer: frontend::native::NativePrefer) {
            if std::env::var_os("RNX_VENDOR_DIR").is_none()
                && let Some(stdlib) = cli::checkout::source_stdlib()
            {
                unsafe {
                    std::env::set_var("RNX_VENDOR_DIR", stdlib);
                }
            }
            if let Some(shot) = screenshot {
                unsafe {
                    std::env::set_var("RNX_DEV_SHOT", shot);
                }
            }
            let path = path.map(|p| p.to_string_lossy().into_owned());
            let target = cli::resolve_scope_target(path.as_deref(), None).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let entry_file = std::path::PathBuf::from(&target.entry);
            let mut runner = match cli::dev::DevRunner::new(&entry_file, &entry, no_rerun, prefer) {
                Ok(r) => r,
                Err(errs) => {
                    let theme = diagnostics::theme::AuraTheme::active();
                    eprint!(
                        "{}",
                        cli::report::render_compile_errors(&theme, &entry_file, &errs)
                    );
                    std::process::exit(1);
                }
            };
            if mcp {
                let (tx, rx) = std::sync::mpsc::channel();
                let link = cli::mcp::DevLink::new(tx);
                std::thread::spawn(move || cli::dev::run_watch_loop(runner, Some(rx)));
                let rt = tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                    .unwrap_or_else(|e| {
                        eprintln!("error: cannot start the async runtime: {e}");
                        std::process::exit(1);
                    });
                if let Err(e) = rt.block_on(cli::mcp::serve_with_dev(link)) {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                return;
            }
            let theme = diagnostics::theme::AuraTheme::active();
            println!(
                "{} {} for changes. {}",
                theme.paint(diagnostics::theme::AuraColor::Purple, "Watching"),
                theme.paint(
                    diagnostics::theme::AuraColor::Cyan,
                    &entry_file.display().to_string()
                ),
                theme.paint(
                    diagnostics::theme::AuraColor::Muted,
                    "Press Ctrl-C to stop."
                )
            );
            match runner.run_entry() {
                Ok(v) => println!(
                    "{}() returned {}.",
                    theme.paint(diagnostics::theme::AuraColor::Cyan, &entry),
                    theme.paint(
                        diagnostics::theme::AuraColor::Orange,
                        &v.to_string()
                    )
                ),
                Err(e) => {
                    eprintln!("Entry failed: {e:?}");
                    std::process::exit(1);
                }
            }
            loop {
                match runner.poll(std::time::Duration::from_millis(500)) {
                    None => {}
                    Some(cli::dev::DevEvent::Swapped { names, elapsed_ms }) => {
                        println!(
                            "{} {} in {} ms.",
                            theme.paint(
                                diagnostics::theme::AuraColor::Purple,
                                "Hot swapped"
                            ),
                            theme.paint(
                                diagnostics::theme::AuraColor::Cyan,
                                &names.join(", ")
                            ),
                            theme.paint(
                                diagnostics::theme::AuraColor::Orange,
                                &elapsed_ms.to_string()
                            )
                        );
                        if runner.should_rerun() {
                            match runner.run_entry() {
                                Ok(v) => println!(
                    "{}() returned {}.",
                    theme.paint(diagnostics::theme::AuraColor::Cyan, &entry),
                    theme.paint(
                        diagnostics::theme::AuraColor::Orange,
                        &v.to_string()
                    )
                ),
                                Err(e) => eprintln!("Entry failed: {e:?}"),
                            }
                        }
                    }
                    Some(cli::dev::DevEvent::Restarted { reason }) => {
                        println!(
                            "{} after a structural change in {}.",
                            theme.paint(diagnostics::theme::AuraColor::Purple, "Rebuilt"),
                            theme.paint(diagnostics::theme::AuraColor::Cyan, &reason)
                        );
                        match runner.run_entry() {
                            Ok(v) => println!(
                    "{}() returned {}.",
                    theme.paint(diagnostics::theme::AuraColor::Cyan, &entry),
                    theme.paint(
                        diagnostics::theme::AuraColor::Orange,
                        &v.to_string()
                    )
                ),
                            Err(e) => eprintln!("Entry failed: {e:?}"),
                        }
                    }
                    Some(cli::dev::DevEvent::Broken { count }) => {
                        eprintln!(
                            "{} with {} error(s); keeping the last good build.",
                            theme.paint(diagnostics::theme::AuraColor::Red, "Build failed"),
                            theme.paint(
                                diagnostics::theme::AuraColor::Orange,
                                &count.to_string()
                            )
                        );
                    }
                }
            }
}
