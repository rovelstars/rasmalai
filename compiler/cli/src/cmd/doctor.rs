pub(super) fn run_doctor(repair_std: bool, registry: Option<String>) {
    if repair_std {
        return repair_stdlib(registry);
    }
    let stdlib_missing = match frontend::stdlib_seed::read_std_pin() {
        None => {
            println!("stdlib cache: no pin file; run `rnx fetch-std` to seed the global cache");
            true
        }
        Some(pin) => {
            let missing = frontend::stdlib_seed::missing_std_packages();
            if missing.is_empty() {
                println!("stdlib cache: ok ({} packages)", pin.packages.len());
                match frontend::products::std_products_summary() {
                    Some((files, pkgs)) if files > 0 => {
                        println!("stdlib products: precompiled ({files} files, {pkgs} packages)");
                    }
                    _ => {
                        println!("stdlib products: absent (run `rnx fetch-std` to precompile them)");
                    }
                }
                false
            } else {
                println!(
                    "stdlib cache: {} missing ({}); run `rnx doctor --repair-std`",
                    missing.len(),
                    missing.join(", ")
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
    let (files, bytes) = frontend::products::products_usage();
    println!(
        "products: {files} files ({} bytes) under {}",
        bytes,
        frontend::products::products_root().display()
    );
    println!(
        "product objects: {} bytes under {}",
        frontend::products::global_objects_usage_bytes(),
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
                println!("dep {}: unpinned (floating range never caches)", status.pkg);
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
            match frontend::products::precompile_std() {
                Ok(pre) => {
                    if pre.entries > 0 {
                        println!(
                            "Precompiled {} std products ({} entries in {} projects)",
                            pre.populated, pre.entries, pre.projects
                        );
                    }
                }
                Err(reason) => {
                    println!(
                        "warning: stdlib precompile skipped ({reason}); first build will populate products"
                    );
                }
            }
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
