
pub(super) fn run_vendor(packages: Vec<String>, dir: Option<std::path::PathBuf>) {
    match cli::vendor_scope_packages(&packages, dir.as_deref()) {
        Ok((names, dest)) => {
            let theme = diagnostics::theme::AuraTheme::active();
            for name in &names {
                println!(
                    "{} {}",
                    theme.paint(diagnostics::theme::AuraColor::Purple, "Vendored"),
                    theme.paint(diagnostics::theme::AuraColor::Cyan, name)
                );
            }
            println!(
                "{} {} packages into {}",
                theme.paint(diagnostics::theme::AuraColor::Purple, "Vendored"),
                theme.paint(
                    diagnostics::theme::AuraColor::Orange,
                    &names.len().to_string()
                ),
                theme.paint(
                    diagnostics::theme::AuraColor::Cyan,
                    &dest.display().to_string()
                )
            );
        }
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
