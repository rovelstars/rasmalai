use diagnostics::theme::{AuraColor, AuraTheme};
use frontend::cache;

fn size(theme: &AuraTheme, bytes: u64) -> String {
    cli::telemetry::fmt_bytes(theme, bytes)
}

fn row(theme: &AuraTheme, label: &str, amount: &str, detail: &str) {
    let label = format!("{label:<9}");
    let amount = format!("{amount:>10}");
    println!(
        "{}{}{}{}",
        theme.paint(AuraColor::Purple, &label),
        theme.paint(AuraColor::Orange, &amount),
        if detail.is_empty() { "" } else { "  " },
        theme.paint(AuraColor::Muted, detail)
    );
}

pub(super) fn run_cache(action: cli::args::CacheAction) {
    let theme = AuraTheme::active();
    match action {
        cli::args::CacheAction::Prune { max_bytes } => {
            let cap = max_bytes.unwrap_or(cache::DEFAULT_RETENTION_BYTES);
            let global = cache::global_cache_dir();
            match cache::prune_lru(&global, cap) {
                Ok(freed) => println!(
                    "Pruned {} from the global cache; usage is now {}.",
                    size(&theme, freed),
                    size(&theme, cache::cache_usage_bytes(&global))
                ),
                Err(e) => {
                    eprintln!("error: cannot prune {}: {e}", global.display());
                    std::process::exit(1);
                }
            }
            let freed_old = frontend::fetch::prune_old_registry_tree();
            if freed_old > 0 {
                println!(
                    "Removed the old registry tree, freeing {}.",
                    size(&theme, freed_old)
                );
            }
            let freed_objects = frontend::products::prune_global_objects_lru(cap);
            println!(
                "Pruned {} of product objects; usage is now {}.",
                size(&theme, freed_objects),
                size(&theme, frontend::products::global_objects_usage_bytes())
            );
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let root = frontend::project::find_project_root(&cwd).unwrap_or(cwd);
            let project = cache::project_cache_dir(&root);
            match cache::prune_lru(&project, cap) {
                Ok(freed) => println!(
                    "Pruned {} from the project cache; usage is now {}.",
                    size(&theme, freed),
                    size(&theme, cache::cache_usage_bytes(&project))
                ),
                Err(e) => {
                    eprintln!("error: cannot prune {}: {e}", project.display());
                    std::process::exit(1);
                }
            }
        }
        cli::args::CacheAction::Status => {
            let global = cache::global_cache_dir();
            row(
                &theme,
                "global",
                &size(&theme, cache::cache_usage_bytes(&global)),
                &global.display().to_string(),
            );
            row(
                &theme,
                "store",
                &size(&theme, frontend::fetch::store_usage_bytes()),
                &frontend::fetch::store_root().display().to_string(),
            );
            let (product_files, product_bytes) = frontend::products::products_usage();
            row(
                &theme,
                "products",
                &size(&theme, product_bytes),
                &format!("{} files", cli::term::human_count(product_files as u64)),
            );
            row(
                &theme,
                "objects",
                &size(&theme, frontend::products::global_objects_usage_bytes()),
                "",
            );
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let root = frontend::project::find_project_root(&cwd).unwrap_or(cwd);
            let project = cache::project_cache_dir(&root);
            row(
                &theme,
                "project",
                &size(&theme, cache::cache_usage_bytes(&project)),
                &project.display().to_string(),
            );
        }
    }
}

pub(super) fn run_clean(package: Option<String>) {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("error: cannot read working directory: {e}");
        std::process::exit(1);
    });
    let root = if let Some(pkg) = package.as_deref() {
        let ws_root = match frontend::project::find_workspace_root_strict(&cwd) {
            Some(r) => r,
            None => {
                eprintln!("error: no workspace found");
                std::process::exit(1);
            }
        };
        match cli::resolve_member(&ws_root, pkg) {
            Ok((dir, _)) => dir,
            Err(e) => {
                eprintln!("{e}");
                std::process::exit(1);
            }
        }
    } else {
        frontend::project::find_project_root(&cwd).unwrap_or(cwd)
    };
    let cache_dir = cache::project_cache_dir(&root);
    match cache::clean_dir(&cache_dir) {
        Ok(freed) => println!(
            "Cleaned {}, freeing {}.",
            cache_dir.display(),
            size(&AuraTheme::active(), freed)
        ),
        Err(e) => {
            eprintln!("error: cannot clean {}: {e}", cache_dir.display());
            std::process::exit(1);
        }
    }
}
