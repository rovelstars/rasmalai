
pub(super) fn run_publish(tarball: Option<std::path::PathBuf>, registry: String, token: Option<String>) {
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
            let (bytes, meta) = match tarball {
                Some(path) => {
                    match cli::publish::read_tarball_meta(std::path::Path::new(&path), &cwd) {
                        Ok(done) => done,
                        Err(e) => {
                            println!("{e}");
                            std::process::exit(1);
                        }
                    }
                }
                None => {
                    let targets = match cli::resolve_pack_targets(&cwd, None) {
                        Ok(t) => t,
                        Err(e) => {
                            println!("{e}");
                            std::process::exit(1);
                        }
                    };
                    let tmp = std::env::temp_dir().join(format!("rnx-publish-{}", std::process::id()));
                    let packed = match cli::pack_targets(&targets, &tmp, true) {
                        Ok(done) => done,
                        Err(e) => {
                            println!("{e}");
                            std::process::exit(1);
                        }
                    };
                    let (name, tar) = packed.first().unwrap_or_else(|| {
                        eprintln!("error: pack produced no archive");
                        std::process::exit(1);
                    });
                    let bytes = std::fs::read(tar).unwrap_or_else(|e| {
                        eprintln!("error: cannot read {}: {e}", tar.display());
                        std::process::exit(1);
                    });
                    let version = targets
                        .packages
                        .iter()
                        .find(|(n, _, _)| n == name)
                        .map(|(_, _, c)| c.version.clone())
                        .unwrap_or_default();
                    let checksum = frontend::checksum::Sha256::hexdigest(&bytes);
                    let _ = std::fs::remove_dir_all(&tmp);
                    (bytes, cli::publish::PackageMeta { name: name.clone(), version, checksum })
                }
            };
            match cli::publish::post_package(&registry, &token, &bytes, &meta) {
                Ok(cli::publish::PublishOutcome::Published { url }) => {
                    println!("┌─ published: {} [v{}]", meta.name, meta.version);
                    println!("│  url: {url}");
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
