
pub(super) fn run_lock(package: Option<String>, prefer: frontend::native::NativePrefer) {
            match cli::lock_scope_with(None, package.as_deref(), prefer) {
                Ok(count) => println!("Locked {count} packages to Project.deplock"),
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
