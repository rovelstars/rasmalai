
pub(super) fn run_fetch(package: Option<String>) {
            match cli::fetch_scope(package.as_deref()) {
                Ok(fetched) => {
                    if fetched.is_empty() {
                        println!("No git dependencies to fetch");
                    }
                    for (name, _, rev) in &fetched {
                        println!("Fetched {name} ({rev})");
                    }
                }
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
