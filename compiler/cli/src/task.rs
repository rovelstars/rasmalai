use diagnostics::{Code, Diagnostic};
use frontend::project::{BindgenDriver, ProjectConfig, TaskDef, TaskOnFailure, TaskStep, TaskStrategy};
use std::path::Path;

pub const BINDGEN_TASK: &str = "bindgen";
pub const PUSH_URL_ENV: &str = "RNX_OCI_PUSH_URL";
pub const PUSH_TOKEN_ENV: &str = "RNX_OCI_PUSH_TOKEN";

pub fn task_kind(def: &TaskDef) -> &'static str {
    match def {
        TaskDef::Steps { matrix, .. } if !matrix.is_empty() => "matrix",
        TaskDef::Steps { .. } => "steps",
        TaskDef::Group { .. } => "metatask",
    }
}

pub fn list_tasks(cfg: &ProjectConfig) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = cfg
        .tasks
        .iter()
        .map(|(name, def)| {
            let detail = match def {
                TaskDef::Steps { steps, matrix } => {
                    if matrix.is_empty() {
                        format!("{} step(s)", steps.len())
                    } else {
                        format!("{} step(s) x {} target(s)", steps.len(), matrix.len())
                    }
                }
                TaskDef::Group { tasks, strategy, on_failure } => {
                    let s = match strategy {
                        TaskStrategy::Serial => "serial",
                        TaskStrategy::Parallel => "parallel",
                    };
                    let f = match on_failure {
                        TaskOnFailure::Stop => "stop",
                        TaskOnFailure::Continue => "continue",
                    };
                    format!("{} task(s), {s}, on-failure {f}", tasks.len())
                }
            };
            (name.clone(), format!("{} ({detail})", task_kind(def)))
        })
        .collect();
    out.sort();
    out
}

fn unknown_task(cfg: &ProjectConfig, name: &str) -> Diagnostic {
    let mut names: Vec<&str> = cfg.tasks.keys().map(String::as_str).collect();
    names.sort();
    let hint = if names.is_empty() {
        "declare `tasks: { <name>: [...] }` in Project.config".to_string()
    } else {
        format!("available: {}", names.join(", "))
    };
    Diagnostic::new(Code::E108, format!("unknown task `{name}`")).with_hint(hint)
}

fn run_shell(root: &Path, task: &str, cmd: &str, target: &str, prefer_env: Option<&str>) -> Result<(), Diagnostic> {
    let mut command = std::process::Command::new(shell_program());
    command.arg(shell_flag()).arg(cmd);
    command.current_dir(root);
    command.env("RNX_TASK", task);
    command.env("RNX_TARGET", target);
    if let Some(prefer) = prefer_env {
        command.env("RNX_PREFER_NATIVE", prefer);
    }
    let status = command.status().map_err(|e| {
        Diagnostic::new(Code::E108, format!("task `{task}`: cannot run `{cmd}`: {e}"))
    })?;
    if status.success() {
        return Ok(());
    }
    Err(Diagnostic::new(
        Code::E108,
        format!("task `{task}`: step `{cmd}` failed with {}", status_text(&status)),
    ))
}

fn shell_program() -> &'static str {
    #[cfg(windows)]
    {
        "cmd"
    }
    #[cfg(not(windows))]
    {
        "sh"
    }
}

fn shell_flag() -> &'static str {
    #[cfg(windows)]
    {
        "/C"
    }
    #[cfg(not(windows))]
    {
        "-c"
    }
}

fn status_text(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "a signal".to_string(),
    }
}

fn run_steps(
    root: &Path,
    cfg: &ProjectConfig,
    text: &str,
    name: &str,
    steps: &[TaskStep],
    matrix: &[String],
    stack: &mut Vec<String>,
    prefer_env: Option<&str>,
) -> Result<(), Diagnostic> {
    if matrix.is_empty() {
        let target = frontend::native::host_triple();
        return run_step_list(root, cfg, text, name, steps, &target, stack, prefer_env);
    }
    for triple in matrix {
        let target = frontend::native::triple_target(triple).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("task `{name}`: bad matrix triple `{triple}`"))
        })?;
        let defs = frontend::project::parse_tasks_for_target(text, target.clone()).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `{name}`: cannot evaluate matrix leg `{triple}`: {e}"))
        })?;
        let leg = defs.get(name).ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("task `{name}`: matrix leg `{triple}` drops the task (keep every leg total)"))
        })?;
        match leg {
            TaskDef::Steps { steps, .. } => {
                run_step_list(root, cfg, text, name, steps, triple, stack, prefer_env)?
            }
            TaskDef::Group { .. } => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("task `{name}`: matrix leg `{triple}` must stay a step list"),
                ));
            }
        }
    }
    Ok(())
}

