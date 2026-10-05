const EXPECTED: &str = "Multi-package resolution success: 42\n";

fn app_dir() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("tests/fixtures/multi_package/app")
}

#[test]
fn package_bare_run_interpreter() {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("--backend")
        .arg("interpreter")
        .current_dir(app_dir())
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
}

#[test]
fn package_bare_build_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-pkg-build-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .current_dir(app_dir())
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), EXPECTED);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn package_missing_dependency_is_e108() {
    let dir = std::env::temp_dir().join(format!("rnx-pkg-missing-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"lonely\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let main = src.join("main.rnx");
    std::fs::write(&main, "import { x } from \"ghost\";\nfn Main(): Int { return x(); }\n").unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("check")
        .arg(&main)
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text =
        String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("unknown package dependency `ghost`"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
