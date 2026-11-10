pub(super) fn run_lock(
    package: Option<String>,
    target: Option<String>,
    prefer: frontend::native::NativePrefer,
) {
    match cli::lock_scope_with_target(None, package.as_deref(), prefer, target.as_deref()) {
        Ok(count) => println!("Locked {count} packages to Project.deplock"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
