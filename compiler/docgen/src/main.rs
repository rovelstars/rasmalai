fn main() {
    let out_dir = std::env::args().nth(1).unwrap_or_else(|| {
        eprintln!("usage: docgen <out-dir>");
        std::process::exit(1);
    });
    match docgen::generate_stdlib_docs_json(std::path::Path::new(&out_dir)) {
        Ok(path) => println!("docs written to {}", path.display()),
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
