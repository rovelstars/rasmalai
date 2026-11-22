use diagnostics::theme::{AuraColor, AuraTheme};

pub(super) fn run_doctor(repair_std: bool, registry: Option<String>, quiet: bool) {
    if repair_std {
        return super::download::repair_stdlib(registry, quiet);
    }
    let theme = AuraTheme::active();
    let stdlib_missing = match frontend::stdlib_seed::read_std_pin() {
        None => {
            println!("stdlib cache: no pin file");
            println!(
                "  {}",
                theme.paint(
                    AuraColor::Muted,
                    "Run `rnx download` to seed the standard library cache."
                )
            );
            true
        }
        Some(pin) => {
            let missing = frontend::stdlib_seed::missing_std_packages();
            if missing.is_empty() {
                println!(
                    "stdlib cache: {} packages, {}",
                    cli::term::human_count(pin.packages.len() as u64),
                    theme.paint(AuraColor::Green, "complete")
                );
                match frontend::products::std_products_summary() {
                    Some((files, pkgs)) if files > 0 => {
                        println!(
                            "stdlib products: precompiled ({} files across {} packages)",
                            cli::term::human_count(files as u64),
                            cli::term::human_count(pkgs as u64)
                        );
                    }
                    _ => {
                        println!("stdlib products: absent");
                        println!(
                            "  {}",
                            theme.paint(
                                AuraColor::Muted,
                                "Run `rnx download` to precompile them."
                            )
                        );
                    }
                }
                false
            } else {
                println!(
                    "stdlib cache: {} package{} missing ({})",
                    cli::term::human_count(missing.len() as u64),
                    if missing.len() == 1 { "" } else { "s" },
                    missing.join(", ")
                );
                println!(
                    "  {}",
                    theme.paint(
                        AuraColor::Muted,
                        "Run `rnx download` or `rnx doctor --repair-std`."
                    )
                );
                true
            }
        }
    };
    print_product_status();
    if stdlib_missing {
        std::process::exit(1);
    }
}

fn print_product_status() {
    let plain = AuraTheme::plain();
    let (files, bytes) = frontend::products::products_usage();
    println!(
        "products: {} files using {} under {}",
        cli::term::human_count(files as u64),
        cli::telemetry::fmt_bytes(&plain, bytes),
        frontend::products::products_root().display()
    );
    println!(
        "product objects: {} under {}",
        cli::telemetry::fmt_bytes(&plain, frontend::products::global_objects_usage_bytes()),
        frontend::products::global_objects_root().display()
    );
    let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
    let root = frontend::project::find_project_root(&cwd).unwrap_or(cwd);
    for status in frontend::modules::product_statuses(&root) {
        match &status.version {
            Some(version) => {
                println!(
                    "dep {}@{}: {}",
                    status.pkg,
                    version,
                    if status.present { "cached" } else { "absent" }
                );
            }
            None => {
                println!(
                    "dep {}: unpinned (a floating range never caches)",
                    status.pkg
                );
            }
        }
    }
}
