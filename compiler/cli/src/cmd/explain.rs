use cli::explain;

pub(super) fn run_explain(code: Option<String>) {
            let code = code.as_deref().unwrap_or("");
            match explain(code) {
                Some((title, fix)) => {
                    let theme = diagnostics::theme::AuraTheme::active();
                    println!(
                        "{}  {}",
                        theme.paint(diagnostics::theme::AuraColor::Red, code),
                        theme.paint(diagnostics::theme::AuraColor::Text, &title)
                    );
                    println!();
                    println!(
                        "{} {}",
                        theme.paint(diagnostics::theme::AuraColor::Muted, "fix:"),
                        fix
                    );
                }
                None => {
                    eprintln!("Unknown diagnostic code `{code}`.");
                    eprintln!("Run `rnx explain E108` to see the shape of a registered code.");
                    std::process::exit(1);
                }
            }
}
