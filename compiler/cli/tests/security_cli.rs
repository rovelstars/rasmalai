use std::io::Write;
use std::process::{Command, Stdio};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-sec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app/src")).unwrap();
    std::fs::create_dir_all(dir.join("dep/src")).unwrap();
    dir
}

fn write(path: &std::path::Path, content: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    let mut f = std::fs::File::create(path).unwrap();
    f.write_all(content.as_bytes()).unwrap();
}

fn app_manifest(deps: &str, permissions: Option<&str>) -> String {
    let mut out = "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    }".to_string();
    if !deps.is_empty() {
        out.push_str(&format!(",\n    dependencies: {{\n        {deps}    }}", deps = deps.replace(" = { path = ", ": { path: ").replace(" }", " }")));
    }
    if let Some(p) = permissions {
        out.push_str(&format!(",\n    permissions: [{p}]"));
    }
    out.push_str("\n}\n");
    out
}

fn dep_manifest(name: &str) -> String {
    format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.2.0\"\n    }},\n    entries: {{ main: \"src/lib.rnx\" }}\n}}\n")
}

fn run_headless(args: &[&str], cwd: &std::path::Path) -> std::process::Output {
    Command::new(rnx())
        .args(args)
        .current_dir(cwd)
        .stdin(Stdio::null())
        .output()
        .unwrap()
}

fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

