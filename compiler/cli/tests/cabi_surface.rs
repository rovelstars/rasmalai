#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> crate::scratch::Scratch {
    crate::scratch::dir(&format!("cabi-{tag}"))
}

fn write_package(dir: &Path, name: &str, entries: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }},\n    entries: {{ {entries} }}\n}}\n"
        ),
    )
    .unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (file, contents) in files {
        std::fs::write(src.join(file), contents).unwrap();
    }
}

fn check(dir: &Path) -> std::process::Output {
    Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .output()
        .unwrap()
}

const API_ROW: &str = "export struct Row {\n    let id: Int;\n}\n\nexport fn makeRow(id: Int): Row {\n    return Row(id);\n}\n";
const MAIN_ROW: &str = "import { makeRow } from \"./api\";\n\nfn Main(): Int {\n    let r = makeRow(7);\n    return r.id;\n}\n";
const CAPI_ADD: &str = "export fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n";

#[test]
fn cabi_file_is_the_only_c_surface() {
    let dir = fresh_dir("ok");
    write_package(
        &dir,
        "capipkg",
        "main: \"src/main.rnx\", cabi: \"src/cabi.rnx\"",
        &[
            ("main.rnx", MAIN_ROW),
            ("api.rnx", API_ROW),
            ("cabi.rnx", CAPI_ADD),
        ],
    );
    let out = check(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
}

#[test]
fn cabi_file_rejects_non_representable_export() {
    let dir = fresh_dir("bad");
    let cabi = "import { Row } from \"./api\";\n\nexport fn buildRow(id: Int): Row {\n    return Row(id);\n}\n";
    write_package(
        &dir,
        "capipkg",
        "main: \"src/main.rnx\", cabi: \"src/cabi.rnx\"",
        &[
            ("main.rnx", MAIN_ROW),
            ("api.rnx", API_ROW),
            ("cabi.rnx", cabi),
        ],
    );
    let out = check(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("buildRow"), "{stderr}");
    assert!(stderr.contains("Row"), "{stderr}");
    assert!(stderr.contains("move this declaration out of the cabi file"), "{stderr}");
    assert!(stderr.contains("export struct"), "{stderr}");
}

#[test]
fn no_cabi_entry_keeps_the_graph_wide_gate() {
    let dir = fresh_dir("noentry");
    write_package(
        &dir,
        "plainpkg",
        "main: \"src/main.rnx\"",
        &[("main.rnx", MAIN_ROW), ("api.rnx", API_ROW)],
    );
    let out = check(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(1), "{stderr}");
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("makeRow"), "{stderr}");
    assert!(stderr.contains("Row"), "{stderr}");
}

#[test]
fn exported_types_in_cabi_file_are_package_api() {
    let dir = fresh_dir("types");
    let cabi = "export struct Row {\n    let id: Int;\n}\n\nexport fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n";
    write_package(
        &dir,
        "capipkg",
        "main: \"src/main.rnx\", cabi: \"src/cabi.rnx\"",
        &[("main.rnx", MAIN_ROW), ("api.rnx", API_ROW), ("cabi.rnx", cabi)],
    );
    let out = check(&dir);
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert_eq!(out.status.code(), Some(0), "{stderr}");
}

#[test]
fn manifest_roundtrips_cabi_entry() {
    let text = "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    entries: { main: \"src/main.rnx\", cabi: \"src/cabi.rnx\" }\n}\n";
    let (project, _) = frontend::project::parse_manifest_for_publish(text).unwrap();
    let project = project.unwrap();
    assert_eq!(project.entries.cabi.as_deref(), Some("src/cabi.rnx"));
    let manifest = frontend::project::Manifest {
        project: Some(project),
        workspace: None,
    };
    let out = frontend::project::manifest_to_rnx(&manifest);
    assert!(out.contains("cabi: \"src/cabi.rnx\""), "{out}");
    let (back, _) = frontend::project::parse_manifest_for_publish(&out).unwrap();
    assert_eq!(
        back.unwrap().entries.cabi.as_deref(),
        Some("src/cabi.rnx")
    );
}
