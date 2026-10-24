
pub(super) fn run_publish(registry: String, token: Option<String>) {
            let token = match token {
                Some(t) => t,
                None => {
                    println!("┌─ error[E501]: missing publisher token");
                    println!("│  cannot publish without an authenticated token");
                    println!("│");
                    println!("└─ hint: pass --token <token> or set RNX_TOKEN in your environment");
                    std::process::exit(1);
                }
            };
            let cwd = std::env::current_dir().unwrap_or_else(|e| {
                eprintln!("error: cannot read working directory: {e}");
                std::process::exit(1);
            });
            let targets = match cli::resolve_pack_targets(&cwd, None) {
                Ok(t) => t,
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            };
            let (name, dir, cfg) = targets.packages.first().unwrap_or_else(|| {
                eprintln!("error: no package found; run `rnx publish` inside a project");
                std::process::exit(1);
            });
            let manifest = frontend::project::Manifest {
                project: Some(cfg.clone()),
                workspace: None,
            };
            if let Err(e) = cli::publish::gate_formatted(dir, &manifest) {
                println!("{e}");
                std::process::exit(1);
            }
            let files = match cli::publish::collect_publish_files(dir, &manifest) {
                Ok(f) => f,
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            };
            let docs = match cli::publish::package_docs(dir, &manifest) {
                Ok(d) => d,
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            };
            let readme = frontend::pack::package_readme_text(dir, &cfg.name);
            let frame = cli::publish::publish_frame(cfg, &readme, &files, &docs);
            match cli::publish::post_frame(&registry, &token, &frame, name) {
                Ok(cli::publish::PublishOutcome::Published { url, digest }) => {
                    println!("┌─ published: {name} [v{}]", cfg.version);
                    println!("│  url: {url}");
                    println!("│  digest: {digest}");
                    println!("└─ status: live");
                }
                Ok(cli::publish::PublishOutcome::Rejected { status, message }) => {
                    println!("┌─ error: publish rejected [HTTP {status}]");
                    println!("│  {message}");
                    println!("└─ hint: check your token and package version");
                    std::process::exit(1);
                }
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
