
pub(super) fn run_vendor(package: Option<String>) {
            match cli::vendor_scope(package.as_deref()) {
                Ok(names) => println!("Vendored {} packages into vendor/", names.len()),
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
