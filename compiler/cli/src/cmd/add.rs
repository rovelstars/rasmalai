
pub(super) fn run_add(package: String, path: Option<std::path::PathBuf>, accept_caps: Option<String>, accept_all_caps: bool) {
            match cli::add_package(
            &package,
            path.as_deref(),
            accept_caps.as_deref(),
            accept_all_caps,
        ) {
            cli::AddResult::Added(msg) => println!("{msg}"),
            cli::AddResult::Failed(e) => {
                println!("{e}");
                std::process::exit(1);
            }
            cli::AddResult::Headless(msg) => {
                eprintln!("{msg}");
                std::process::exit(2);
            }
        }
}
