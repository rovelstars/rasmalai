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

pub(super) fn run_fetch_std(registry: Option<String>) {
    let (default, overrides) = std_registry(registry.as_deref());
    match frontend::stdlib_seed::seed_stdlib_cache(default.as_ref(), &overrides) {
        Ok(report) => {
            for (name, version) in &report.packages {
                println!("Fetched {name}@{version}");
            }
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
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