#[test]
fn passing_build_with_matching_lock() {
    let dir = fresh_dir("pass");
    let app = dir.join("app");
    write(&app.join("Project.config"), &app_manifest("", None));
    write(
        &app.join("src/main.rnx"),
        "import { helper } from \"dep\";\nfn Main(): Int {\n    helper();\n    return 0;\n}\n",
    );
    write(&dir.join("dep/Project.config"), &dep_manifest("dep"));
    write(&dir.join("dep/src/lib.rnx"), "fn helper(): Int {\n    return 1;\n}\n");
    let add = run_headless(
        &["add", "dep", "--path", "../dep"],
        &app,
    );
    assert!(add.status.success(), "{}", combined(&add));
    let check = run_headless(&["check"], &app);
    assert_eq!(check.status.code(), Some(0), "{}", combined(&check));
    let build = run_headless(
        &["build", "-q"],
        &app,
    );
    assert_eq!(build.status.code(), Some(0), "{}", combined(&build));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s101_rejects_unapproved_capability() {
    let dir = fresh_dir("s101");
    let app = dir.join("app");
    write(&app.join("Project.config"), &app_manifest("", None));
    write(
        &app.join("src/main.rnx"),
        "import { loadData } from \"dep\";\nfn Main(): Int {\n    loadData(\"/tmp/a\");\n    return 0;\n}\n",
    );
    write(&dir.join("dep/Project.config"), &dep_manifest("dep"));
    write(
        &dir.join("dep/src/lib.rnx"),
        "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n",
    );
    let add = run_headless(
        &["add", "dep", "--path", "../dep", "--accept-caps=fs:delegated"],
        &app,
    );
    assert!(add.status.success(), "{}", combined(&add));
    write(
        &dir.join("dep/src/lib.rnx"),
        "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    File.open(\"/tmp/cache.lock\", FileMode.Read);\n    return 0;\n}\n",
    );
    let build = run_headless(&["build", "-q"], &app);
    assert_eq!(build.status.code(), Some(1), "{}", combined(&build));
    let text = combined(&build);
    assert!(text.contains("S101"), "{text}");
    assert!(text.contains("fs:read:/tmp/**"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn s102_rejects_ceiling_violation() {
    let dir = fresh_dir("s102");
    let app = dir.join("app");
    write(&app.join("Project.config"), &app_manifest("", None));
    write(
        &app.join("src/main.rnx"),
        "import { loadData } from \"dep\";\nfn Main(): Int {\n    loadData(\"/tmp/a\");\n    return 0;\n}\n",
    );
    write(&dir.join("dep/Project.config"), &dep_manifest("dep"));
    write(
        &dir.join("dep/src/lib.rnx"),
        "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n",
    );
    let add = run_headless(
        &["add", "dep", "--path", "../dep", "--accept-caps=fs:delegated"],
        &app,
    );
    assert!(add.status.success(), "{}", combined(&add));
    write(
        &app.join("Project.config"),
        &app_manifest("dep = { path = \"../dep\" }\n", Some("")),
    );
    let check = run_headless(&["check"], &app);
    assert_eq!(check.status.code(), Some(1), "{}", combined(&check));
    let text = combined(&check);
    assert!(text.contains("S102"), "{text}");
    assert!(text.contains("fs:delegated"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_add_denied_with_exit_2() {
    let dir = fresh_dir("headless");
    let app = dir.join("app");
    write(&app.join("Project.config"), &app_manifest("", None));
    write(&app.join("src/main.rnx"), "fn Main(): Int {\n    return 0;\n}\n");
    write(&dir.join("dep/Project.config"), &dep_manifest("dep"));
    write(
        &dir.join("dep/src/lib.rnx"),
        "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n",
    );
    let add = run_headless(&["add", "dep", "--path", "../dep"], &app);
    assert_eq!(add.status.code(), Some(2), "{}", combined(&add));
    let text = combined(&add);
    assert!(text.contains("--accept-caps"), "{text}");
    assert!(text.contains("fs:delegated"), "{text}");
    assert!(
        !app.join("Project.deplock").is_file(),
        "lockfile must stay untouched on denial"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn headless_add_with_accept_caps_writes_v2() {
    let dir = fresh_dir("accept");
    let app = dir.join("app");
    write(&app.join("Project.config"), &app_manifest("", None));
    write(&app.join("src/main.rnx"), "fn Main(): Int {\n    return 0;\n}\n");
    write(&dir.join("dep/Project.config"), &dep_manifest("dep"));
    write(
        &dir.join("dep/src/lib.rnx"),
        "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n",
    );
    let add = run_headless(
        &["add", "dep", "--path", "../dep", "--accept-caps=fs:delegated"],
        &app,
    );
    assert_eq!(add.status.code(), Some(0), "{}", combined(&add));
    let lock_text = std::fs::read_to_string(app.join("Project.deplock")).unwrap();
    assert!(lock_text.contains("version: 2"), "{lock_text}");
    assert!(lock_text.contains("tier: \"delegated\""), "{lock_text}");
    assert!(lock_text.contains("\"fs:delegated\""), "{lock_text}");
    let config = std::fs::read_to_string(app.join("Project.config")).unwrap();
    assert!(config.contains("dep: { path: \"../dep\" }"), "{config}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lockfile_round_trips_through_rnx() {
    let v1 = "export default {\n    version: 1,\n    packages: [\n        {\n            name: \"old\",\n            version: \"1.0.0\",\n            source: \"path:../old\",\n            checksum: \"abc\",\n            dependencies: []\n        }\n    ]\n}\n";
    let lock = frontend::deplock::ProjectDepLock::parse(v1).unwrap();
    assert_eq!(lock.packages.len(), 1);
    assert_eq!(lock.packages[0].tier, "pure");
    assert!(lock.packages[0].capabilities.is_empty());
    let round = lock.to_rnx();
    assert!(round.contains("tier: \"pure\""), "{round}");
    let again = frontend::deplock::ProjectDepLock::parse(&round).unwrap();
    assert_eq!(again, lock);
}

#[test]
fn permissions_section_parses() {
    let text = "export default {\n    project: {\n        name: \"lib\",\n        version: \"0.1.0\"\n    },\n    permissions: [\"fs:delegated\"]\n}\n";
    let dir = std::env::temp_dir().join(format!("rnx-sec-perm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("Project.config"), text).unwrap();
    let config =
        frontend::project::ProjectConfig::load_from_dir(&dir).unwrap().unwrap();
    assert_eq!(
        config.permissions,
        Some(vec![frontend::project::PermissionDecl {
            perm: "fs:delegated".to_string(),
            reason: None
        }])
    );
    let _ = std::fs::remove_dir_all(&dir);
}
