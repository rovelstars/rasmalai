#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;
use std::process::{Command, Output};

fn rnx() -> &'static str {
    env!("CARGO_BIN_EXE_rnx")
}

fn program(tag: &str, src: &str) -> crate::scratch::Scratch {
    let dir = crate::scratch::dir(&format!("log-{tag}"));
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    dir
}

fn run(dir: &Path, level: Option<&str>) -> Output {
    let mut cmd = Command::new(rnx());
    cmd.arg("run").arg("main.rnx").current_dir(dir);
    match level {
        Some(level) => {
            cmd.env("RNX_LOG", level);
        }
        None => {
            cmd.env_remove("RNX_LOG");
        }
    }
    cmd.output().unwrap()
}

fn clean_stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|line| !line.starts_with("warning: using local source for `@std/"))
        .map(|line| format!("{line}\n"))
        .collect()
}

const ALL_LEVELS: &str = "fn Main(): Int {\n    debug!(\"d\");\n    log!(\"l\");\n    info!(\"starting\");\n    warn!(\"low disk: {}\", 3);\n    error!(\"failed: {}\", \"x\");\n    return 0;\n}\n";

#[test]
fn default_threshold_is_info_and_formats_like_io() {
    let dir = program("default", ALL_LEVELS);
    let out = run(&dir, None);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(
        clean_stderr(&out),
        "INFO l\nINFO starting\nWARN low disk: 3\nERROR failed: x\n"
    );
}

#[test]
fn debug_threshold_shows_debug() {
    let dir = program("debug", ALL_LEVELS);
    let out = run(&dir, Some("debug"));
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(
        clean_stderr(&out),
        "DEBUG d\nINFO l\nINFO starting\nWARN low disk: 3\nERROR failed: x\n"
    );
}

#[test]
fn warn_threshold_hides_info() {
    let dir = program("warn", ALL_LEVELS);
    let out = run(&dir, Some("warn"));
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(
        clean_stderr(&out),
        "WARN low disk: 3\nERROR failed: x\n"
    );
}

#[test]
fn error_threshold_hides_warn() {
    let dir = program("error", ALL_LEVELS);
    let out = run(&dir, Some("error"));
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(clean_stderr(&out), "ERROR failed: x\n");
}

#[test]
fn off_produces_no_output() {
    let dir = program("off", ALL_LEVELS);
    let out = run(&dir, Some("off"));
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert_eq!(clean_stderr(&out), "");
}

#[test]
fn hidden_level_does_not_format_a_missing_argument() {
    let dir = program("hidden", "fn Main(): Int {\n    debug!(\"hidden {}\", );\n    return 0;\n}\n");
    let out = run(&dir, None);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(clean_stderr(&out), "");
}

#[test]
fn explicit_import_and_ambient_macros_agree() {
    let dir = program(
        "explicit",
        "import { warn } from \"@std/log\";\nfn Main(): Int {\n    info!(\"ambient\");\n    warn!(\"direct {}\", 1);\n    return 0;\n}\n",
    );
    let out = run(&dir, None);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(
        clean_stderr(&out),
        "INFO ambient\nWARN direct 1\n"
    );
}
