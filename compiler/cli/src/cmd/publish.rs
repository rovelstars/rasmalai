pub(super) fn run_publish(registry: String, token: Option<String>, artifact: Option<String>) {
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
    let asserted = match artifact {
        Some(raw) => match cli::publish::parse_artifact_digest(&raw) {
            Ok(d) => Some(d),
            Err(e) => {
                println!("{e}");
                std::process::exit(1);
            }
        },
        None => None,
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
    if cfg.artifacts.is_some() && asserted.is_none() {
        println!(
            "error[E108]: package declares artifacts `{}` but no --artifact digest was asserted",
            cfg.artifacts.as_deref().unwrap_or_default()
        );
        println!(
            "hint: pass `--artifact sha256:<hex>` with the index digest you pinned out-of-band"
        );
        std::process::exit(1);
    }
    if cfg.artifacts.is_none() && asserted.is_some() {
        println!(
            "error[E108]: --artifact digest asserted but Project.config declares no `artifacts`"
        );
        println!("hint: declare the bare OCI repo ref first, or drop the flag");
        std::process::exit(1);
    }
    if let (Some(bare), Some(digest)) = (cfg.artifacts.as_deref(), asserted.as_deref()) {
        match cli::publish::fetch_oci_index(bare) {
            Ok((bytes, served)) => {
                if served != digest {
                    println!("error[E108]: artifact digest mismatch (asserted {digest}, origin serves {served} for `{bare}`)");
                    std::process::exit(1);
                }
                let hexes = cli::publish::index_blob_hexes(bare, &bytes);
                if let Err(e) = cli::publish::cross_check_local_artifacts(dir, &hexes) {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
            Err(e) => {
                println!("warn: skipping local artifact cross-check: {e}");
            }
        }
    }
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
    let frame = cli::publish::publish_frame(cfg, &readme, &files, &docs, asserted.as_deref());
    match cli::publish::post_frame(&registry, &token, &frame, name) {
        Ok(cli::publish::PublishOutcome::Published {
            url,
            digest,
            artifact_digest,
        }) => {
            println!("┌─ published: {name} [v{}]", cfg.version);
            println!("│  url: {url}");
            println!("│  digest: {digest}");
            match artifact_digest {
                Some(a) => println!("│  artifact: {a}"),
                None => println!("│  artifact: none"),
            }
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
