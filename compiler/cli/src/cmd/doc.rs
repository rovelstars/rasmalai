use super::*;

pub(super) fn run_doc(package: Option<String>, open: bool, no_deps: bool, all: bool, json: bool, out_dir: Option<std::path::PathBuf>, stdlib: bool, quiet: bool) {
            let include_private = all;
            let cwd = std::env::current_dir().unwrap_or_else(|e| {
                eprintln!("error: cannot read working directory: {e}");
                std::process::exit(1);
            });
            if json && stdlib {
                let dir = out_dir
                    .unwrap_or_else(|| cwd.join("target").join("doc"));
                match cli::generate_stdlib_docs_json(&dir) {
                    Ok(path) => {
                        if !quiet {
                            println!("docs written to {}", path.display());
                        }
                    }
                    Err(e) => {
                        println!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            if json {
                match cli::generate_docs_json(&cwd, package.as_deref(), no_deps, include_private) {
                    Ok(path) => {
                        if !quiet {
                            println!("docs written to {}", path.display());
                        }
                    }
                    Err(e) => {
                        println!("{e}");
                        std::process::exit(1);
                    }
                }
                return;
            }
            match cli::generate_docs(&cwd, package.as_deref(), no_deps, include_private) {
                Ok(index) => {
                    if !quiet {
                        println!("docs written to {}", index.display());
                    }
                    if open && open_browser(&index).is_err() {
                        eprintln!("warning: could not open {}", index.display());
                    }
                }
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
