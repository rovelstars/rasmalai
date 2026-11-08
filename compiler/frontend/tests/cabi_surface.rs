use std::path::Path;

fn project(root: &Path, cabi: &str, main_src: &str, cabi_src: &str) {
    std::fs::create_dir_all(root.join("src")).unwrap();
    let manifest = format!(
        "export default {{\n    project: {{ name: \"cabi-test\", version: \"0.1.0\" }},\n    entries: {{ main: \"src/main.rnx\", cabi: \"{cabi}\" }},\n}}\n"
    );
    std::fs::write(root.join("Project.config"), manifest).unwrap();
    std::fs::write(root.join("src/main.rnx"), main_src).unwrap();
    if !cabi_src.is_empty() {
        std::fs::write(root.join(cabi), cabi_src).unwrap();
    }
}

fn codes(root: &Path) -> Vec<String> {
    match frontend::check::check_package(&root.join("src/main.rnx")) {
        Ok(_) => Vec::new(),
        Err(diags) => diags.iter().map(|d| d.message.clone()).collect(),
    }
}

#[test]
fn cabi_surface_accepts_c_representable_exports() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "src/cabi.rnx",
        "fn Main(): Int {\n    return 0;\n}\n",
        "export fn rnx_add(a: Int, b: Int): Int {\n    return a + b;\n}\n",
    );
    let messages = codes(dir.path());
    assert!(messages.is_empty(), "{messages:?}");
}

#[test]
fn cabi_surface_without_exports_is_rejected() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "src/cabi.rnx",
        "fn Main(): Int {\n    return 0;\n}\n",
        "fn helper(): Int {\n    return 1;\n}\n",
    );
    let messages = codes(dir.path());
    assert!(
        messages.iter().any(|m| m.contains("declares no `export fn`")),
        "{messages:?}"
    );
}

#[test]
fn cabi_surface_rejects_re_exports() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "src/cabi.rnx",
        "fn Main(): Int {\n    return 0;\n}\n",
        "export { rnx_add } from \"./impl\"\n",
    );
    std::fs::write(
        dir.path().join("src/impl.rnx"),
        "export fn rnx_add(a: Int, b: Int): Int {\n    return a + b;\n}\n",
    );
    let messages = codes(dir.path());
    assert!(
        messages.iter().any(|m| m.contains("re-export writes no C symbol")),
        "{messages:?}"
    );
}

#[test]
fn cabi_surface_rejects_rich_signatures() {
    let dir = tempfile::tempdir().unwrap();
    project(
        dir.path(),
        "src/cabi.rnx",
        "fn Main(): Int {\n    return 0;\n}\n",
        "export class Handle {\n    let n: Int = 0\n}\nexport fn make(): Handle {\n    return new Handle()\n}\n",
    );
    let messages = codes(dir.path());
    assert!(
        messages.iter().any(|m| m.contains("non-C-ABI return type")),
        "{messages:?}"
    );
}

#[test]
fn missing_cabi_path_is_a_manifest_error() {
    let dir = tempfile::tempdir().unwrap();
    std::fs::write(
        dir.path().join("Project.config"),
        "export default {\n    project: { name: \"cabi-test\", version: \"0.1.0\" },\n    entries: { main: \"src/main.rnx\", cabi: \"src/missing.rnx\" },\n}\n",
    )
    .unwrap();
    let text = std::fs::read_to_string(dir.path().join("Project.config")).unwrap();
    let issues = frontend::project::validate_manifest_text(&text, Some(dir.path()));
    assert!(
        issues.iter().any(|i| i.code == diagnostics::Code::E108
            && i.message.contains("entries.cabi path")),
        "{issues:?}"
    );
}
