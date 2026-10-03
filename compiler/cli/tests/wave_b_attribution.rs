use std::path::{Path, PathBuf};
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-waveb-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_project(dir: &Path, main_src: &str, lib_src: &str) {
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"attrpkg\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(src.join("lib")).unwrap();
    std::fs::write(src.join("main.rnx"), main_src).unwrap();
    std::fs::write(src.join("lib").join("util.rnx"), lib_src).unwrap();
}

fn run_check(dir: &Path, json: bool) -> std::process::Output {
    let mut cmd = Command::new(rnx());
    cmd.arg("check").env("NO_COLOR", "1").current_dir(dir).arg("src/main.rnx");
    if json {
        cmd.arg("--json");
    }
    cmd.output().unwrap()
}

const MAIN_SRC: &str = "import { helper } from \"./lib/util.rnx\";\n\nfn Main(): Int {\n    return helper();\n}\n";

#[test]
fn semantic_error_in_import_attributes_to_import() {
    let dir = fresh_dir("sem");
    write_project(
        &dir,
        MAIN_SRC,
        "fn helper(): Int {\n    let x: Int = \"not an int\";\n    return x;\n}\n",
    );
    let out = run_check(&dir, false);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("lib/util.rnx"), "must name the import, got: {stderr}");
    assert!(stderr.contains("not an int"), "must render the import source, got: {stderr}");
    assert!(!stderr.contains("main.rnx"), "must not blame the entry, got: {stderr}");
}

#[test]
fn parse_error_in_import_attributes_to_import() {
    let dir = fresh_dir("parse");
    write_project(
        &dir,
        MAIN_SRC,
        "fn helper(): Int {\n    let x: Int = 42;\n    return (x + ;\n}\n",
    );
    let out = run_check(&dir, false);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("lib/util.rnx"), "must name the import, got: {stderr}");
    assert!(!stderr.contains("main.rnx"), "must not blame the entry, got: {stderr}");
}

#[test]
fn import_error_json_carries_file_line_col() {
    let dir = fresh_dir("json");
    write_project(
        &dir,
        MAIN_SRC,
        "fn helper(): Int {\n    let x: Int = \"not an int\";\n    return x;\n}\n",
    );
    let out = run_check(&dir, true);
    assert_eq!(out.status.code(), Some(1));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    let combined = stdout + &stderr;
    assert!(combined.contains("lib/util.rnx"), "json must name the import, got: {combined}");
    assert!(combined.contains("\"line\":2"), "json must carry the line, got: {combined}");
}

#[test]
fn clean_multifile_project_passes() {
    let dir = fresh_dir("clean");
    write_project(
        &dir,
        MAIN_SRC,
        "fn helper(): Int {\n    let x: Int = 41;\n    return x + 1;\n}\n",
    );
    let out = run_check(&dir, false);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
}
