
pub(super) fn run_vendor(package: Option<String>, stdlib: Option<std::path::PathBuf>) {
    if let Some(dir) = stdlib {
        match cli::stage_std_vendor(&dir) {
            Ok(names) => println!(
                "Staged {} stdlib packages into {}",
                names.len(),
                dir.display()
            ),
            Err(e) => {
                println!("{e}");
                std::process::exit(1);
            }
        }
        if package.is_none() {
            return;
        }
    }
    match cli::vendor_scope(package.as_deref()) {
        Ok(names) => println!("Vendored {} packages into vendor/", names.len()),
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
