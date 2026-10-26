use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;

pub(super) fn registry_from_flag(
    flag: Option<&str>,
) -> (Option<frontend::project::RegistryConfig>, BTreeMap<String, frontend::project::RegistryConfig>) {
    let url = flag.map(|s| s.to_string()).or_else(|| {
        std::env::var("RNX_REGISTRY")
            .ok()
            .filter(|s| !s.trim().is_empty())
    });
    let default = url.map(|u| frontend::project::RegistryConfig {
        url: u,
        token_env: None,
        ca_cert: None,
    });
    (default, BTreeMap::new())
}

pub(super) fn run_download(packages: Vec<String>, local: bool, registry: Option<String>) {
    let (default, overrides) = registry_from_flag(registry.as_deref());
    if packages.is_empty() {
        return run_stdlib_seed(default.as_ref(), &overrides, local);
    }
    let mut reqs = BTreeMap::new();
    for raw in &packages {
        match frontend::deplock::parse_update_selector(raw) {
            Ok(sel) => {
                let range = sel.selector.unwrap_or_else(|| "*".to_string());
                if !frontend::fetch::is_valid_range(&range) {
                    println!(
                        "{}",
                        Diagnostic::new(
                            Code::E108,
                            format!("requirement for `{}` is not a valid range (*, latest, X.Y.Z, ^, ~, >=)", sel.name),
                        )
                    );
                    std::process::exit(1);
                }
                reqs.insert(sel.name, range);
            }
            Err(_) => {
                println!(
                    "{}",
                    Diagnostic::new(
                        Code::E108,
                        format!("bad download target `{raw}`: expected `name` or `name@range`"),
                    )
                );
                std::process::exit(1);
            }
        }
    }
    let scope = if local {
        Some(local_scope_root())
    } else {
        None
    };
    match frontend::fetch::download_packages(&reqs, default.as_ref(), &overrides, scope.as_deref()) {
        Ok(fetched) => {
            if local {
                let scope = scope.as_ref().unwrap();
                println!(
                    "Downloaded into {}",
                    frontend::fetch::project_registry_root(scope).display()
                );
                let std: Vec<(String, String)> = fetched
                    .iter()
                    .filter(|f| f.name.starts_with("@std/"))
                    .map(|f| (f.name.clone(), f.version.clone()))
                    .collect();
                if !std.is_empty() {
                    let base = frontend::fetch::registry_base_for(
                        &std[0].0,
                        default.as_ref(),
                        &overrides,
                    );
                    match frontend::stdlib_seed::merge_project_pin(scope, &base, &std) {
                        Ok(report) => {
                            println!("pinned {} packages to {}", report.packages.len(), report.pin_path.display());
                        }
                        Err(e) => {
                            println!("{e}");
                            std::process::exit(1);
                        }
                    }
                }
            }
            for f in &fetched {
                println!("Downloaded {}@{}", f.name, f.version);
            }
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}

fn local_scope_root() -> std::path::PathBuf {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("error: cannot read working directory: {e}");
        std::process::exit(1);
    });
    match frontend::project::find_workspace_root(&cwd) {
        Some(root) => root,
        None => {
            println!(
                "{}",
                Diagnostic::new(Code::E108, "no Project.config found; run `rnx init` first")
            );
            std::process::exit(1);
        }
    }
}

fn run_stdlib_seed(
    default: Option<&frontend::project::RegistryConfig>,
    overrides: &BTreeMap<String, frontend::project::RegistryConfig>,
    local: bool,
) {
    if local {
        let scope = local_scope_root();
        match frontend::stdlib_seed::seed_stdlib_cache_local(&scope, default, overrides) {
            Ok(report) => {
                for (name, version) in &report.packages {
                    println!("Downloaded {name}@{version}");
                }
                finish_local_seed(&scope, &report);
            }
            Err(e) => {
                println!("{e}");
                std::process::exit(1);
            }
        }
        return;
    }
    match frontend::stdlib_seed::seed_stdlib_cache(default, overrides) {
        Ok(report) => {
            for (name, version) in &report.packages {
                println!("Downloaded {name}@{version}");
            }
            finish_global_seed(&report);
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}

pub(super) fn finish_global_seed(report: &frontend::stdlib_seed::StdSeedReport) {
    println!(
        "pinned {} packages to {}",
        report.packages.len(),
        report.pin_path.display()
    );
    if let Ok(cwd) = std::env::current_dir()
        && let Some(scope_root) = frontend::project::find_workspace_root(&cwd)
    {
        let pre = frontend::products::precompile_scope(&scope_root);
        if pre.entries > 0 {
            println!(
                "Precompiled {} dep products ({} entries in {} projects)",
                pre.populated, pre.entries, pre.projects
            );
        }
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

fn finish_local_seed(scope: &std::path::Path, report: &frontend::stdlib_seed::StdSeedReport) {
    println!(
        "pinned {} packages to {}",
        report.packages.len(),
        report.pin_path.display()
    );
    let pre = frontend::products::precompile_scope(scope);
    if pre.entries > 0 {
        println!(
            "Precompiled {} dep products ({} entries in {} projects)",
            pre.populated, pre.entries, pre.projects
        );
    }
}

pub(super) fn repair_stdlib(registry: Option<String>) {
    let (default, overrides) = registry_from_flag(registry.as_deref());
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
