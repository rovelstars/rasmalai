use frontend::cache;

pub(super) fn run_cache(action: cli::args::CacheAction) {
    match action {
        cli::args::CacheAction::Prune { max_bytes } => {
            let cap = max_bytes.unwrap_or(cache::DEFAULT_RETENTION_BYTES);
            let dir = cache::global_cache_dir();
            match cache::prune_lru(&dir, cap) {
                Ok(freed) => {
                    println!(
                        "pruned {} bytes from {} (usage now {} bytes)",
                        freed,
                        dir.display(),
                        cache::cache_usage_bytes(&dir)
                    );
                }
                Err(e) => {
                    eprintln!("error: cannot prune {}: {e}", dir.display());
                    std::process::exit(1);
                }
            }
        }
        cli::args::CacheAction::Status => {
            let global = cache::global_cache_dir();
            println!("global: {} ({} bytes)", global.display(), cache::cache_usage_bytes(&global));
            let cwd = std::env::current_dir().unwrap_or_else(|_| std::path::PathBuf::from("."));
            let root = frontend::project::find_project_root(&cwd).unwrap_or(cwd);
            let project = cache::project_cache_dir(&root);
            println!("project: {} ({} bytes)", project.display(), cache::cache_usage_bytes(&project));
        }
    }
}

pub(super) fn run_clean(package: Option<String>) {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("error: cannot read working directory: {e}");
        std::process::exit(1);
    });
    let root = if let Some(_pkg) = package.as_deref() {
        match frontend::project::find_workspace_root(&cwd) {
            Some(r) => r,
            None => {
                eprintln!("error: no workspace found");
                std::process::exit(1);
            }
        }
    } else {
        frontend::project::find_project_root(&cwd).unwrap_or(cwd)
    };
    let cache_dir = cache::project_cache_dir(&root);
    match cache::clean_dir(&cache_dir) {
        Ok(freed) => println!("cleaned {} ({} bytes freed)", cache_dir.display(), freed),
        Err(e) => {
            eprintln!("error: cannot clean {}: {e}", cache_dir.display());
            std::process::exit(1);
        }
    }
}
