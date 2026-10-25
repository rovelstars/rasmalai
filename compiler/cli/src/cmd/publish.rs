fn run_artifact_pipeline(
    dir: &std::path::Path,
    cfg: &frontend::project::ProjectConfig,
    bare: &str,
    base: &str,
    push_token: Option<&str>,
) -> String {
    if cfg.bindgen.is_some()
        && let Err(e) = cli::task::run_bindgen_driver(dir, cfg)
    {
        println!("{e}");
        std::process::exit(1);
    }
    let inputs = match cli::publish::collect_pipeline_inputs(dir, cfg) {
        Ok(inputs) if !inputs.is_empty() => inputs,
        Ok(_) => {
            println!("error[E108]: nothing to push: no bindgen outputs and no files under `{}/`", cli::publish::PUBLISH_STAGE_DIR);
            println!("hint: declare `bindgen.outputs` or stage per-target blobs, then retry");
            std::process::exit(1);
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    };
    let assembly = match frontend::oci::assemble_index(&inputs) {
        Ok(assembly) => assembly,
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    };
    let hexes = frontend::oci::assembly_blob_hexes(&assembly);
    for input in &inputs {
        let hex = frontend::checksum::Sha256::hexdigest(&input.bytes);
        if !hexes.contains(&hex) {
            println!(
                "error[E108]: pipeline file `{}` is not in the assembled index (stale stage or stale push)",
                input.title
            );
            std::process::exit(1);
        }
    }
    let (_, repo) = match cli::publish::artifact_repo_parts(bare) {
        Ok(parts) => parts,
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = cli::publish::push_assembly(base, &repo, push_token, &assembly) {
        println!("{e}");
        std::process::exit(1);
    }
    eprintln!("notice: pushed artifact image for `{bare}` ({} file(s), index {})", inputs.len(), assembly.digest);
    assembly.digest
}

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
    let pushed = match cfg.artifacts.as_deref() {
        Some(bare) => match cli::publish::push_plan() {
            Some((base, push_token)) => Some(run_artifact_pipeline(dir, cfg, bare, &base, push_token.as_deref())),
            None => None,
        },
        None => None,
    };
    let asserted = match (pushed.as_deref(), artifact) {
        (Some(pushed), Some(raw)) => match cli::publish::parse_artifact_digest(&raw) {
            Ok(flag) if flag == pushed => Some(pushed.to_string()),
            Ok(flag) => {
                println!("error[E108]: artifact digest mismatch (--artifact asserts {flag}, the pipeline pushed {pushed})");
                std::process::exit(1);
            }
            Err(e) => {
                println!("{e}");
                std::process::exit(1);
            }
        },
        (Some(pushed), None) => Some(pushed.to_string()),
        (None, Some(raw)) => match cli::publish::parse_artifact_digest(&raw) {
            Ok(d) => Some(d),
            Err(e) => {
                println!("{e}");
                std::process::exit(1);
            }
        },
        (None, None) => None,
    };
    if cfg.artifacts.is_some() && asserted.is_none() {
        println!(
            "error[E108]: package declares artifacts `{}` but no --artifact digest was asserted",
            cfg.artifacts.as_deref().unwrap_or_default()
        );
        println!(
            "hint: pass `--artifact sha256:<hex>` with the index digest you pinned out-of-band, or set {} to push the pipeline image",
            cli::task::PUSH_URL_ENV
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
    if pushed.is_none()
        && let (Some(bare), Some(digest)) = (cfg.artifacts.as_deref(), asserted.as_deref())
    {
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
    if let Err(e) = cli::publish::publish_license(cfg, dir) {
        println!("{e}");
        std::process::exit(1);
    }
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
