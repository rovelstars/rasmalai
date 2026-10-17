#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use cli::{RunOutcome, run_source};
use diagnostics::Code;
use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn compile_codes(src: &str) -> Vec<Code> {
    match run_source(src, "Main", Vec::new()) {
        cli::RunResult { outcome: RunOutcome::Compile(errs), .. } => {
            errs.iter().map(|d| d.code).collect()
        }
        cli::RunResult { outcome: _, .. } => panic!("expected compile failure"),
    }
}

#[test]
fn run_source_blocks_mismatched_assign_with_e205() {
    let src = "fn Main(): Int {\n    let a = 2;\n    a = \"hello\";\n    return 0;\n}\n";
    assert!(compile_codes(src).contains(&Code::E205));
}

#[test]
fn run_source_accepts_unannotated_binding_as_null() {
    let src = "fn Main(): Int {\n    let a;\n    assert(a == null, \"uninit is null\");\n    return 0;\n}\n";
    match run_source(src, "Main", Vec::new()) {
        cli::RunResult { outcome: RunOutcome::Value(_), .. } => {}
        cli::RunResult { outcome: _, .. } => panic!("expected execution"),
    }
}

#[test]
fn run_source_executes_explicit_any_reassign() {
    let src = "fn Main(): Int {\n    let a: Any = 2;\n    a = \"hello\";\n    return 0;\n}\n";
    match run_source(src, "Main", Vec::new()) {
        cli::RunResult { outcome: RunOutcome::Value(_), .. } => {}
        cli::RunResult { outcome: _, .. } => panic!("expected execution"),
    }
}

fn write_prog(dir: &std::path::Path, body: &str) -> PathBuf {
    let p = dir.join("main.rnx");
    std::fs::write(&p, body).unwrap();
    p
}

#[test]
fn cli_run_exits_nonzero_on_e205() {
    let dir = crate::scratch::dir("typesafe");
    let prog = write_prog(&dir, "fn Main(): Int {\n    let a = 2;\n    a = \"hello\";\n    return 0;\n}\n");
    let out = Command::new(rnx())
        .arg("run")
        .env("NO_COLOR", "1")
        .arg(&prog)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stdout}{stderr}");
    assert_ne!(out.status.code(), Some(0), "{combined}");
    assert!(combined.contains("E205"), "{combined}");
}

#[test]
fn cli_run_exits_nonzero_on_e206_const() {
    let dir = crate::scratch::dir("typesafe206");
    let prog = write_prog(&dir, "fn Main(): Int {\n    const b;\n    return 0;\n}\n");
    let out = Command::new(rnx())
        .arg("run")
        .env("NO_COLOR", "1")
        .arg(&prog)
        .output()
        .unwrap();
    let stderr = String::from_utf8_lossy(&out.stderr);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let combined = format!("{stdout}{stderr}");
    assert_ne!(out.status.code(), Some(0), "{combined}");
    assert!(combined.contains("E206"), "{combined}");
}
