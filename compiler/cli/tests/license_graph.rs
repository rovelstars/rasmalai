#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn write_pkg(dir: &std::path::Path, name: &str, version: &str, extra: &str, lib_src: &str, license: Option<&str>) {
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"{version}\"\n    }},\n    license: \"MIT\",\n    entries: {{ main: \"src/lib.rnx\" }}{extra}\n}}\n"),
    )
    .unwrap();
    std::fs::write(dir.join("src").join("lib.rnx"), lib_src).unwrap();
    if let Some(text) = license {
        std::fs::write(dir.join("LICENSE"), text).unwrap();
    }
}

fn run(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(rnx());
    cmd.args(args).current_dir(dir);
    cmd.output().unwrap()
}

fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn license_tree(tag: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("licgraph-{tag}"));
    write_pkg(
        &dir.join("dep"),
        "dep",
        "0.2.0",
        "",
        "fn f(): Int { return 1; }\n",
        Some("MIT License\n"),
    );
    std::fs::create_dir_all(dir.join("app").join("src")).unwrap();
    std::fs::write(
        dir.join("app").join("Project.config"),
        "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    license: \"MIT\",\n    entries: { main: \"src/main.rnx\" },\n    dependencies: {\n        dep: { path: \"../dep\" }\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("app").join("LICENSE"), "MIT License\n").unwrap();
    std::fs::write(dir.join("app").join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
    dir
}

#[test]
fn audit_licenses_reports_graph_and_deny_gate() {
    let dir = license_tree("audit");
    let app = dir.join("app");
    let lock = run(&app, &["lock"]);
    assert!(lock.status.success(), "{}", combined(&lock));
    let out = run(&app, &["audit", "--licenses"]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("licenses:"), "{text}");
    assert!(text.contains("dep@0.2.0: MIT"), "{text}");
    assert!(text.contains("obligation: none"), "{text}");
    assert!(text.contains("no conflicts"), "{text}");
    let out = run(&app, &["audit", "--licenses", "--deny-license", "MIT"]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("DENIED"), "{text}");
    assert!(text.contains("denied license present"), "{text}");
    let out = run(&app, &["audit", "--licenses", "--deny-license", "Apache-2.0"]);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let out = run(&app, &["audit", "--deny-license", "Bogus-1.0"]);
    assert_ne!(out.status.code(), Some(0), "{}", combined(&out));
    assert!(combined(&out).contains("unknown license"), "{}", combined(&out));
}

#[test]
fn lock_rejects_dynamic_link_on_source_dependency() {
    let dir = crate::scratch::dir("liclink-dyn");
    std::fs::create_dir_all(dir.join("dep").join("src")).unwrap();
    write_pkg(&dir.join("dep"), "dep", "0.2.0", "", "fn f(): Int { return 1; }\n", Some("MIT License\n"));
    std::fs::create_dir_all(dir.join("app").join("src")).unwrap();
    std::fs::write(
        dir.join("app").join("Project.config"),
        "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    entries: { main: \"src/main.rnx\" },\n    dependencies: { dep: { path: \"../dep\", link: \"dynamic\" } }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("app").join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
    let out = run(&dir.join("app"), &["lock"]);
    assert!(!out.status.success(), "dynamic source link locked");
    assert!(combined(&out).contains("needs a native dependency"), "{}", combined(&out));
}

#[test]
fn publish_rejects_missing_license_file() {
    let dir = crate::scratch::dir("licpub-missing");
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: { name: \"probe\", version: \"1.2.3\", description: \"T.\" },\n    license: \"MIT\"\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int {\n    return 0;\n}\n").unwrap();
    let out = run(&dir, &["publish", "--registry", "http://localhost:9/api/packages", "--token", "dummy"]);
    assert_eq!(out.status.code(), Some(1), "{}", combined(&out));
    assert!(combined(&out).contains("no LICENSE"), "{}", combined(&out));
}
