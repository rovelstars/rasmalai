pub(super) fn run_update(
    packages: Vec<String>,
    major: bool,
    package: Option<String>,
    prefer: frontend::native::NativePrefer,
    quiet: bool,
) {
    let result = super::with_status(quiet, "Updating dependencies", || {
        cli::update_scope_with(None, package.as_deref(), packages, major, prefer)
    });
    match result {
        Ok(summary) => {
            let theme = diagnostics::theme::AuraTheme::active();
            for change in &summary.updated {
                println!(
                    "Updated {} from {} to {}.",
                    theme.paint(diagnostics::theme::AuraColor::Cyan, &change.name),
                    theme.paint(diagnostics::theme::AuraColor::Muted, &change.old),
                    theme.paint(diagnostics::theme::AuraColor::Green, &change.new)
                );
            }
            for held in &summary.held {
                println!(
                    "Held {} at {}; moving to {} needs --major.",
                    theme.paint(diagnostics::theme::AuraColor::Cyan, &held.name),
                    theme.paint(diagnostics::theme::AuraColor::Muted, &held.old),
                    held.new
                );
            }
            if summary.updated.is_empty() && summary.held.is_empty() {
                println!("Already up to date.");
            }
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
