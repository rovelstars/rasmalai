pub(super) fn run_unpublish(target: String, registry: String, token: Option<String>) {
    let token = match token {
        Some(t) => t,
        None => {
            println!("┌─ error[E501]: missing publisher token");
            println!("│  cannot unpublish without an authenticated token");
            println!("└─ hint: pass --token <token> or set RNX_TOKEN in your environment");
            std::process::exit(1);
        }
    };
    let (name, version) = match target.rsplit_once('@') {
        Some((n, v)) if !n.is_empty() && !v.is_empty() => (n, v),
        _ => {
            println!("error[E108]: expected a version coordinate like `name@1.2.3`");
            std::process::exit(1);
        }
    };
    let request_id = cli::publish::new_request_id();
    match cli::publish::post_unpublish(&registry, &token, name, version, &request_id) {
        Ok(cli::publish::UnpublishOutcome::Removed) => {
            println!("┌─ unpublished: {name} [v{version}]");
            println!("└─ status: removed (the number stays burned)");
        }
        Ok(cli::publish::UnpublishOutcome::Rejected { status, message }) => {
            println!("┌─ error: unpublish rejected [HTTP {status}]");
            println!("│  {message}");
            println!("└─ hint: only versions inside the 72-hour window can be unpublished");
            std::process::exit(1);
        }
        Err(e) => {
            println!("{e}");
            std::process::exit(1);
        }
    }
}
