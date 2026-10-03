use std::path::{Path, PathBuf};
use std::process::Command;

// Regression pin for the loop-carried ternary ARC bug (fixed).
//
// Failure mode was: `out = cond ? a_str : b_str` inside a loop abort()ed on
// Cranelift/LLVM/AOT with `rnx panic: rnx_release on zero count`. The
// interpreter was unaffected. Minimal trigger needs all three: a loop
// back-edge, a ternary producing owned Strings, and reassignment into
// an outer variable (fresh per-iteration bindings like
// `let t = cond ? "e" : "o"` are fine, as are loops without ternaries).
//
// Root cause: the second-lowered branch's `Copy dst <- src` of the merge
// temp emitted `release old dst` from the flow-insensitive static `owned`
// set. On straight-line joins the stale release hits a null slot
// (harmless no-op); on loop re-entry it hits the previous iteration's
// live value, already moved into the outer variable.
//
// Fix: a stale `release old dst` may only fire when the destination is
// genuinely live. Release-old additionally requires write-dominance:
// `last_write[dst]` must dominate the current block (via
// `lir::licm::compute_dominators`; writes tracked centrally in
// cranelift's `set()` / LLVM's `store()`). Sibling-branch and
// loop-backedge pollution no longer dominates, so those releases are
// safely suppressed; stack locals keep standard cleanup. Suppression
// can only delay frees to function scope, never add releases, so no new
// failure mode is introduced. A discarded earlier attempt (nulling
// moved-from slots) broke tuple field share-copies, which legitimately
// re-read moved-from slots, and was reverted.
//
// These tests run each backend in a SUBPROCESS so any future abort in
// this area cannot kill the in-process harness.

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

const REPRO: &str = "fn Main(): Int {\n\
    let i = 0;\n\
    let out = \"\";\n\
    while (i < 5) {\n\
        out = i % 2 == 0 ? out + \"e\" : out + \"o\";\n\
        i = i + 1;\n\
    }\n\
    assert(out == \"eoeoe\", \"pattern\");\n\
    let j = 0;\n\
    let acc = \"\";\n\
    while (j < 5) {\n\
        let hit = j % 2 == 0 ? \"hit\" : null;\n\
        acc = acc + (hit ?? \"miss\");\n\
        j = j + 1;\n\
    }\n\
    assert(acc == \"hitmisshitmisshit\", \"coalesce loop\");\n\
    return 0;\n\
}\n";

fn fresh_src(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-looptern-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, REPRO).unwrap();
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
fn loop_ternary_release_interpreter() {
    let main = fresh_src("interp");
    let out = run_backend(&main, "interpreter");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn loop_ternary_release_cranelift() {
    let main = fresh_src("cl");
    let out = run_backend(&main, "cranelift");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn loop_ternary_release_llvm() {
    let main = fresh_src("ll");
    let out = run_backend(&main, "llvm");
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
}

#[test]
fn loop_ternary_release_aot() {
    let main = fresh_src("aot");
    let dir = main.parent().unwrap().to_path_buf();
    let bin_path = dir.join("looptern_bin");
    let build = Command::new(rnx())
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot exit: {}", String::from_utf8_lossy(&run.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}
