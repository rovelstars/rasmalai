pub(super) fn run_update(
    packages: Vec<String>,
    major: bool,
    package: Option<String>,
    prefer: frontend::native::NativePrefer,
) {
    match cli::update_scope_with(None, package.as_deref(), packages, major, prefer) {
        Ok(summary) => {
            for change in &summary.updated {
                println!("updated {} {} -> {}", change.name, change.old, change.new);
            }
            for held in &summary.held {
                println!(
                    "held {} at {} (major move to {} needs --major)",
                    held.name, held.old, held.new
                );
            }
            if summary.updated.is_empty() && summary.held.is_empty() {
                println!("Already up to date.");
            }
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
