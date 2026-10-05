use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-wave4-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(path: &PathBuf, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn manifest(name: &str) -> String {
    format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n")
}

fn run_backend(file: &PathBuf, backend: &str, args: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(rnx());
    cmd.arg("run").arg("--backend").arg(backend).arg(file);
    if !args.is_empty() {
        cmd.arg("--");
        for a in args {
            cmd.arg(a);
        }
    }
    cmd.output().unwrap()
}

const ARGS_SRC: &str = "import { Process } from \"@std/process\";\nfn Main(): Int {\n    let a = Process.args();\n    print(a.length());\n    for x in a {\n        print(x);\n    }\n    return 0;\n}\n";

const USER_ARGS: &[&str] = &["first arg", "second", "third with spaces", "--flag", ""];

#[test]
fn argv_forwards_identically_on_all_backends() {
    let dir = fresh_dir("argv");
    let file = dir.join("args.rnx");
    write_file(&file, ARGS_SRC);
    let mut prior: Option<(i32, String)> = None;
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend(&file, backend, USER_ARGS);
        let code = out.status.code().unwrap();
        let stdout = String::from_utf8(out.stdout.clone()).unwrap();
        assert_eq!(code, 0, "{backend}: exit, stderr: {}", String::from_utf8_lossy(&out.stderr));
        assert!(
            out.stderr.is_empty(),
            "{backend}: stderr must stay clean, got: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        let mut lines = stdout.lines();
        let user: Vec<&str> = lines.by_ref().skip(2).collect();
        assert_eq!(
            user,
            USER_ARGS,
            "{backend}: user argv must survive spaces, flags, and empties verbatim"
        );
        let full = format!("{code}:{stdout}");
        if let Some((_, prev)) = prior.as_ref() {
            let prev_lines: Vec<&str> = prev.lines().skip(1).collect();
            let cur_lines: Vec<&str> = stdout.lines().skip(1).collect();
            assert_eq!(prev_lines, cur_lines, "{backend}: argv output must match other backends");
        }
        prior = Some((code, full));
    }
    let build = Command::new(rnx())
        .arg("build")
        .arg(&file)
        .arg("-q")
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let mut cmd = Command::new(&bin);
    for a in USER_ARGS {
        cmd.arg(a);
    }
    let out = cmd.output().unwrap();
    assert_eq!(out.status.code(), Some(0));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let user: Vec<&str> = stdout.lines().skip(2).collect();
    assert_eq!(user, USER_ARGS, "aot: user argv must match jit/interpreter");
    let _ = std::fs::remove_dir_all(&dir);
}

const EXIT_SRC: &str = "import { Process } from \"@std/process\";\nfn Main(): Int {\n    print(\"before exit\");\n    Process.exit(3);\n    print(\"after exit\");\n    return 0;\n}\n";

#[test]
fn process_exit_flushes_stdout_on_all_backends() {
    let dir = fresh_dir("exit");
    let file = dir.join("exit.rnx");
    write_file(&file, EXIT_SRC);
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend(&file, backend, &[]);
        assert_eq!(out.status.code(), Some(3), "{backend}: exit code");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "before exit\n",
            "{backend}: prior output must survive Process.exit"
        );
    }
    let build = Command::new(rnx())
        .arg("build")
        .arg(&file)
        .arg("-q")
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let out = Command::new(&bin).output().unwrap();
    assert_eq!(out.status.code(), Some(3), "aot: exit code");
    assert_eq!(String::from_utf8(out.stdout).unwrap(), "before exit\n", "aot: prior output");
    let _ = std::fs::remove_dir_all(&dir);
}

const CEILING_CONFIG: &str = "export default {\n    project: {\n        name: \"t3\",\n        version: \"0.1.0\"\n    },\n    permissions: [\"fs:read:/data\"]\n}\n";
const CEILING_SRC: &str =
    "unsafe fn Raw(x: Int): Int { return x * 2 }\n\nunsafe { print(Raw(21)); }\n";

