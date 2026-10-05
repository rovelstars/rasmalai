use std::path::{Path, PathBuf};
use std::process::Command;

// JavaScript-ergonomics pins for control flow: stray semicolons parse as
// `Stmt::Empty` (a pure no-op, discarded nowhere, emitted nowhere), and
// `if`/`else` accept single-statement bodies by wrapping them in synthetic
// `Block`s (the same shape `defer` already used for bare statements).
// Dangling `else` binds to the nearest open `if`, as recursive descent
// guarantees.
//
// `let` is Rasmalai's mutable binding; there is no `mut` keyword, so the
// accumulator below is a plain `let`. Declarations are rejected as
// single-statement bodies (`if (c) let x = 1;` is E108, mirroring the JS
// rule against lexical declarations in single-statement context).
//
// Each backend runs in a SUBPROCESS so an abort on any one of them cannot
// kill the in-process harness.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

const SRC: &str = "fn fib(a: Int): Int {\n\
    if (a == 0) { return 0; };\n\
    if (a == 1) return 1;\n\
    return fib(a - 1) + fib(a - 2);\n\
}\n\
\n\
fn test_dangling_else(x: Int): Int {\n\
    let res = 0;\n\
    if (x > 0)\n\
        if (x > 10) res = 2;\n\
        else res = 1;\n\
    return res;\n\
}\n\
\n\
fn Main(): Int {\n\
    assert(fib(0) == 0, \"fib 0\");\n\
    assert(fib(1) == 1, \"fib 1\");\n\
    assert(fib(10) == 55, \"fib 10\");\n\
    assert(test_dangling_else(5) == 1, \"nested else binds to inner if\");\n\
    assert(test_dangling_else(15) == 2, \"inner then\");\n\
    assert(test_dangling_else(-1) == 0, \"outer false\");\n\
    return 0;\n\
}\n";

const DECL_SRC: &str = "fn Main(): Int {\n\
    if (true) let x = 1;\n\
    return 0;\n\
}\n";

fn fresh_src(tag: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-jscompat-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    main
}

fn run_backend(main: &Path, backend: &str) -> std::process::Output {
    Command::new(rnx())
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(main)
        .output()
        .unwrap()
}

#[test]
fn js_compat_control_flow_interpreter() {
    let main = fresh_src("interp", SRC);
    let out = run_backend(&main, "interpreter");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn js_compat_control_flow_cranelift() {
    let main = fresh_src("cl", SRC);
    let out = run_backend(&main, "cranelift");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn js_compat_control_flow_llvm() {
    let main = fresh_src("ll", SRC);
    let out = run_backend(&main, "llvm");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn js_compat_control_flow_aot() {
    let main = fresh_src("aot", SRC);
    let dir = main.parent().unwrap().to_path_buf();
    let build = Command::new(rnx())
        .arg("build")
        .arg(&main)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot exit: {}", String::from_utf8_lossy(&run.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn js_compat_decl_as_single_body_rejected() {
    let main = fresh_src("decl", DECL_SRC);
    let out = Command::new(rnx()).arg("check").arg(&main).output().unwrap();
    assert!(!out.status.success(), "declaration as single-statement body must fail");
    let err = String::from_utf8_lossy(&out.stderr);
    assert!(
        err.contains("single-statement"),
        "diagnostic names the rule, got: {err}"
    );
    let _ = std::fs::remove_dir_all(main.parent().unwrap());
}
