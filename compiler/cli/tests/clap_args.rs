use std::path::PathBuf;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(args: &[&str], dir: &std::path::Path) -> (i32, String, String) {
    let out = std::process::Command::new(rnx())
        .args(args)
        .current_dir(dir)
        .output()
        .unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn proj(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let root = std::env::temp_dir().join(format!("rnx-clap-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: {\n        name: \"clap\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    for (rel, src) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    root
}

#[test]
fn test_help_exits_zero() {
    let dir = std::env::temp_dir();
    let (code, stdout, _) = run(&["--help"], &dir);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("Rasmalai systems language"), "{stdout}");
    assert!(stdout.contains("completions"), "{stdout}");
}

#[test]
fn test_typo_suggests_subcommand() {
    let dir = std::env::temp_dir();
    let (code, _, stderr) = run(&["chck"], &dir);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("check"), "{stderr}");
}

#[test]
fn test_completions_for_all_shells() {
    let dir = std::env::temp_dir();
    for shell in ["bash", "zsh", "fish", "powershell", "elvish"] {
        let (code, stdout, stderr) = run(&["completions", shell], &dir);
        assert_eq!(code, 0, "{shell}: {stderr}");
        assert!(stdout.contains("rnx"), "{shell} completion empty");
    }
}

#[test]
fn test_exact_filter_matches_whole_name() {
    let root = proj(
        "exact",
        &[
            ("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n"),
            (
                "tests/t.rnx",
                "test fn test_alpha() {\n    assert(true, \"a\");\n}\ntest fn test_alpha_beta() {\n    assert(true, \"b\");\n}\n",
            ),
        ],
    );
    let (code, text, _) = run(&["test", "test_alpha"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("test_alpha_beta"), "{text}");
    let root = proj(
        "exact2",
        &[
            ("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n"),
            (
                "tests/t.rnx",
                "test fn test_alpha() {\n    assert(true, \"a\");\n}\ntest fn test_alpha_beta() {\n    assert(true, \"b\");\n}\n",
            ),
        ],
    );
    let (code, text, _) = run(&["test", "--exact", "tests.t.test_alpha"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(code, 0, "{text}");
    assert!(!text.contains("test_alpha_beta"), "{text}");
    assert!(text.contains("tests: 1 passed, 0 failed in "), "{text}");
}

#[test]
fn test_lint_fix_renames_unused() {
    let root = proj(
        "fix",
        &[(
            "src/main.rnx",
            "fn Main(): Int {\n    let stray = 1;\n    return 0;\n}\n",
        )],
    );
    let (code, _, stderr) = run(&["lint", "--fix"], &root);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("Fixed 1"), "{stderr}");
    let text = std::fs::read_to_string(root.join("src").join("main.rnx")).unwrap();
    assert!(text.contains("_stray"), "{text}");
    let (code, _, stderr) = run(&["lint"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(code, 0, "{stderr}");
    assert!(stderr.contains("No lint issues"), "{stderr}");
}

#[test]
fn test_run_and_test_default_to_cranelift_backend() {
    let dir = std::env::temp_dir();
    let (_, stdout, stderr) = run(&["run", "--help"], &dir);
    let text = stdout + &stderr;
    assert!(text.contains("default: cranelift"), "{text}");
    let (_, stdout, stderr) = run(&["test", "--help"], &dir);
    let text = stdout + &stderr;
    assert!(text.contains("default: cranelift"), "{text}");
    let (_, stdout, stderr) = run(&["bench", "--help"], &dir);
    let text = stdout + &stderr;
    assert!(text.contains("default: llvm"), "{text}");
}

#[test]
fn test_build_output_flag_rejected() {
    let root = proj(
        "output-flag",
        &[("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n")],
    );
    let (code, _, stderr) = run(&["build", "-o", "foo"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(code, 2, "build -o must fail, got: {stderr}");
    assert!(stderr.contains("unexpected argument"), "{stderr}");
}

#[test]
fn test_quiet_suppresses_ok_line() {
    let root = proj(
        "quiet",
        &[("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n")],
    );
    let (_, _, stderr) = run(&["check"], &root);
    assert!(stderr.contains("(ok)"), "{stderr}");
    let (_, _, stderr) = run(&["check", "--quiet"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(!stderr.contains("(ok)"), "{stderr}");
}

#[test]
fn test_verbose_logs_context() {
    let root = proj(
        "verbose",
        &[("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n")],
    );
    let (_, _, stderr) = run(&["check", "--verbose"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(stderr.contains("rnx:"), "{stderr}");
}

#[test]
fn test_no_color_strips_escapes() {
    let root = proj(
        "nocolor",
        &[("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n")],
    );
    let (_, stdout, stderr) = run(&["--no-color", "check"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert!(!stdout.contains('\x1b'), "{stdout}");
    assert!(!stderr.contains('\x1b'), "{stderr}");
}
