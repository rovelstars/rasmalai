
pub(super) fn run_audit(path: Option<std::path::PathBuf>, json: bool, export_manifest: bool) {
            let dir = match path {
                Some(p) => p,
                None => match std::env::current_dir() {
                    Ok(d) => d,
                    Err(e) => {
                        eprintln!("error: cannot read working directory: {e}");
                        std::process::exit(1);
                    }
                },
            };
            let report = match cli::audit::audit_path(&dir) {
                Ok(r) => r,
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            };
            if export_manifest {
                println!("{}", cli::audit::export_manifest(&report));
            } else if json {
                println!("{}", cli::audit::report_json(&report));
            } else {
                let drift = cli::audit::lockfile_drift(&dir);
                print!("{}", cli::audit::render_audit_text(&report, &drift));
            }
}
