
pub(super) fn run_check(paths: Vec<std::path::PathBuf>, package: Option<String>, json: bool, expand_macros: bool, verbose: bool, quiet: bool) {
            if verbose {
                eprintln!("rnx: checking {} path(s)", paths.len());
            }
            let label = match paths.as_slice() {
                [] => "Checking the current project".to_string(),
                [one] => format!("Checking {}", one.display()),
                _ => format!(
                    "Checking {} path{}",
                    paths.len(),
                    if paths.len() == 1 { "" } else { "s" }
                ),
            };
            let outcome = super::with_status(quiet || json, &label, || {
                cli::check::run_check(&paths, package.as_deref(), json, expand_macros, quiet)
            });
            print!("{}", outcome.stdout);
            eprint!("{}", outcome.stderr);
            std::process::exit(outcome.code);
}
