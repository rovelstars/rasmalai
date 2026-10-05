use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-check-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_package(dir: &Path, name: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n"),
    )
    .unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (file, contents) in files {
        std::fs::write(src.join(file), contents).unwrap();
    }
}

const MAIN_OK: &str = "import { helper } from \"./util\";\n\nfn Main(): Int {\n    return helper();\n}\n";
const UTIL_OK: &str = "fn helper(): Int {\n    return 41;\n}\n";
const AWAIT_OUTSIDE_ASYNC: &str = "fn Main(): Int {\n    await foo();\n    return 0;\n}\n";

#[test]
fn test_check_valid_package() {
    let dir = fresh_dir("valid");
    write_package(&dir, "checkpkg", &[("main.rnx", MAIN_OK), ("util.rnx", UTIL_OK)]);
    let out = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("Checked"), "{stderr}");
    assert!(stderr.contains("checkpkg"), "{stderr}");
    assert!(stderr.contains("(ok)"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_check_type_error_detection() {
    let dir = fresh_dir("typeerr");
    write_package(&dir, "badpkg", &[("main.rnx", AWAIT_OUTSIDE_ASYNC)]);
    let out = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E109"), "{stderr}");
    assert!(stderr.contains(":1:"), "{stderr}");
    assert!(stderr.contains("fn Main(): Int {"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_check_json_output() {
    let dir = fresh_dir("json");
    write_package(&dir, "jsonpkg", &[("main.rnx", AWAIT_OUTSIDE_ASYNC)]);
    let out = Command::new(rnx())
        .arg("check")
        .arg("--json")
        .current_dir(&dir)
        .arg("src/main.rnx")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    let value: serde_json::Value =
        serde_json::from_str(&stdout).unwrap_or_else(|e| panic!("invalid JSON {e}: {stdout}"));
    let items = value.as_array().unwrap_or_else(|| panic!("expected array: {stdout}"));
    assert_eq!(items.len(), 2, "{stdout}");
    let item = &items[0];
    assert_eq!(item["code"], serde_json::Value::String("E109".to_string()), "{stdout}");
    assert_eq!(item["severity"], serde_json::Value::String("error".to_string()), "{stdout}");
    assert_eq!(item["line"], serde_json::Value::from(1), "{stdout}");
    assert!(item["col"].as_u64().unwrap_or(0) >= 1, "{stdout}");
    assert!(item["message"].as_str().unwrap_or("").contains("await"), "{stdout}");
    let item = &items[1];
    assert_eq!(item["code"], serde_json::Value::String("E303".to_string()), "{stdout}");
    assert_eq!(item["severity"], serde_json::Value::String("error".to_string()), "{stdout}");
    assert!(item["file"].as_str().unwrap_or("").ends_with("main.rnx"), "{stdout}");
    assert!(item["message"].as_str().unwrap_or("").contains("foo"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn has_artifacts(dir: &Path) -> bool {
    let mut stack = vec![dir.to_path_buf()];
    while let Some(current) = stack.pop() {
        let entries = match std::fs::read_dir(&current) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if entry.file_name() == "target" {
                    return true;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "o" || ext == "obj") {
                return true;
            }
        }
    }
    false
}

#[test]
fn test_check_skips_codegen_and_artifacts() {
    let dir = fresh_dir("artifacts");
    write_package(&dir, "cleanpkg", &[("main.rnx", MAIN_OK), ("util.rnx", UTIL_OK)]);
    assert!(!dir.join("target").exists());
    let out = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(!has_artifacts(&dir), "check created build artifacts");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_check_latency_comparison() {
    let dir = fresh_dir("latency");
    write_package(&dir, "speedpkg", &[("main.rnx", MAIN_OK), ("util.rnx", UTIL_OK)]);
    let main = dir.join("src").join("main.rnx");
    let name = main.to_string_lossy().into_owned();
    let check_start = Instant::now();
    let check = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .arg(&name)
        .output()
        .unwrap();
    let check_elapsed = check_start.elapsed();
    assert!(check.status.success(), "{}", String::from_utf8_lossy(&check.stderr));
    let build_start = Instant::now();
    let build = Command::new(rnx())
        .arg("build")
        .arg(&name)
        .output()
        .unwrap();
    let build_elapsed = build_start.elapsed();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    assert!(
        check_elapsed < build_elapsed,
        "check {check_elapsed:?} not faster than build {build_elapsed:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_check_manifest_warnings() {
    let dir = fresh_dir("manifest");
    write_package(&dir, "checkpkg", &[("main.rnx", MAIN_OK), ("util.rnx", UTIL_OK)]);
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"checkpkg\",\n        version: \"0.1\",\n        bogus: 1\n    },\n    zzz: 1\n}\n",
    )
    .unwrap();
    let out = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("W201"), "{stderr}");
    assert!(stderr.contains("project.bogus"), "{stderr}");
    assert!(stderr.contains("unrecognized field `zzz`"), "{stderr}");
    assert!(stderr.contains("0.1"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_check_manifest_file_directly() {
    let dir = fresh_dir("manifest-direct");
    std::fs::write(dir.join("Project.config"), "export default {\n    project: {\n        name: \"d\"\n    }\n}\n").unwrap();
    let out = Command::new(rnx())
        .arg("check")
        .arg(dir.join("Project.config"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("version"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_check_rejects_non_source_files_cleanly() {
    let dir = fresh_dir("not-source");
    let note = dir.join("notes.txt");
    std::fs::write(&note, "hello\n").unwrap();
    let out = Command::new(rnx())
        .arg("check")
        .arg(&note)
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("not a checkable file"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

const MAIN_IMPORTS_BAD: &str = "import { helper } from \"./util\";\n\nfn Main(): Int {\n    return helper();\n}\n";
const UTIL_ARROW: &str = "fn helper(): Int => 41;\n";

#[test]
fn test_check_parse_error_attributes_owning_file() {
    let dir = fresh_dir("attr");
    write_package(&dir, "attrpkg", &[("main.rnx", MAIN_IMPORTS_BAD), ("util.rnx", UTIL_ARROW)]);
    let out = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E105"), "{stderr}");
    assert!(stderr.contains("util.rnx"), "{stderr}");
    assert!(stderr.contains("fn helper(): Int => 41;"), "{stderr}");
    assert!(!stderr.contains("main.rnx:1:"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

const MAIN_TWO_BAD: &str = "import { a } from \"./a\";\nimport { b } from \"./b\";\n\nfn Main(): Int {\n    return a() + b();\n}\n";

#[test]
fn test_check_reports_errors_across_files() {
    let dir = fresh_dir("multi");
    write_package(
        &dir,
        "multipkg",
        &[
            ("main.rnx", MAIN_TWO_BAD),
            ("a.rnx", "fn a(): Int => 1;\n"),
            ("b.rnx", "fn b(): Int => 2;\n"),
        ],
    );
    let out = Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(stderr.matches("E105").count(), 2, "{stderr}");
    assert!(stderr.contains("a.rnx"), "{stderr}");
    assert!(stderr.contains("b.rnx"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
