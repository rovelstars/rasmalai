
pub(super) fn run_pack(package: Option<String>, out_dir: Option<std::path::PathBuf>, gzip: bool) {
            let cwd = std::env::current_dir().unwrap_or_else(|e| {
                eprintln!("error: cannot read working directory: {e}");
                std::process::exit(1);
            });
            let targets = match cli::resolve_pack_targets(&cwd, package.as_deref()) {
                Ok(t) => t,
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            };
            let out = out_dir
                .unwrap_or_else(|| targets.root.join("target").join("package"));
            match cli::pack_targets(&targets, &out, gzip) {
                Ok(done) => {
                    for (name, tar) in done {
                        let show = tar
                            .strip_prefix(&targets.root)
                            .map(|p| p.display().to_string())
                            .unwrap_or_else(|_| tar.display().to_string());
                        let version = targets
                            .packages
                            .iter()
                            .find(|(n, _, _)| n == &name)
                            .map(|(_, _, c)| c.version.clone())
                            .unwrap_or_default();
                        let bytes = std::fs::read(&tar).unwrap_or_default();
                        let digest = frontend::checksum::Sha256::hexdigest(&bytes);
                        let size = bytes.len();
                        let size_show = if size >= 1024 {
                            format!("{:.1} KB", size as f64 / 1024.0)
                        } else {
                            format!("{size} B")
                        };
                        let kind = if gzip { "compressed" } else { "uncompressed" };
                        println!("┌─ pack: {name} [v{version}]");
                        println!("│  archive:   {show}");
                        println!("│  checksum:  {digest}");
                        println!("│  size:      {size_show} ({kind})");
                        println!("└─ status: ready for publishing");
                    }
                }
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
