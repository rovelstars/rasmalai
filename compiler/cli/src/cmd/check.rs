
pub(super) fn run_check(paths: Vec<std::path::PathBuf>, package: Option<String>, json: bool, expand_macros: bool, verbose: bool, quiet: bool) {
            if verbose {
                eprintln!("rnx: checking {} path(s)", paths.len());
            }
            let outcome = cli::check::run_check(&paths, package.as_deref(), json, expand_macros, quiet);
            print!("{}", outcome.stdout);
            eprint!("{}", outcome.stderr);
            std::process::exit(outcome.code);
}
