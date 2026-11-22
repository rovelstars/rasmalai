
pub(super) fn run_unpack(archive: Option<String>, out_dir: Option<String>) {
            let archive = archive.unwrap_or_else(|| {
                eprintln!("usage: rnx unpack <archive> [--out-dir <dir>]");
                std::process::exit(2);
            });
            let dest = out_dir.unwrap_or_else(|| ".".to_string());
            match cli::unpack::unpack_archive(
                std::path::Path::new(&archive),
                std::path::Path::new(&dest),
            ) {
                Ok((n, bytes)) => {
                    let theme = diagnostics::theme::AuraTheme::active();
                    println!(
                        "{} {} file{} ({}) into {}.",
                        theme.paint(
                            diagnostics::theme::AuraColor::Purple,
                            "Extracted"
                        ),
                        theme.paint(
                            diagnostics::theme::AuraColor::Orange,
                            &cli::term::human_count(n as u64)
                        ),
                        if n == 1 { "" } else { "s" },
                        cli::telemetry::fmt_bytes(&theme, bytes),
                        theme.paint(
                            diagnostics::theme::AuraColor::Cyan,
                            &dest
                        )
                    );
                }
                Err(e) => {
                    eprintln!("{e}");
                    std::process::exit(1);
                }
            }
}
