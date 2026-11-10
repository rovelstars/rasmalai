#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;
use std::process::{Command, Output};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(args: &[&str], dir: &Path, envs: &[(&str, &str)]) -> Output {
    let mut cmd = Command::new(rnx());
    cmd.args(args).current_dir(dir).env("NO_COLOR", "1");
    for (k, v) in envs {
        cmd.env(k, v);
    }
    cmd.output().unwrap()
}

#[test]
fn vec_macro_builds_and_runs_from_the_cli() {
    let dir = crate::scratch::dir("syntax-vec");
    std::fs::write(
        dir.join("main.rnx"),
        "fn Main(): Int {\n    let v = vec![1, 2, 3]\n    println!(\"len={v.len()} sum={v[0] + v[1] + v[2]}\")\n    return 0\n}\n",
    )
    .unwrap();

    let checked = run(&["check", "main.rnx"], &dir, &[]);
    assert!(
        checked.status.success(),
        "check rejected vec!: {}",
        String::from_utf8_lossy(&checked.stderr)
    );

    let out = run(&["run", "--backend", "interpreter", "main.rnx"], &dir, &[]);
    assert!(
        out.status.success(),
        "run rejected vec!: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "len=3 sum=6\n");
}

#[test]
fn embed_bytes_macro_builds_and_runs_from_the_cli() {
    let dir = crate::scratch::dir("syntax-embed");
    std::fs::write(dir.join("blob.bin"), [1u8, 2, 3, 4]).unwrap();
    std::fs::write(
        dir.join("main.rnx"),
        "fn Main(): Int {\n    let b = EmbedBytes!(\"blob.bin\")\n    println!(\"len={b.len()} first={b[0]} last={b[3]}\")\n    return 0\n}\n",
    )
    .unwrap();

    let checked = run(&["check", "main.rnx"], &dir, &[]);
    assert!(
        checked.status.success(),
        "check rejected EmbedBytes!: {}",
        String::from_utf8_lossy(&checked.stderr)
    );

    let out = run(&["run", "--backend", "interpreter", "main.rnx"], &dir, &[]);
    assert!(
        out.status.success(),
        "run rejected EmbedBytes!: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    assert_eq!(String::from_utf8_lossy(&out.stdout), "len=4 first=1 last=4\n");
}

#[test]
fn unknown_macro_still_reports_e303() {
    let dir = crate::scratch::dir("syntax-unknown");
    std::fs::write(
        dir.join("main.rnx"),
        "fn Main(): Int {\n    let v = BogusMacro![1]\n    return 0\n}\n",
    )
    .unwrap();

    let out = run(&["check", "main.rnx"], &dir, &[]);
    assert_eq!(out.status.code(), Some(1), "unknown macro must fail the check");
    let text = String::from_utf8_lossy(&out.stderr);
    assert!(text.contains("E303"), "expected E303, got: {text}");
    assert!(text.contains("BogusMacro"), "expected the macro name, got: {text}");
}
