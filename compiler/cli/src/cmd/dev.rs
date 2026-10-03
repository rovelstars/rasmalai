
pub(super) fn run_dev(path: Option<std::path::PathBuf>, entry: String, no_rerun: bool, mcp: bool) {
            let path = path.map(|p| p.to_string_lossy().into_owned());
            let target = cli::resolve_scope_target(path.as_deref(), None).unwrap_or_else(|e| {
                eprintln!("{e}");
                std::process::exit(1);
            });
            let entry_file = std::path::PathBuf::from(&target.entry);
            let mut runner = match cli::dev::DevRunner::new(&entry_file, &entry, no_rerun) {
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
                        eprintln!("cannot start async runtime: {e}");
                        std::process::exit(1);
                    });
                if let Err(e) = rt.block_on(cli::mcp::serve_with_dev(link)) {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
                return;
            }
            match runner.run_entry() {
                Ok(v) => println!("[rnx dev] {entry}() -> {v}"),
                Err(e) => {
                    eprintln!("[rnx dev] entry failed: {e:?}");
                    std::process::exit(1);
                }
            }
            println!("[rnx dev] watching {}", entry_file.display());
            loop {
                match runner.poll(std::time::Duration::from_millis(500)) {
                    None => {}
                    Some(cli::dev::DevEvent::Swapped { names, elapsed_ms }) => {
                        println!("[rnx dev] hot swapped {} in {elapsed_ms}ms", names.join(", "));
                        if runner.should_rerun() {
                            match runner.run_entry() {
                                Ok(v) => println!("[rnx dev] {entry}() -> {v}"),
                                Err(e) => eprintln!("[rnx dev] entry failed: {e:?}"),
                            }
                        }
                    }
                    Some(cli::dev::DevEvent::Restarted { reason }) => {
                        println!("[rnx dev] structural change detected in {reason} -> restarting");
                        match runner.run_entry() {
                            Ok(v) => println!("[rnx dev] {entry}() -> {v}"),
                            Err(e) => eprintln!("[rnx dev] entry failed: {e:?}"),
                        }
                    }
                    Some(cli::dev::DevEvent::Broken { count }) => {
                        eprintln!("[rnx dev] {count} error(s); keeping last valid build");
                    }
                }
            }
}
