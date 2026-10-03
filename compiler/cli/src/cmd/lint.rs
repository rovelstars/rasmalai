
pub(super) fn run_lint(paths: Vec<std::path::PathBuf>, package: Option<String>, sarif: bool, json: bool, deny_warnings: bool, fix: bool) {
            let outcome =
                cli::lint::run_lint(&paths, package.as_deref(), sarif, json, deny_warnings, fix);
            print!("{}", outcome.stdout);
            eprint!("{}", outcome.stderr);
            std::process::exit(outcome.code);
}