fn run_step_list(
    root: &Path,
    cfg: &ProjectConfig,
    text: &str,
    name: &str,
    steps: &[TaskStep],
    target: &str,
    stack: &mut Vec<String>,
    prefer_env: Option<&str>,
) -> Result<(), Diagnostic> {
    for step in steps {
        match step {
            TaskStep::Run(cmd) => run_shell(root, name, cmd, target, prefer_env)?,
            TaskStep::Invoke(task) => run_named(root, cfg, text, task, stack, prefer_env)?,
        }
    }
    Ok(())
}

fn run_named(
    root: &Path,
    cfg: &ProjectConfig,
    text: &str,
    name: &str,
    stack: &mut Vec<String>,
    prefer_env: Option<&str>,
) -> Result<(), Diagnostic> {
    if stack.contains(&name.to_string()) {
        let mut chain = stack.clone();
        chain.push(name.to_string());
        return Err(Diagnostic::new(
            Code::E108,
            format!("task cycle: {}", chain.join(" -> ")),
        ));
    }
    if name == BINDGEN_TASK {
        return run_bindgen_driver(root, cfg);
    }
    let def = cfg.tasks.get(name).ok_or_else(|| unknown_task(cfg, name))?;
    stack.push(name.to_string());
    let result = run_def(root, cfg, text, name, def, stack, prefer_env);
    stack.pop();
    result
}

fn run_group(
    root: &Path,
    cfg: &ProjectConfig,
    text: &str,
    name: &str,
    tasks: &[String],
    strategy: TaskStrategy,
    on_failure: TaskOnFailure,
    stack: &mut Vec<String>,
    prefer_env: Option<&str>,
) -> Result<(), Diagnostic> {
    match strategy {
        TaskStrategy::Serial => {
            let mut failures: Vec<String> = Vec::new();
            for task in tasks {
                match run_named(root, cfg, text, task, stack, prefer_env) {
                    Ok(()) => {}
                    Err(e) => {
                        if matches!(on_failure, TaskOnFailure::Stop) {
                            return Err(e);
                        }
                        failures.push(format!("{task}: {}", e.message));
                    }
                }
            }
            if failures.is_empty() {
                Ok(())
            } else {
                Err(Diagnostic::new(
                    Code::E108,
                    format!("task `{name}`: {} subtask(s) failed:\n{}", failures.len(), failures.join("\n")),
                ))
            }
        }
        TaskStrategy::Parallel => {
            let outcomes: Vec<(String, Result<(), String>)> = std::thread::scope(|s| {
                let mut handles = Vec::with_capacity(tasks.len());
                for task in tasks {
                    let mut stack = stack.clone();
                    stack.push(name.to_string());
                    handles.push((task.clone(), s.spawn(move || {
                        run_named(root, cfg, text, task, &mut stack, prefer_env)
                            .map_err(|e| e.message.clone())
                    })));
                }
                handles
                    .into_iter()
                    .map(|(task, h)| {
                        let outcome = h.join().unwrap_or_else(|_| Err("task thread panicked".to_string()));
                        (task, outcome)
                    })
                    .collect()
            });
            let failures: Vec<String> = outcomes
                .into_iter()
                .filter_map(|(task, r)| r.err().map(|m| format!("{task}: {m}")))
                .collect();
            if failures.is_empty() {
                Ok(())
            } else {
                Err(Diagnostic::new(
                    Code::E108,
                    format!("task `{name}`: {} subtask(s) failed:\n{}", failures.len(), failures.join("\n")),
                ))
            }
        }
    }
}

fn run_def(
    root: &Path,
    cfg: &ProjectConfig,
    text: &str,
    name: &str,
    def: &TaskDef,
    stack: &mut Vec<String>,
    prefer_env: Option<&str>,
) -> Result<(), Diagnostic> {
    match def {
        TaskDef::Steps { steps, matrix } => run_steps(root, cfg, text, name, steps, matrix, stack, prefer_env),
        TaskDef::Group { tasks, strategy, on_failure } => {
            run_group(root, cfg, text, name, tasks, *strategy, *on_failure, stack, prefer_env)
        }
    }
}

pub fn run_task_text(
    root: &Path,
    text: &str,
    cfg: &ProjectConfig,
    name: &str,
    prefer: frontend::native::NativePrefer,
) -> Result<(), Diagnostic> {
    let prefer_env = match prefer {
        frontend::native::NativePrefer::System => Some("system"),
        frontend::native::NativePrefer::Bundled => Some("bundled"),
        frontend::native::NativePrefer::None => None,
    };
    run_named(root, cfg, text, name, &mut Vec::new(), prefer_env)
}

