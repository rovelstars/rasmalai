
pub(super) fn run_fmt(paths: Vec<std::path::PathBuf>, check: bool, diff: bool) {
            let mut paths = paths;
            if paths.is_empty() {
                let cwd = std::env::current_dir().unwrap_or_else(|e| {
                    eprintln!("error: cannot read working directory: {e}");
                    std::process::exit(1);
                });
                paths.push(cwd);
            }
            let outcome = cli::fmt::run_fmt(&paths, check, diff);
            print!("{}", outcome.stdout);
            eprint!("{}", outcome.stderr);
            std::process::exit(outcome.code);
}
