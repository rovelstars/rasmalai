
pub(super) fn run_unpack(archive: Option<String>, out_dir: Option<String>) {
            let archive = archive.unwrap_or_else(|| {
                eprintln!("usage: rnx unpack <archive> [--out-dir <dir>]");
                std::process::exit(2);
            });
            let dest = out_dir.unwrap_or_else(|| ".".to_string());
            match cli::unpack::unpack_archive(
                std::path::Path::new(&archive),
                std::path::Path::new(&dest),
            ) {
                Ok((n, bytes)) => {
                    println!("Extracted {n} files ({bytes} bytes) into {dest}");
                }
                Err(e) => {
                    println!("{e}");
                    std::process::exit(1);
                }
            }
}