fn driver_options(
    root: &Path,
    pkg_name: &str,
    driver: &BindgenDriver,
    dir: &str,
    title: &str,
    target: &str,
) -> Result<frontend::bindgen::BindgenOptions, Diagnostic> {
    let native_lib = driver.native_lib.clone().ok_or_else(|| {
        Diagnostic::new(
            Code::E108,
            "task `bindgen`: the driver is missing `native_lib` (declare `bindgen: { ..., native_lib: \"foo\", ... }`)",
        )
    })?;
    let mut headers = Vec::with_capacity(driver.headers.len());
    for h in &driver.headers {
        let path = root.join(h);
        if !path.is_file() {
            return Err(Diagnostic::new(
                Code::E108,
                format!("task `bindgen`: header `{h}` does not exist (output `{dir}/{title}`)"),
            ));
        }
        headers.push(path);
    }
    Ok(frontend::bindgen::BindgenOptions {
        headers,
        include_dirs: driver.include.iter().map(|i| root.join(i)).collect(),
        native_lib,
        pkg_name: pkg_name.to_string(),
        target: target.to_string(),
        prefixes: driver.prefix.clone(),
        skips: driver.skip.clone(),
        case: match &driver.case {
            Some(s) => frontend::bindgen::CaseMode::parse(s).map_err(|e| {
                Diagnostic::new(Code::E108, format!("task `bindgen`: bad `case`: {e}"))
            })?,
            None => frontend::bindgen::CaseMode::Keep,
        },
        renames: {
            let mut out = Vec::with_capacity(driver.rename.len());
            for r in &driver.rename {
                out.push(frontend::bindgen::parse_rename(r).map_err(|e| {
                    Diagnostic::new(Code::E108, format!("task `bindgen`: bad `rename`: {e}"))
                })?);
            }
            out
        },
        upstream_name: driver.upstream.clone(),
        title: Some(title.to_string()),
    })
}

