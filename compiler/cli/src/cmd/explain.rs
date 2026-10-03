use cli::explain;

pub(super) fn run_explain(code: Option<String>) {
            let code = code.as_deref().unwrap_or("");
            match explain(code) {
                Some((title, fix)) => println!("{code}: {title}\nfix: {fix}"),
                None => {
                    eprintln!("unknown code `{code}`");
                    std::process::exit(1);
                }
            }
}
