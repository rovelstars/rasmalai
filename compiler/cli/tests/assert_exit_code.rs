use std::process::Command;

fn write_case(tag: &str, body: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-assert-exit-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, body).unwrap();
    main
}

fn run_rnx(main: &std::path::Path, backend: &str) -> std::process::Output {
    Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("run")
        .arg(main)
        .arg("--backend")
        .arg(backend)
        .arg("--no-color")
        .arg("-q")
        .output()
        .unwrap()
}

const FAIL_SRC: &str = "fn Main(): Int {\n    assert(1 == 2, \"intentional failure\");\n    return 0;\n}\n";
const PASS_SRC: &str = "fn Main(): Int {\n    assert(1 == 1, \"all good\");\n    return 0;\n}\n";

#[test]
fn interpreter_failing_assert_exits_1_with_location() {
    let main = write_case("interp-fail", FAIL_SRC);
    let out = run_rnx(&main, "interpreter");
    assert_eq!(out.status.code(), Some(1), "exit code");
    assert!(out.stdout.is_empty(), "stdout stays clean: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("intentional failure"), "message: {stderr}");
    assert!(stderr.contains("main.rnx:2:5"), "file and line: {stderr}");
    let _ = std::fs::remove_dir_all(main.parent().unwrap());
}

#[test]
fn cranelift_failing_assert_exits_1_on_stderr() {
    let main = write_case("jit-fail", FAIL_SRC);
    let out = run_rnx(&main, "cranelift");
    assert_eq!(out.status.code(), Some(1), "exit code");
    assert!(out.stdout.is_empty(), "stdout stays clean: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("intentional failure"), "message: {stderr}");
    let _ = std::fs::remove_dir_all(main.parent().unwrap());
}

#[test]
fn llvm_failing_assert_exits_1_on_stderr() {
    let main = write_case("llvm-fail", FAIL_SRC);
    let out = run_rnx(&main, "llvm");
    assert_eq!(out.status.code(), Some(1), "exit code");
    assert!(out.stdout.is_empty(), "stdout stays clean: {out:?}");
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(stderr.contains("intentional failure"), "message: {stderr}");
    let _ = std::fs::remove_dir_all(main.parent().unwrap());
}

#[test]
fn aot_failing_assert_exits_1_on_stderr() {
    let main = write_case("aot-fail", FAIL_SRC);
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("-q")
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code(), Some(1), "exit code");
    assert!(run.stdout.is_empty(), "stdout stays clean");
    let stderr = String::from_utf8_lossy(&run.stderr);
    assert!(stderr.contains("intentional failure"), "message: {stderr}");
    let _ = std::fs::remove_dir_all(main.parent().unwrap());
}

#[test]
fn passing_assert_stays_exit_0() {
    for backend in ["interpreter", "cranelift"] {
        let main = write_case(&format!("pass-{backend}"), PASS_SRC);
        let out = run_rnx(&main, backend);
        assert_eq!(out.status.code(), Some(0), "{backend} exit code");
        let _ = std::fs::remove_dir_all(main.parent().unwrap());
    }
}