pub fn run_bindgen_driver(root: &Path, cfg: &ProjectConfig) -> Result<(), Diagnostic> {
    let driver = cfg.bindgen.as_ref().ok_or_else(|| {
        Diagnostic::new(
            Code::E108,
            "no `bindgen` driver table: declare `bindgen: { headers: [...], outputs: { <dir>: { title: ... } } }` or run `rnx task <name>`",
        )
    })?;
    for (dir, decl) in &driver.outputs {
        let target = decl
            .target
            .clone()
            .unwrap_or_else(frontend::native::host_triple);
        let opts = driver_options(root, &cfg.name, driver, dir, &decl.title, &target)?;
        let output = frontend::bindgen::generate(&opts).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: `{dir}/{}`: {e}", decl.title))
        })?;
        for warning in &output.warnings {
            eprintln!("warning: {warning}");
        }
        let out_dir = root.join(dir);
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: cannot create `{}`: {e}", out_dir.display()))
        })?;
        let rnx_path = out_dir.join(&decl.title);
        std::fs::write(&rnx_path, &output.rnx).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: cannot write `{}`: {e}", rnx_path.display()))
        })?;
        let oci_path = out_dir.join("bindgen.oci.json");
        std::fs::write(&oci_path, &output.oci_json).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: cannot write `{}`: {e}", oci_path.display()))
        })?;
        eprintln!(
            "bindgen: wrote {} ({} functions, {} skipped) for {target}",
            rnx_path.display(),
            output.emitted,
            output.skipped.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn project_with(tag: &str, body: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("task-{tag}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Project.config"), body).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        dir
    }

    fn config_of(dir: &std::path::Path) -> (String, ProjectConfig) {
        let text = std::fs::read_to_string(dir.join("Project.config")).unwrap();
        let manifest = frontend::project::load_manifest(dir).unwrap().unwrap();
        (text, manifest.project.unwrap())
    }

    fn run(dir: &std::path::Path, name: &str) -> Result<(), Diagnostic> {
        let (text, cfg) = config_of(dir);
        run_task_text(dir, &text, &cfg, name, frontend::native::NativePrefer::None)
    }

    #[test]
    fn unknown_task_names_available_tasks() {
        let dir = project_with(
            "unknown",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { check: [\"true\"] }\n}\n",
        );
        let err = run(&dir, "missing").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("unknown task"), "{}", err.message);
    }

    #[test]
    #[cfg(unix)]
    fn serial_group_stops_on_first_failure() {
        let dir = project_with(
            "serial",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        first: [\"touch first.done\"],\n        bad: [\"exit 3\"],\n        last: [\"touch last.done\"],\n        all: { tasks: [\"first\", \"bad\", \"last\"] }\n    }\n}\n",
        );
        let err = run(&dir, "all").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("exit code 3"), "{}", err.message);
        assert!(dir.join("first.done").is_file());
        assert!(!dir.join("last.done").exists());
    }

    #[test]
    #[cfg(unix)]
    fn group_continue_runs_everything_and_reports() {
        let dir = project_with(
            "continue",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        bad: [\"exit 3\"],\n        last: [\"touch last.done\"],\n        all: { tasks: [\"bad\", \"last\"], \"on-failure\": \"continue\" }\n    }\n}\n",
        );
        let err = run(&dir, "all").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("bad"), "{}", err.message);
        assert!(dir.join("last.done").is_file());
    }

    #[test]
    #[cfg(unix)]
    fn parallel_group_runs_all_branches() {
        let dir = project_with(
            "parallel",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        a: [\"touch a.done\"],\n        b: [\"touch b.done\"],\n        all: { tasks: [\"a\", \"b\"], strategy: \"parallel\" }\n    }\n}\n",
        );
        run(&dir, "all").unwrap();
        assert!(dir.join("a.done").is_file());
        assert!(dir.join("b.done").is_file());
    }

    #[test]
    fn cycles_fail_closed() {
        let dir = project_with(
            "cycle",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        a: [{ task: \"b\" }],\n        b: [{ task: \"a\" }]\n    }\n}\n",
        );
        let err = run(&dir, "a").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("cycle"), "{}", err.message);
    }

    #[test]
    #[cfg(unix)]
    fn bindgen_driver_generates_per_output_titles() {
        let dir = project_with(
            "driver",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: {\n        headers: [\"include/add.h\"],\n        prefix: [\"demo_\"],\n        native_lib: \"demo\",\n        outputs: { \"src/gen\": { title: \"demo.rnx\" } }\n    }\n}\n",
        );
        std::fs::create_dir_all(dir.join("include")).unwrap();
        std::fs::write(dir.join("include").join("add.h"), "int demo_add(int a, int b);\n").unwrap();
        run(&dir, "bindgen").unwrap();
        let generated = std::fs::read_to_string(dir.join("src").join("gen").join("demo.rnx")).unwrap();
        assert!(generated.contains("from native \"demo\""), "{generated}");
        let oci = std::fs::read_to_string(dir.join("src").join("gen").join("bindgen.oci.json")).unwrap();
        assert!(oci.contains("demo.rnx"), "{oci}");
        assert!(oci.contains("run.rasmalai.role"), "{oci}");
    }

    #[test]
    #[cfg(unix)]
    fn bindgen_driver_applies_case_and_rename() {
        let dir = project_with(
            "driver-case",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: {\n        headers: [\"include/add.h\"],\n        prefix: [\"demo_\"],\n        case: \"camel\",\n        rename: [\"demo_add=addNums\"],\n        native_lib: \"demo\",\n        outputs: { \"src/gen\": { title: \"demo.rnx\" } }\n    }\n}\n",
        );
        std::fs::create_dir_all(dir.join("include")).unwrap();
        std::fs::write(
            dir.join("include").join("add.h"),
            "int demo_add(int first_value, int second_value);\nint demo_sub(int x, int y);\n",
        )
        .unwrap();
        run(&dir, "bindgen").unwrap();
        let generated = std::fs::read_to_string(dir.join("src").join("gen").join("demo.rnx")).unwrap();
        assert!(generated.contains("unsafe fn addNums(firstValue: Int, secondValue: Int)"), "{generated}");
        assert!(
            generated.contains("fn demo_add(firstValue: Int, secondValue: Int): Int as __rnx_demo_add"),
            "{generated}"
        );
        assert!(generated.contains("unsafe fn demoSub(x: Int, y: Int)"), "{generated}");
    }

    #[test]
    #[cfg(unix)]
    fn matrix_evaluates_steps_per_leg() {
        let dir = project_with(
            "matrix",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        build: {\n            steps: [{ run: target.os == \"linux\" ? \"touch linux.done\" : \"touch other.done\" }],\n            matrix: [\"x86_64-unknown-linux-gnu\", \"aarch64-apple-darwin\"]\n        }\n    }\n}\n",
        );
        run(&dir, "build").unwrap();
        assert!(dir.join("linux.done").is_file());
        assert!(dir.join("other.done").is_file());
    }

    #[test]
    fn bindgen_task_needs_driver_table() {
        let dir = project_with(
            "nodriver",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { check: [\"true\"] }\n}\n",
        );
        let err = run(&dir, "bindgen").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("driver"), "{}", err.message);
    }

    #[test]
    fn listing_names_kinds() {
        let dir = project_with(
            "list",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        check: [\"true\"],\n        all: { tasks: [\"check\"], strategy: \"parallel\" }\n    }\n}\n",
        );
        let (_, cfg) = config_of(&dir);
        let listed = list_tasks(&cfg);
        assert_eq!(listed.len(), 2);
        assert!(listed[0].1.starts_with("metatask"));
        assert!(listed[1].1.starts_with("steps"));
    }
}
