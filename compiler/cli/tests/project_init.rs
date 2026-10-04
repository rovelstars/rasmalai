fn base(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-init-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn scaffold(tag: &str) -> std::path::PathBuf {
    let dir = base(tag);
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let init = std::process::Command::new(rnx)
        .arg("init")
        .arg("sample_project")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));
    dir.join("sample_project")
}

#[test]
fn test_init_scaffold() {
    let dir = base("scaffold");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let init = std::process::Command::new(rnx)
        .arg("init")
        .arg("sample_project")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(init.status.success(), "{}", String::from_utf8_lossy(&init.stderr));
    let root = dir.join("sample_project");
    let config = std::fs::read_to_string(root.join("Project.config")).unwrap();
    assert_eq!(
        config,
        "export default {\n    project: {\n        name: \"sample_project\",\n        version: \"0.1.0\"\n    }\n}\n"
    );
    let main = std::fs::read_to_string(root.join("src").join("main.rnx")).unwrap();
    assert_eq!(
        main,
        "fn Main(): Int {\n    print(\"Hello from sample_project!\");\n    return 0;\n}\n"
    );
    let stdout = String::from_utf8(init.stdout).unwrap();
    assert!(stdout.contains("sample_project"), "{stdout}");
    let again = std::process::Command::new(rnx)
        .arg("init")
        .arg("sample_project")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(!again.status.success());
    let stderr = String::from_utf8(again.stderr).unwrap();
    assert!(stderr.contains("already exists and is not empty"), "{stderr}");
    std::fs::write(
        root.join("Project.config"),
"// sample project manifest\nexport default {\n    project: {\n        name: \"sample_project\",\n        version: \"0.1.0\"\n    },\n    dependencies: {\n        // local helper\n        helper: { path: \"libs/helper\" }\n    }\n}\n",
    )
    .unwrap();
    let run = std::process::Command::new(rnx)
        .arg("run")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Hello from sample_project!\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bare_run_from_root() {
    let root = scaffold("run-root");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Hello from sample_project!\n");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn test_bare_run_from_nested_subdir() {
    let root = scaffold("run-nested");
    let nested = root.join("src").join("nested");
    std::fs::create_dir_all(&nested).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .current_dir(&nested)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Hello from sample_project!\n");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn test_bare_build_native_binary() {
    let root = scaffold("build");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .current_dir(&root)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let exe = root.join(".rnx-cache").join("build").join("dev").join("sample_project");
    assert!(exe.is_file(), "bare build writes .rnx-cache/build/dev/sample_project");
    let run = std::process::Command::new(&exe).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Hello from sample_project!\n");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}
