
pub(super) fn run_vendor(packages: Vec<String>, dir: Option<std::path::PathBuf>) {
    match cli::vendor_scope_packages(&packages, dir.as_deref()) {
        Ok((names, dest)) => {
            for name in &names {
                println!("Vendored {name}");
            }
            println!("Vendored {} packages into {}", names.len(), dest.display());
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
