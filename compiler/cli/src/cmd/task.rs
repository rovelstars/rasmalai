pub(super) fn run_task(name: Option<String>, prefer: frontend::native::NativePrefer) {
    let cwd = std::env::current_dir().unwrap_or_else(|e| {
        eprintln!("error: cannot read working directory: {e}");
        std::process::exit(1);
    });
    let root = frontend::project::find_project_root(&cwd).unwrap_or_else(|| {
        eprintln!("error: No file specified and no Project.config found");
        std::process::exit(1);
    });
    let text = std::fs::read_to_string(root.join(frontend::project::MANIFEST_FILE)).unwrap_or_else(|e| {
        eprintln!("error: cannot read {}: {e}", root.join(frontend::project::MANIFEST_FILE).display());
        std::process::exit(1);
    });
    let manifest = frontend::project::load_manifest(&root).unwrap_or_else(|e| {
        println!("{e}");
        std::process::exit(1);
    });
    let manifest = manifest.unwrap_or_else(|| {
        eprintln!("error: {} has no manifest table", root.display());
        std::process::exit(1);
    });
    let cfg = manifest.project.unwrap_or_else(|| {
        eprintln!("error: {} has no `project` table; tasks live beside `project`", root.display());
        std::process::exit(1);
    });
    match name {
        None => {
            let listed = cli::task::list_tasks(&cfg);
            if listed.is_empty() {
                println!("no tasks declared (add `tasks: {{ <name>: [...] }}` to Project.config)");
                return;
            }
            for (task, kind) in listed {
                println!("{task}: {kind}");
            }
        }
        Some(task) => {
            if let Err(e) = cli::task::run_task_text(&root, &text, &cfg, &task, prefer) {
                println!("{e}");
                std::process::exit(1);
            }
        }
    }
}
