const EXPECTED: &str = "Workspace run success: 42\n";

fn workspace_tree(tag: &str) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("rnx-ws-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let src =
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/workspace_monorepo");
    copy_dir(&src, &base);
    base
}

fn copy_dir(from: &std::path::Path, to: &std::path::Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap() {
        let entry = entry.unwrap();
        let dest = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &dest);
        } else {
            std::fs::copy(entry.path(), &dest).unwrap();
        }
    }
}

#[test]
fn test_workspace_lock() {
    let ws = workspace_tree("lock");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("lock")
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(
        String::from_utf8(out.stdout).unwrap(),
        "Locked 2 packages to Project.deplock\n"
    );
    let text = std::fs::read_to_string(ws.join("Project.deplock")).unwrap();
    assert!(text.contains("name: \"calc\""), "{text}");
    assert!(text.contains("name: \"player\""), "{text}");
    assert!(text.contains("source: \"path:libs/calc\""), "{text}");
    assert!(text.contains("source: \"path:apps/player\""), "{text}");
    assert!(text.contains("dependencies: [\"calc 0.1.0\"]"), "{text}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_workspace_run_package_flag() {
    let ws = workspace_tree("run");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("-p")
        .arg("player")
        .current_dir(&ws)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_workspace_build_package_flag() {
    let ws = workspace_tree("build");
    std::fs::create_dir_all(ws.join("target")).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg("-p")
        .arg("player")
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_workspace_locked_run_from_member_dir() {
    let ws = workspace_tree("locked");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let lock = std::process::Command::new(rnx)
        .arg("lock")
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("--locked")
        .current_dir(ws.join("apps").join("player"))
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_unknown_package_flag_error() {
    let ws = workspace_tree("unknown");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("-p")
        .arg("non_existent")
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(!run.status.success());
    let text =
        String::from_utf8(run.stdout).unwrap() + &String::from_utf8(run.stderr).unwrap();
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("unknown workspace member `non_existent`"), "{text}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_bare_run_from_workspace_root_needs_package_flag() {
    let ws = workspace_tree("bare");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .current_dir(&ws)
        .output()
        .unwrap();
    assert!(!run.status.success());
    let text =
        String::from_utf8(run.stdout).unwrap() + &String::from_utf8(run.stderr).unwrap();
    assert!(text.contains("specify -p <package>"), "{text}");
    let _ = std::fs::remove_dir_all(&ws);
}

#[test]
fn test_clean_package_flag_scopes_to_member() {
    let base = std::env::temp_dir().join(format!("rnx-ws-clean-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(&base).unwrap();
    std::fs::write(
        base.join("Project.config"),
        "export default {\n    workspace: {\n        members: [\"alpha\", \"beta\"]\n    }\n}\n",
    )
    .unwrap();
    for member in ["alpha", "beta"] {
        let dir = base.join(member);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::create_dir_all(dir.join(".rnx-cache")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            format!("export default {{\n    project: {{\n        name: \"{member}\",\n        version: \"0.1.0\"\n    }}\n}}\n"),
        )
        .unwrap();
        std::fs::write(
            dir.join("src").join("main.rnx"),
            "fn Main(): Int {\n    return 0;\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join(".rnx-cache").join("stale.bin"), "stale").unwrap();
    }
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let clean = std::process::Command::new(rnx)
        .arg("clean")
        .arg("-p")
        .arg("alpha")
        .current_dir(&base)
        .output()
        .unwrap();
    assert!(clean.status.success(), "{}", String::from_utf8_lossy(&clean.stderr));
    let stdout = String::from_utf8(clean.stdout).unwrap();
    assert!(stdout.contains("alpha"), "{stdout}");
    assert!(
        !base.join("alpha").join(".rnx-cache").join("stale.bin").exists(),
        "alpha cache must be cleaned"
    );
    assert!(
        base.join("beta").join(".rnx-cache").join("stale.bin").is_file(),
        "beta cache must survive clean -p alpha"
    );
    let _ = std::fs::remove_dir_all(&base);
}
