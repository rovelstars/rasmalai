use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-wave2-{tag}-{}", std::process::id()));
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

fn run_backend(file: &PathBuf, backend: &str) -> std::process::Output {
    Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(file)
        .output()
        .unwrap()
}

const LOWER_FAIL_SRC: &str =
    "fn Main(): Int {\n    for (a, b) in 0 .. 3 {\n        print(a);\n    }\n    return 0;\n}\n";

#[test]
fn check_matches_run_on_lowering_failure() {
    let dir = fresh_dir("parity");
    let file = dir.join("lowerfail.rnx");
    write_file(&file, LOWER_FAIL_SRC);
    let check = Command::new(rnx()).arg("check").arg(&file).output().unwrap();
    let run = run_backend(&file, "interpreter");
    assert_eq!(check.status.code(), Some(1), "check must reject lowering failures");
    assert_eq!(run.status.code(), Some(1), "run must reject lowering failures");
    assert_eq!(
        String::from_utf8_lossy(&check.stderr),
        String::from_utf8_lossy(&run.stdout),
        "check and run must report the identical diagnostic"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const RECURSE_SRC: &str =
    "fn loop_rec(): Int {\n    return loop_rec();\n}\nfn Main(): Int {\n    print(loop_rec());\n    return 0;\n}\n";

#[test]
fn recursion_traps_cleanly_on_all_backends() {
    let dir = fresh_dir("recurse");
    let file = dir.join("deep.rnx");
    write_file(&file, RECURSE_SRC);
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend(&file, backend);
        assert_eq!(
            out.status.code(),
            Some(1),
            "{backend}: recursion must exit 1, not abort"
        );
        let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
        assert!(
            stderr.contains("call stack exhausted"),
            "{backend}: stderr must name stack exhaustion, got: {stderr}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

const THROW_SRC: &str = "fn boom(): String throws {\n    throw \"fatal error\";\n}\nfn Main(): Int {\n    print(\"before\");\n    boom();\n    print(\"after\");\n    return 0;\n}\n";

fn assert_uncaught_shape(out: &std::process::Output, backend: &str) {
    assert_eq!(out.status.code(), Some(1), "{backend}: exit code");
    assert_eq!(
        String::from_utf8(out.stdout.clone()).unwrap(),
        "before\n",
        "{backend}: program output must stay on stdout, diagnostics off it"
    );
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        stderr.contains("Uncaught exception: fatal error"),
        "{backend}: stderr, got: {stderr}"
    );
}

#[test]
fn uncaught_throw_renders_identically() {
    let dir = fresh_dir("uncaught");
    let file = dir.join("throw.rnx");
    write_file(&file, THROW_SRC);
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend(&file, backend);
        assert_uncaught_shape(&out, backend);
    }
    let bin = dir.join("throw_bin");
    let build = Command::new(rnx())
        .arg("build")
        .arg(&file)
        .arg("-o")
        .arg(&bin)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let out = Command::new(&bin).output().unwrap();
    assert_uncaught_shape(&out, "aot");
    let _ = std::fs::remove_dir_all(&dir);
}

const FS_MAIN_SRC: &str = "fn loadData(path: String): Int {\n    File.open(path, FileMode.Read);\n    return 0;\n}\nfn Main(): Int {\n    return loadData(\"/tmp/a\");\n}\n";

#[test]
fn lock_records_scanned_capabilities() {
    let dir = fresh_dir("lockcaps");
    write_file(
        &dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"fstest\",\n        version: \"0.1.0\"\n    }\n}\n",
    );
    write_file(&dir.join("src/main.rnx"), FS_MAIN_SRC);
    let lock = Command::new(rnx())
        .arg("lock")
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    let text = std::fs::read_to_string(dir.join("Project.deplock")).unwrap();
    assert!(!text.contains("tier = \"pure\""), "tier must be scan-derived:\n{text}");
    assert!(
        text.lines().any(|l| l.contains("fs:read")),
        "capabilities must list the scanned file read:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
