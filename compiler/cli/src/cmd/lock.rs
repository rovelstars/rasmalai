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
        Ok(count) => {
            let theme = diagnostics::theme::AuraTheme::active();
            println!(
                "{} {} packages to Project.deplock",
                theme.paint(diagnostics::theme::AuraColor::Purple, "Locked"),
                theme.paint(
                    diagnostics::theme::AuraColor::Orange,
                    &count.to_string()
                )
            );
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
