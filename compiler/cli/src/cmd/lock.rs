
pub(super) fn run_lock(package: Option<String>) {
            match cli::lock_scope(None, package.as_deref()) {
                Ok(count) => println!("Locked {count} packages to Project.deplock"),
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
