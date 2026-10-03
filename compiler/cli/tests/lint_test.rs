use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn write_src(tag: &str, name: &str, src: &str) -> (PathBuf, String) {
    let dir = std::env::temp_dir().join(format!("rnx-lint-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let file = dir.join(name);
    std::fs::write(&file, src).unwrap();
    (dir, file.to_string_lossy().into_owned())
}

fn run_lint(args: &[&str]) -> std::process::Output {
    Command::new(rnx())
        .arg("lint")
        .env("NO_COLOR", "1")
        .args(args)
        .output()
        .unwrap()
}

const UNUSED_SRC: &str = "fn Main(): Int {\n    let x = 10;\n    let _y = 20;\n    return 0;\n}\n";

#[test]
fn test_lint_unused_variable_and_ignore_prefix() {
    let (dir, name) = write_src("unused", "main.rnx", UNUSED_SRC);
    let out = run_lint(&[&name]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("L001"), "{stderr}");
    assert!(stderr.contains("`x`"), "{stderr}");
    assert!(!stderr.contains("`_y`"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

const DEAD_SRC: &str = "fn demo(): Int {\n    return 42;\n    let dead = 1;\n}\n";

#[test]
fn test_lint_unreachable_code() {
    let (dir, name) = write_src("dead", "main.rnx", DEAD_SRC);
    let out = run_lint(&[&name]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("L003"), "{stderr}");
    assert!(stderr.contains("let dead = 1;"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

const NODOC_SRC: &str = "pub fn exportMe(): Void {}\n";
const DOC_SRC: &str = "/** Documentation. */\npub fn exportMe(): Void {}\n";

#[test]
fn test_lint_missing_docstring_on_pub_item() {
    let (dir, name) = write_src("nodoc", "main.rnx", NODOC_SRC);
    let out = run_lint(&[&name]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("L004"), "{stderr}");
    assert!(stderr.contains("exportMe"), "{stderr}");
    std::fs::write(dir.join("main.rnx"), DOC_SRC).unwrap();
    let fixed = run_lint(&[&name]);
    assert_eq!(fixed.status.code(), Some(0), "{}", String::from_utf8_lossy(&fixed.stderr));
    let restderr = String::from_utf8_lossy(&fixed.stderr).into_owned();
    assert!(!restderr.contains("L004"), "{restderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_lint_sarif_schema_compliance() {
    let (dir, name) = write_src("sarif", "main.rnx", UNUSED_SRC);
    let out = Command::new(rnx())
        .arg("lint")
        .arg("--sarif")
        .arg(&name)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let value: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid SARIF {e}: {stdout}"));
    assert!(value["$schema"].as_str().unwrap_or("").contains("sarif"), "{stdout}");
    assert_eq!(value["version"], serde_json::Value::String("2.1.0".to_string()), "{stdout}");
    let runs = value["runs"].as_array().unwrap_or_else(|| panic!("no runs: {stdout}"));
    assert!(!runs.is_empty(), "{stdout}");
    let results = runs[0]["results"].as_array().unwrap_or_else(|| panic!("no results: {stdout}"));
    assert!(!results.is_empty(), "{stdout}");
    let location = &results[0]["locations"][0]["physicalLocation"];
    assert!(location["artifactLocation"]["uri"].as_str().unwrap_or("").ends_with("main.rnx"), "{stdout}");
    assert!(location["region"]["startLine"].as_u64().unwrap_or(0) >= 1, "{stdout}");
    assert!(results[0]["ruleId"].as_str().unwrap_or("").starts_with('L'), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_lint_deny_warnings_flag() {
    let (dir, name) = write_src("deny", "main.rnx", UNUSED_SRC);
    let lenient = run_lint(&[&name]);
    assert_eq!(lenient.status.code(), Some(0), "{}", String::from_utf8_lossy(&lenient.stderr));
    let strict = run_lint(&["--deny-warnings", &name]);
    assert_eq!(strict.status.code(), Some(1), "{}", String::from_utf8_lossy(&strict.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

const SHADOW_SRC: &str = "fn Main(): Int {\n    let rock = 1;\n    print(rock);\n    while true {\n        let rock = 2;\n        print(rock);\n    }\n    return rock;\n}\n";

const SHADOW_UNUSED_SRC: &str = "fn Main(): Int {\n    let rock = 1;\n    while true {\n        let rock = 2;\n        print(rock);\n    }\n    return 0;\n}\n";

#[test]
fn test_lint_shadowed_bindings_resolve_innermost() {
    let (dir, name) = write_src("shadow", "main.rnx", SHADOW_SRC);
    let out = run_lint(&[&name]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!stderr.contains("L001"), "false unused warning: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);

    let (dir2, name2) = write_src("shadow-unused", "main.rnx", SHADOW_UNUSED_SRC);
    let out2 = run_lint(&[&name2]);
    assert_eq!(out2.status.code(), Some(0), "{}", String::from_utf8_lossy(&out2.stderr));
    let stderr2 = String::from_utf8_lossy(&out2.stderr).into_owned();
    assert!(stderr2.contains("L001"), "missing unused warning: {stderr2}");
    assert!(stderr2.contains("2:5"), "warning on wrong binding: {stderr2}");
    let _ = std::fs::remove_dir_all(&dir2);
}

const UNPARSEABLE_SRC: &str = "fn broken(): Int => 1;\n";

#[test]
fn test_lint_reports_unparseable_file() {
    let (dir, name) = write_src("unparseable", "main.rnx", UNPARSEABLE_SRC);
    let out = run_lint(&[&name]);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!stderr.contains("No lint issues found"), "{stderr}");
    assert!(stderr.contains("E105"), "{stderr}");
    assert!(stderr.contains("main.rnx"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

const RESHADOW_SRC: &str = "fn Main(): Int {\n    let k = 0;\n    while k < 2 {\n        let t0 = 1;\n        let m1 = t0 + 1;\n        let t0 = 2;\n        let m2 = t0 + 1;\n        print(m1 + m2);\n        k = k + 1;\n    }\n    return 0;\n}\n";

#[test]
fn test_lint_same_block_redeclare_read_after_is_used() {
    let (dir, name) = write_src("redeclare", "main.rnx", RESHADOW_SRC);
    let out = run_lint(&[&name]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(!stderr.contains("L001"), "false unused warning: {stderr}");
    let before = std::fs::read_to_string(&name).unwrap();
    let fixed = Command::new(rnx())
        .arg("lint")
        .arg("--fix")
        .env("NO_COLOR", "1")
        .arg(&name)
        .output()
        .unwrap();
    assert_eq!(fixed.status.code(), Some(0), "{}", String::from_utf8_lossy(&fixed.stderr));
    let after = std::fs::read_to_string(&name).unwrap();
    assert_eq!(before, after, "--fix must not rename a read shadowed variable");
    let _ = std::fs::remove_dir_all(&dir);
}

const RESHADOW_UNUSED_SRC: &str = "fn Main(): Int {\n    let t0 = 1;\n    print(t0);\n    let t0 = 2;\n    return 0;\n}\n";

#[test]
fn test_lint_shadowed_unused_withholds_autofix() {
    let (dir, name) = write_src("redeclare-unused", "main.rnx", RESHADOW_UNUSED_SRC);
    let out = run_lint(&[&name]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("L001"), "missing unused warning: {stderr}");
    assert!(stderr.contains("(0 auto-fixable)"), "shadowed finding must not be auto-fixable: {stderr}");
    let before = std::fs::read_to_string(&name).unwrap();
    let fixed = Command::new(rnx())
        .arg("lint")
        .arg("--fix")
        .env("NO_COLOR", "1")
        .arg(&name)
        .output()
        .unwrap();
    assert_eq!(fixed.status.code(), Some(0), "{}", String::from_utf8_lossy(&fixed.stderr));
    let after = std::fs::read_to_string(&name).unwrap();
    assert_eq!(before, after, "--fix must leave shadowed bindings alone");
    let _ = std::fs::remove_dir_all(&dir);
}
