
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
            println!("Watching {} for changes. Press Ctrl-C to stop.", entry_file.display());
            match runner.run_entry() {
                Ok(v) => println!("{entry}() returned {v}."),
                Err(e) => {
                    eprintln!("Entry failed: {e:?}");
                    std::process::exit(1);
                }
            }
            loop {
                match runner.poll(std::time::Duration::from_millis(500)) {
                    None => {}
                    Some(cli::dev::DevEvent::Swapped { names, elapsed_ms }) => {
                        println!("Hot swapped {} in {elapsed_ms} ms.", names.join(", "));
                        if runner.should_rerun() {
                            match runner.run_entry() {
                                Ok(v) => println!("{entry}() returned {v}."),
                                Err(e) => eprintln!("Entry failed: {e:?}"),
                            }
                        }
                    }
                    Some(cli::dev::DevEvent::Restarted { reason }) => {
                        println!("Rebuilt after a structural change in {reason}.");
                        match runner.run_entry() {
                            Ok(v) => println!("{entry}() returned {v}."),
                            Err(e) => eprintln!("Entry failed: {e:?}"),
                        }
                    }
                    Some(cli::dev::DevEvent::Broken { count }) => {
                        eprintln!("Build failed with {count} error(s); keeping the last good build.");
                    }
                }
            }
}