#[test]
fn permissions_ceiling_enforced_without_lockfile() {
    let dir = fresh_dir("ceiling");
    write_file(&dir.join("Project.config"), CEILING_CONFIG);
    write_file(&dir.join("src/main.rnx"), CEILING_SRC);
    let check = Command::new(rnx()).arg("check").current_dir(&dir).output().unwrap();
    assert_eq!(check.status.code(), Some(1), "check must enforce the ceiling");
    let text = String::from_utf8_lossy(&check.stderr).into_owned();
    assert!(text.contains("S102"), "got:\n{text}");
    assert!(!text.contains("(ok)"), "no success line on failure, got:\n{text}");
    let build = Command::new(rnx())
        .arg("build")
        .arg("-q")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert_eq!(build.status.code(), Some(1), "build must not ship a ceiling violation");
    assert!(!dir.join("t3bin").exists(), "no binary may be written");
    let run = Command::new(rnx()).arg("run").current_dir(&dir).output().unwrap();
    assert_eq!(run.status.code(), Some(0), "bare run stays unenforced");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "42\n");
    let lock = Command::new(rnx()).arg("lock").current_dir(&dir).output().unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    let locked = Command::new(rnx()).arg("run").arg("--locked").current_dir(&dir).output().unwrap();
    assert_eq!(locked.status.code(), Some(1), "run --locked enforces the ceiling");
    assert!(
        String::from_utf8_lossy(&locked.stderr).contains("S102"),
        "{}",
        String::from_utf8_lossy(&locked.stderr)
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const TOPLEVEL_FS_SRC: &str = "import { File, OpenMode } from \"@std/fs\";\nfn Main(): Int {\n    let f = File.open(\"/tmp/rnx-notes.txt\", OpenMode.Read);\n    let t = f.readText().unwrapOr(\"\");\n    f.close();\n    print(t.length());\n    return 0;\n}\n";

#[test]
fn audit_attributes_file_capability_and_exports_manifest() {
    let dir = fresh_dir("audit");
    write_file(&dir.join("Project.config"), &manifest("p"));
    write_file(&dir.join("src/main.rnx"), TOPLEVEL_FS_SRC);
    let audit = Command::new(rnx())
        .arg("audit")
        .arg("--json")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(audit.status.success());
    let report: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(report["tier"], "ambient");
    let caps = report["capabilities"].as_array().unwrap();
    assert!(caps.iter().any(|c| c == "fs:read:/tmp/**"), "{report}");
    let manifest_out = Command::new(rnx())
        .arg("audit")
        .arg("--export-manifest")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(manifest_out.status.success());
    let exported: serde_json::Value = serde_json::from_slice(&manifest_out.stdout).unwrap();
    for key in ["name", "version", "tier", "capabilities"] {
        assert!(exported.get(key).is_some(), "manifest schema key {key}: {exported}");
    }
    assert_eq!(exported["name"], "p");
    assert_eq!(exported["tier"], "ambient");
    assert!(
        exported["capabilities"].as_array().unwrap().iter().any(|c| c == "fs:read:/tmp/**"),
        "exported manifest must carry the scanned capability: {exported}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const TOPLEVEL_STMT_SRC: &str = "import { File, OpenMode } from \"@std/fs\";\nlet f = File.open(\"/tmp/rnx-notes.txt\", OpenMode.Read);\nlet t = f.readText().unwrapOr(\"\");\nf.close();\nprint(t.length());\n";

#[test]
fn audit_sees_top_level_statements() {
    let dir = fresh_dir("toplevel");
    write_file(&dir.join("Project.config"), &manifest("p"));
    write_file(&dir.join("src/main.rnx"), TOPLEVEL_STMT_SRC);
    let audit = Command::new(rnx())
        .arg("audit")
        .arg("--json")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(audit.status.success());
    let report: serde_json::Value = serde_json::from_slice(&audit.stdout).unwrap();
    assert_eq!(report["tier"], "ambient", "top-level file use is not pure: {report}");
    assert!(
        report["capabilities"].as_array().unwrap().iter().any(|c| c == "fs:read:/tmp/**"),
        "{report}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
