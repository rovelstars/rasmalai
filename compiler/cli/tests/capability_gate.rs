#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;
use std::process::{Command, Output};

fn rnx() -> &'static str {
    env!("CARGO_BIN_EXE_rnx")
}

fn write(path: &Path, text: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, text).unwrap();
}

fn mini_prelude(root: &Path) {
    write(
        &root.join("deps/prelude/Project.config"),
        "export default {\n    project: { name: \"@mini/prelude\", version: \"0.2.1\" },\n    entries: { main: \"src/main.rnx\" }\n}\n",
    );
    let mut names = String::new();
    for name in [
        "Int", "Float", "FastFloat", "Bool", "String", "Char", "Array", "Map", "Set", "Date",
        "Error", "Any", "Void", "GenRef", "Result", "Iterator",
    ] {
        names.push_str(&format!("pub class {name} {{}}\n"));
    }
    write(&root.join("deps/prelude/src/main.rnx"), &names);
}

fn dep(root: &Path, name: &str, capability: &str, body: &str) {
    write(
        &root.join(format!("deps/{name}/Project.config")),
        &format!(
            "export default {{\n    project: {{ name: \"{name}\", version: \"0.1.0\" }},\n    capabilities: [\"{capability}\"],\n    entries: {{ main: \"src/main.rnx\" }}\n}}\n"
        ),
    );
    write(&root.join(format!("deps/{name}/src/main.rnx")), body);
}

fn app(root: &Path, deps: &str, main: &str) {
    write(
        &root.join("Project.config"),
        &format!(
            "export default {{\n    project: {{\n        name: \"capapp\",\n        version: \"0.1.0\",\n        targets: [\"x86_64-unknown-uefi\"]\n    }},\n    dependencies: {{\n        \"@std/prelude\": {{ alias: \"@mini/prelude\" }},\n        \"@mini/prelude\": {{ path: \"deps/prelude\" }},\n{deps}    }}\n}}\n"
        ),
    );
    write(&root.join("src/main.rnx"), main);
}

fn check(root: &Path) -> Output {
    Command::new(rnx())
        .arg("check")
        .arg("src/main.rnx")
        .current_dir(root)
        .output()
        .unwrap()
}

#[test]
fn fs_package_is_rejected_on_uefi() {
    let root = crate::scratch::dir("cap-fs");
    mini_prelude(&root);
    dep(&root, "fsdep", "fs", "export fn touch(): Int { return 1; }\n");
    app(
        &root,
        "        fsdep: { path: \"deps/fsdep\" },\n",
        "import { touch } from \"fsdep\";\nfn Main(): Int { return touch(); }\n",
    );
    let out = check(&root);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("fsdep"), "{text}");
    assert!(text.contains("`fs`"), "{text}");
    assert!(text.contains("x86_64-unknown-uefi"), "{text}");
}

#[test]
fn console_package_imports_on_uefi() {
    let root = crate::scratch::dir("cap-console");
    mini_prelude(&root);
    dep(&root, "condep", "console", "export fn ping(): Int { return 2; }\n");
    app(
        &root,
        "        condep: { path: \"deps/condep\" },\n",
        "import { ping } from \"condep\";\nfn Main(): Int { return ping(); }\n",
    );
    let out = check(&root);
    assert!(
        out.status.success(),
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
}

#[test]
fn unknown_capability_name_is_e108() {
    let root = crate::scratch::dir("cap-unknown");
    write(
        &root.join("Project.config"),
        "export default {\n    project: { name: \"capapp\", version: \"0.1.0\" },\n    capabilities: [\"telepathy\"]\n}\n",
    );
    write(&root.join("src/main.rnx"), "fn Main(): Int { return 0; }\n");
    let out = check(&root);
    assert!(!out.status.success());
    let text = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("E108"), "{text}");
    assert!(text.contains("telepathy"), "{text}");
}
