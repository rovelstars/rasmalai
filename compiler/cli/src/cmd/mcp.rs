
pub(super) fn run_mcp() {
            let rt = tokio::runtime::Builder::new_current_thread()
                .enable_all()
                .build()
                .unwrap_or_else(|e| {
                    eprintln!("error: cannot start the async runtime: {e}");
                    std::process::exit(1);
                });
            if let Err(e) = rt.block_on(cli::mcp::serve()) {
                eprintln!("{e}");
                std::process::exit(1);
            }
}
