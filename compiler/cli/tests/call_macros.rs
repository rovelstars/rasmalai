#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;
use std::process::{Command, Output};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(args: &[&str], dir: &Path) -> Output {
    let mut cmd = Command::new(rnx());
    cmd.args(args).current_dir(dir).env("NO_COLOR", "1");
    cmd.output().unwrap()
}

fn clean_stderr(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr)
        .lines()
        .filter(|l| !l.starts_with("warning: using local source"))
        .collect::<Vec<_>>()
        .join("\n")
}

fn write_program(dir: &Path, src: &str) {
    std::fs::write(dir.join("main.rnx"), src).unwrap();
}

#[test]
fn println_and_print_output_and_newlines() {
    let dir = crate::scratch::dir("call-macro-output");
    write_program(
        &dir,
        "fn Main(): Int {\n    println!(\"x = {}\", 10)\n    print!(\"no newline\")\n    return 0\n}\n",
    );
    let out = run(&["run", "main.rnx"], &dir);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "x = 10\nno newline");
}

#[test]
fn eprintln_routes_to_stderr() {
    let dir = crate::scratch::dir("call-macro-stderr");
    write_program(
        &dir,
        "fn Main(): Int {\n    eprintln!(\"err {}\", 1)\n    return 0\n}\n",
    );
    let out = run(&["run", "main.rnx"], &dir);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "");
    assert!(clean_stderr(&out).contains("err 1"), "{}", clean_stderr(&out));
}

#[test]
fn format_placeholders_braces_missing_and_extra() {
    let dir = crate::scratch::dir("call-macro-format");
    write_program(
        &dir,
        "fn Main(): Int {\n    println!(\"x =\", 1)\n    println!(\"{{literal}}\", 2)\n    println!(\"{}\", 1, 2, 3)\n    println!(\"{} {}\", 1)\n    return 0\n}\n",
    );
    let out = run(&["run", "main.rnx"], &dir);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "x = 1\n{literal} 2\n1 2 3\n1 {}\n"
    );
    let stderr = clean_stderr(&out);
    assert_eq!(
        stderr.matches("placeholders than arguments").count(),
        1,
        "{stderr}"
    );
}

#[test]
fn explicit_import_and_ambient_macro_both_resolve() {
    let dir = crate::scratch::dir("call-macro-explicit");
    write_program(
        &dir,
        "import { println } from \"@std/io\"\n\nfn Main(): Int {\n    println!(\"explicit {}\", 7)\n    println!(\"ambient {}\", 8)\n    return 0\n}\n",
    );
    let out = run(&["run", "main.rnx"], &dir);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(
        String::from_utf8_lossy(&out.stdout),
        "explicit 7\nambient 8\n"
    );
}

#[test]
fn ordinary_print_call_is_rejected() {
    let dir = crate::scratch::dir("call-macro-print-rejected");
    write_program(&dir, "fn Main(): Int {\n    print(\"a\", 1)\n    return 0\n}\n");
    let out = run(&["check", "main.rnx"], &dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = clean_stderr(&out);
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("print!"), "{stderr}");

    write_program(&dir, "fn Main(): Int {\n    print!(\"a\", 1)\n    return 0\n}\n");
    let out = run(&["run", "main.rnx"], &dir);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    assert_eq!(String::from_utf8_lossy(&out.stdout), "a 1");
}

#[test]
fn wrong_bang_usage_is_e108_in_both_directions() {
    let dir = crate::scratch::dir("call-macro-wrong-bang");
    write_program(
        &dir,
        "fn Main(): Int {\n    println(\"x\")\n    return 0\n}\n",
    );
    let out = run(&["check", "main.rnx"], &dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = clean_stderr(&out);
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("call macro"), "{stderr}");

    write_program(
        &dir,
        "fn helper(x: Int): Int {\n    return x\n}\n\nfn Main(): Int {\n    return helper!(1)\n}\n",
    );
    let out = run(&["check", "main.rnx"], &dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = clean_stderr(&out);
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("remove `!`"), "{stderr}");
}

#[test]
fn macro_signature_validation_fails_the_check() {
    let dir = crate::scratch::dir("call-macro-signature");
    write_program(
        &dir,
        "#[Macro]\nexport fn bad(a: Int): Int {\n    return a\n}\n\nfn Main(): Int {\n    return 0\n}\n",
    );
    let out = run(&["check", "main.rnx"], &dir);
    assert_eq!(out.status.code(), Some(1));
    let stderr = clean_stderr(&out);
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("Array<Any>"), "{stderr}");
    assert!(stderr.contains("Void"), "{stderr}");
}

#[test]
fn expand_macros_dumps_syntax_stages() {
    let dir = crate::scratch::dir("call-macro-expand-dump");
    write_program(
        &dir,
        "fn Main(): Int {\n    println!(\"x = {}\", 10)\n    return 0\n}\n",
    );
    let out = run(&["check", "--expand-macros", "main.rnx"], &dir);
    assert!(out.status.success(), "{}", clean_stderr(&out));
    let dump = dir.join(".rnx-cache").join("expansions");
    for name in ["syntax.before", "syntax.after"] {
        let text = std::fs::read_to_string(dump.join(name))
            .unwrap_or_else(|e| panic!("{name}: {e}"));
        assert!(!text.is_empty(), "{name}");
    }
    let after = std::fs::read_to_string(dump.join("syntax.after")).unwrap();
    assert!(after.contains("Array"), "{after}");
}
