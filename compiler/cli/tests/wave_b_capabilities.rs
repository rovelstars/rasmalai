use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_pkg(tag: &str, name: &str, lib_src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-waveb-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.3.0\",\n        entry: \"src/lib.rnx\"\n    }}\n}}\n"),
    )
    .unwrap();
    std::fs::write(dir.join("src/lib.rnx"), lib_src).unwrap();
    dir
}

fn audit(dir: &std::path::Path) -> std::process::Output {
    Command::new(rnx())
        .args(["audit", "--path"])
        .arg(dir)
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

const MUTATING_LIB: &str = "fn loadData(dir: String): Int {\n    File.open(dir + \"/data.txt\", FileMode.Read);\n    return 0;\n}\n";
const PASS_LIB: &str = "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\n";

#[test]
fn s201_fires_on_mutated_delegated_sink() {
    let dir = fresh_pkg("s201", "mutlib", MUTATING_LIB);
    let out = audit(&dir);
    assert_ne!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("S201"), "{text}");
    assert!(text.contains("fs"), "{text}");
    assert!(text.contains("File.open"), "{text}");
    assert!(text.contains("untrusted path mutation"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn passthrough_delegated_stays_clean() {
    let dir = fresh_pkg("pass", "passlib", PASS_LIB);
    let out = audit(&dir);
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let text = combined(&out);
    assert!(text.contains("fs:delegated"), "{text}");
    assert!(!text.contains("S201"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn lock_project(tag: &str, app_deps: &str, app_main: &str, dep_lib: &str) -> std::path::PathBuf {
    let dir =
        std::env::temp_dir().join(format!("rnx-waveb-lock-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("app").join("src")).unwrap();
    std::fs::create_dir_all(dir.join("dep").join("src")).unwrap();
    std::fs::write(
        dir.join("app").join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n        {app_deps}    }}\n}}\n"),
    )
    .unwrap();
    std::fs::write(dir.join("app").join("src").join("main.rnx"), app_main).unwrap();
    std::fs::write(
        dir.join("dep").join("Project.config"),
        "export default {\n    project: {\n        name: \"dep\",\n        version: \"0.3.0\",\n        entry: \"src/lib.rnx\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("dep").join("src").join("lib.rnx"), dep_lib).unwrap();
    dir
}

#[test]
fn lock_records_scanned_capabilities() {
    let dir = lock_project(
        "clean",
        "dep: { path: \"../dep\" }\n",
        "import { loadData } from \"dep\";\n\nfn Main(): Int {\n    print(loadData(\"/tmp/x\"));\n    return 0;\n}\n",
        PASS_LIB,
    );
    let out = Command::new(rnx())
        .args(["lock"])
        .current_dir(dir.join("app"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(0), "{}", combined(&out));
    let lock = std::fs::read_to_string(dir.join("app").join("Project.deplock")).unwrap();
    assert!(lock.contains("fs:delegated"), "lock must carry scanned caps, got: {lock}");
    assert!(lock.contains("delegated"), "lock must carry scanned tier, got: {lock}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn lock_rejects_s201_dependency() {
    let dir = lock_project(
        "reject",
        "dep: { path: \"../dep\" }\n",
        "import { loadData } from \"dep\";\n\nfn Main(): Int {\n    print(loadData(\"/tmp/x\"));\n    return 0;\n}\n",
        MUTATING_LIB,
    );
    let out = Command::new(rnx())
        .args(["lock"])
        .current_dir(dir.join("app"))
        .output()
        .unwrap();
    assert_ne!(out.status.code(), Some(0), "{}", combined(&out));
    assert!(combined(&out).contains("S201"), "{}", combined(&out));
    assert!(
        !dir.join("app").join("Project.deplock").exists(),
        "no clean lockfile may be written"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
