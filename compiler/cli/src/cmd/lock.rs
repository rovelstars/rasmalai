pub(super) fn run_lock(
    package: Option<String>,
    target: Option<String>,
    prefer: frontend::native::NativePrefer,
    quiet: bool,
) {
    let result = super::with_status(quiet, "Resolving dependencies", || {
        cli::lock_scope_with_target(None, package.as_deref(), prefer, target.as_deref())
    });
    match result {
        Ok(count) => println!("Locked {count} packages to Project.deplock"),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
