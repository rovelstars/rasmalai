use diagnostics::{Code, Diagnostic};
use frontend::project::RegistryConfig;
use std::collections::BTreeMap;

pub(super) fn std_registry(
    flag: Option<&str>,
) -> (Option<RegistryConfig>, BTreeMap<String, RegistryConfig>) {
    let url = flag.map(|s| s.to_string()).or_else(|| {
        std::env::var("RNX_REGISTRY")
            .ok()
            .filter(|s| !s.trim().is_empty())
    });
    let default = url.map(|u| RegistryConfig {
        url: u,
        token_env: None,
        ca_cert: None,
    });
    (default, BTreeMap::new())
}

pub(super) fn run_fetch_std(registry: Option<String>, local: bool, from: Option<std::path::PathBuf>) {
    if from.is_some() {
        let e = Diagnostic::new(
            Code::E108,
            "`rnx fetch-std --from` was removed; stage with `rnx vendor --stdlib <DIR>` and point `RNX_VENDOR_DIR` at it (or copy it over `vendor/`) to override locally, or pass `--local` to seed the cache from the toolchain's embedded sources",
        );
        println!("{e}");
        std::process::exit(1);
    }
    if local {
        return run_fetch_std_local();
    }
    let (default, overrides) = std_registry(registry.as_deref());
    match frontend::stdlib_seed::seed_stdlib_cache(default.as_ref(), &overrides) {
        Ok(report) => {
            for (name, version) in &report.packages {
                println!("Fetched {name}@{version}");
            }
            finish_fetch_std(&report);
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}

fn run_fetch_std_local() {
    match frontend::stdlib_seed::seed_embedded_std_cache() {
        Ok(report) => {
            for (name, version) in &report.packages {
                println!("Seeded {name}@{version} from the toolchain");
            }
            finish_fetch_std(&report);
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}

fn finish_fetch_std(report: &frontend::stdlib_seed::StdSeedReport) {
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
