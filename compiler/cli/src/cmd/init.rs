
pub(super) fn run_init(name: Option<String>) {
            let name = name.as_deref().unwrap_or_else(|| {
                eprintln!("usage: rnx init <name>");
                std::process::exit(2);
            });
            let cwd = std::env::current_dir().unwrap_or_else(|e| {
                eprintln!("error: cannot read working directory: {e}");
                std::process::exit(1);
            });
            match cli::init_project(name, &cwd) {
                Ok(root) => {
                    println!("Initialized project `{name}` in {}", root.display());
                    println!("  manifest: Project.config");
                    println!("  entry: src/main.rnx");
                    println!("Next: cd {name} && rnx run");
                }
                Err(e) => {
                    eprintln!("error: {e}");
                    std::process::exit(1);
                }
            }
}
