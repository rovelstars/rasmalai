
pub(super) fn run_audit(
    path: Option<std::path::PathBuf>,
    json: bool,
    export_manifest: bool,
    registry: Option<String>,
) {
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
        return;
    }
    if json {
        println!("{}", cli::audit::report_json(&report));
    } else {
        let drift = cli::audit::lockfile_drift(&dir);
        print!("{}", cli::audit::render_audit_text(&report, &drift));
    }
    let base = cli::audit::advisory_registry(registry.as_deref());
    match cli::audit::audit_advisories(&dir, &base) {
        cli::audit::AdvisoryCheck::Skipped(note) => {
            eprintln!("note: {note}");
        }
        cli::audit::AdvisoryCheck::Rejected(reason) => {
            eprintln!("error: {reason}");
            std::process::exit(1);
        }
        cli::audit::AdvisoryCheck::Done(findings) => {
            let text = cli::audit::render_advisory_text(&findings);
            if json {
                eprint!("{text}");
            } else {
                print!("{text}");
            }
            if frontend::advisory::has_hold(&findings) {
                std::process::exit(1);
            }
        }
    }
}
