use std::path::{Path, PathBuf};
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-wavea-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_package(dir: &Path, name: &str, src: &str) {
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n"),
    )
    .unwrap();
    let src_dir = dir.join("src");
    std::fs::create_dir_all(&src_dir).unwrap();
    std::fs::write(src_dir.join("main.rnx"), src).unwrap();
}

fn run_check(dir: &Path) -> std::process::Output {
    Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .arg("src/main.rnx")
        .output()
        .unwrap()
}

#[test]
fn deep_parens_report_e108_without_abort() {
    let dir = fresh_dir("parens");
    let src = format!(
        "fn Main(): Int {{\n    let x = {}1{};\n    return x;\n}}\n",
        "(".repeat(250),
        ")".repeat(250)
    );
    write_package(&dir, "deepparens", &src);
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1), "expected exit 1, not a signal abort");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E108"), "expected E108, got: {stderr}");
    assert!(stderr.contains("too deep"), "expected depth message, got: {stderr}");
}

#[test]
fn deep_block_nesting_reports_e108_without_abort() {
    let dir = fresh_dir("blocks");
    let mut src = String::from("fn Main(): Int {\n");
    for _ in 0..200 {
        src.push_str("    if true {\n");
    }
    src.push_str("    return 1;\n");
    for _ in 0..200 {
        src.push_str("    }\n");
    }
    src.push_str("    return 0;\n}\n");
    write_package(&dir, "deepblocks", &src);
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1), "expected exit 1, not a signal abort");
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E108"), "expected E108, got: {stderr}");
}

#[test]
fn unknown_method_on_class_reports_e108() {
    let dir = fresh_dir("method");
    write_package(
        &dir,
        "badmethod",
        "class Point { let x: Int; init(x: Int) { this.x = x } fn get(): Int { return this.x } }\n\nfn Main(): Int {\n    let p = new Point(1);\n    p.nonExistent();\n    return 0;\n}\n",
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E108"), "expected E108, got: {stderr}");
    assert!(stderr.contains("nonExistent"), "expected method name, got: {stderr}");
    assert!(stderr.contains("Point"), "expected receiver type, got: {stderr}");
}

#[test]
fn known_method_on_class_passes_check() {
    let dir = fresh_dir("goodmethod");
    write_package(
        &dir,
        "goodmethod",
        "class Point { let x: Int; init(x: Int) { this.x = x } fn get(): Int { return this.x } }\n\nfn Main(): Int {\n    let p = new Point(1);\n    return p.get();\n}\n",
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn inherited_method_on_subclass_passes_check() {
    let dir = fresh_dir("inherit");
    write_package(
        &dir,
        "inherit",
        "class Base { let x: Int; init(x: Int) { this.x = x } fn get(): Int { return this.x } }\n\nclass Child extends Base { init(x: Int) { super(x); } fn extra(): Int { return 1 } }\n\nfn Main(): Int {\n    let c = new Child(3);\n    return c.get() + c.extra();\n}\n",
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn unknown_method_on_subclass_still_reports_e108() {
    let dir = fresh_dir("subunknown");
    write_package(
        &dir,
        "subunknown",
        "class Base { let x: Int; init(x: Int) { this.x = x } fn get(): Int { return this.x } }\n\nclass Child extends Base {}\n\nfn Main(): Int {\n    let c = new Child(3);\n    c.nope();\n    return 0;\n}\n",
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("unknown method `nope` on `Child`"), "got: {stderr}");
}

#[test]
fn wrong_arity_on_class_method_reports_e108() {
    let dir = fresh_dir("arity");
    write_package(
        &dir,
        "badarity",
        "class Point { let x: Int; init(x: Int) { this.x = x } fn get(): Int { return this.x } }\n\nfn Main(): Int {\n    let p = new Point(1);\n    return p.get(1, 2);\n}\n",
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E108"), "expected E108, got: {stderr}");
}
