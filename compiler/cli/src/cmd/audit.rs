
pub(super) fn run_audit(
    path: Option<std::path::PathBuf>,
    json: bool,
    export_manifest: bool,
    licenses: bool,
    deny_license: Vec<String>,
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
    match frontend::security::scan_dep_prefix(&dir) {
        Ok(hits) if hits.is_empty() => {}
        Ok(hits) => {
            eprint!("{}", cli::report::render_security_issues(&hits));
            eprintln!(
                "hint: call the owning `@std/*` wrapper instead of the `__rnx_*` internal (rule S103)"
            );
            std::process::exit(1);
        }
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    }
    let denied = cli::audit::parse_deny_licenses(&deny_license);
    for license in &denied {
        if !frontend::license::is_known_spdx(license) {
            eprintln!("error[E108]: unknown license `{license}` in --deny-license (expected a known SPDX id)");
            std::process::exit(1);
        }
    }
    let mut license_failed = false;
    if licenses || !denied.is_empty() {
        match cli::audit::license_report_for_dir(&dir) {
            Ok(license_report) => {
                let text = cli::audit::render_license_text(&license_report, &denied);
                if json {
                    eprint!("{text}");
                } else {
                    print!("{text}");
                }
                if !cli::audit::denied_hits(&license_report, &denied).is_empty() {
                    license_failed = true;
                }
                if !license_report.conflicts.is_empty() {
                    license_failed = true;
                }
            }
            Err(e) => {
                if licenses || !denied.is_empty() {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
        }
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
    if license_failed {
        std::process::exit(1);
    }
}
