pub(super) fn run_doctor(repair_std: bool, registry: Option<String>) {
    if repair_std {
        return repair_stdlib(registry);
    }
    match frontend::stdlib_seed::read_std_pin() {
        None => {
            println!("stdlib cache: no pin file; run `rnx fetch-std` to seed the global cache");
            std::process::exit(1);
        }
        Some(pin) => {
            let missing = frontend::stdlib_seed::missing_std_packages();
            if missing.is_empty() {
                println!("stdlib cache: ok ({} packages)", pin.packages.len());
            } else {
                println!(
                    "stdlib cache: {} missing ({}); run `rnx doctor --repair-std`",
                    missing.len(),
                    missing.join(", ")
                );
                std::process::exit(1);
            }
        }
    }
}

fn repair_stdlib(registry: Option<String>) {
    let (default, overrides) = super::fetch_std::std_registry(registry.as_deref());
    let before: std::collections::BTreeSet<String> = frontend::fetch::scan_cache_have()
        .iter()
        .map(|h| format!("{}@{}", h.full, h.version))
        .collect();
    match frontend::stdlib_seed::seed_stdlib_cache(default.as_ref(), &overrides) {
        Ok(report) => {
            let repaired = report
                .packages
                .iter()
                .filter(|(name, version)| !before.contains(&format!("{name}@{version}")))
                .count();
            if repaired == 0 {
                println!(
                    "stdlib cache: ok ({} packages, none missing)",
                    report.packages.len()
                );
            } else {
                println!(
                    "stdlib cache: repaired {repaired} of {} packages",
                    report.packages.len()
                );
            }
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
