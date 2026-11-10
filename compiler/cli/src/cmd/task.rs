fn select_project(package: Option<&str>) -> Result<cli::task::TaskProject, diagnostics::Diagnostic> {
    let cwd = std::env::current_dir().map_err(|e| {
        diagnostics::Diagnostic::new(diagnostics::Code::E108, format!("cannot read working directory: {e}"))
    })?;
    match package {
        Some(name) => {
            let ws_root = frontend::project::find_workspace_root_strict(&cwd).ok_or_else(|| {
                diagnostics::Diagnostic::new(
                    diagnostics::Code::E108,
                    format!("unknown workspace member `{name}`"),
                )
            })?;
            let (dir, _) = cli::resolve_member(&ws_root, name)?;
            cli::task::TaskProject::load(dir, ws_root)
        }
        None => {
            let dir = frontend::project::find_project_root(&cwd).ok_or_else(|| {
                diagnostics::Diagnostic::new(
                    diagnostics::Code::E108,
                    "No file specified and no Project.config found",
                )
            })?;
            let scope_root =
                frontend::project::find_workspace_root_strict(&dir).unwrap_or_else(|| dir.clone());
            cli::task::TaskProject::load(dir, scope_root)
        }
    }
}

pub(super) fn run_task(
    name: Option<String>,
    package: Option<String>,
    all: bool,
    explain: bool,
    allow: Vec<String>,
    force: bool,
    watch: bool,
    prefer: frontend::native::NativePrefer,
) {
    let project = match select_project(package.as_deref()) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    let result = match name {
        None if explain => Err(diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            "`--explain` needs a task reference",
        )
        .with_hint("run `rnx task --explain <name>` or `rnx task --explain <package>#<task>`")),
        None => {
            if all {
                let registry = match cli::task::TaskRegistry::build(&project) {
                    Ok(r) => r,
                    Err(e) => {
                        eprintln!("{e}");
                        std::process::exit(1);
                    }
                };
                let listed = cli::task::list_all_tasks(&registry);
                if listed.is_empty() {
                    println!("no tasks declared (add `tasks: {{ <name>: [...] }}` to Project.config)");
                    return;
                }
                for (task, detail) in listed {
                    println!("{task}: {detail}");
                }
            } else {
                let listed = cli::task::list_tasks(&project.config);
                if listed.is_empty() {
                    println!("no tasks declared (add `tasks: {{ <name>: [...] }}` to Project.config)");
                    return;
                }
                for (task, kind) in listed {
                    println!("{task}: {kind}");
                }
            }
            return;
        }
        Some(task) if watch && !explain => {
            let root = project.dir.clone();
            println!("watching {} (Ctrl-C to stop)", root.display());
            loop {
                let options = cli::task::RunOptions::new(allow.clone(), force, false);
                if let Err(e) = cli::task::run_task(&project, &task, &options, prefer) {
                    eprintln!("{e}");
                }
                let baseline = cli::task::tree_fingerprint(&root);
                loop {
                    std::thread::sleep(std::time::Duration::from_millis(400));
                    if cli::task::tree_fingerprint(&root) != baseline {
                        println!("change detected, rerunning");
                        break;
                    }
                }
            }
        }
        Some(task) => {
            if explain {
                cli::task::explain_task(&project, &task, prefer)
            } else {
                let options = cli::task::RunOptions::new(allow, force, false);
                cli::task::run_task(&project, &task, &options, prefer)
            }
        }
    };
    if let Err(e) = result {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
