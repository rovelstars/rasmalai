#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

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

fn proj(tag: &str, files: &[(&str, &str)]) -> crate::scratch::Scratch {
    let root = crate::scratch::dir(&format!("clap-{tag}"));
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
    let dir = crate::scratch::dir("clap-help");
    let (code, stdout, _) = run(&["--help"], &dir);
    assert_eq!(code, 0, "{stdout}");
    assert!(stdout.contains("Rasmalai systems language"), "{stdout}");
    assert!(stdout.contains("completions"), "{stdout}");
}

#[test]
fn test_fetch_std_removed() {
    let dir = crate::scratch::dir("clap-nofetchstd");
    let (code, _, stderr) = run(&["fetch-std"], &dir);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("fetch-std"), "{stderr}");
}

#[test]
fn test_download_help_names_packages_and_local() {
    let dir = crate::scratch::dir("clap-download");
    let (_, stdout, _) = run(&["download", "--help"], &dir);
    assert!(stdout.contains("name@range"), "{stdout}");
    assert!(stdout.contains("--local"), "{stdout}");
    assert!(stdout.contains("@std"), "{stdout}");
}

#[test]
fn test_vendor_help_names_dir_default() {
    let dir = crate::scratch::dir("clap-vendor");
    let (_, stdout, _) = run(&["vendor", "--help"], &dir);
    assert!(stdout.contains("--dir"), "{stdout}");
    assert!(stdout.contains(".vendor"), "{stdout}");
}

#[test]
fn test_typo_suggests_subcommand() {
    let dir = crate::scratch::dir("clap-typo");
    let (code, _, stderr) = run(&["chck"], &dir);
    assert_eq!(code, 2, "{stderr}");
    assert!(stderr.contains("check"), "{stderr}");
}

#[test]
fn test_completions_for_all_shells() {
    let dir = crate::scratch::dir("clap-completions");
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
fn test_session_commands_hide_engine_selection() {
    let dir = crate::scratch::dir("clap-defaults");
    for cmd in ["run", "test", "bench"] {
        let (_, stdout, stderr) = run(&[cmd, "--help"], &dir);
        let text = stdout + &stderr;
        assert!(!text.contains("--backend"), "{cmd} leaks backend selection:\n{text}");
        assert!(!text.contains("cranelift"), "{cmd} leaks an engine name:\n{text}");
        assert!(!text.contains("llvm"), "{cmd} leaks an engine name:\n{text}");
    }
    let (_, stdout, stderr) = run(&["bench", "--help"], &dir);
    let text = stdout + &stderr;
    assert!(!text.contains("--release"), "bench leaks a profile flag:\n{text}");
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

#[test]
fn piped_test_output_stays_line_based() {
    let root = proj(
        "pipedtest",
        &[
            ("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n"),
            (
                "tests/t.rnx",
                "test fn test_ok() {\n    assert(true, \"ok\");\n}\n",
            ),
        ],
    );
    let (code, stdout, stderr) = run(&["test"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("tests: 1 passed, 0 failed in "), "{stdout}");
    assert!(!stdout.contains('\x1b'), "piped stdout carried escapes: {stdout:?}");
    assert!(!stderr.contains('\x1b'), "piped stderr carried escapes: {stderr:?}");
    assert!(!stdout.contains('\r'), "piped stdout carried cursor returns: {stdout:?}");
}

#[cfg(target_os = "linux")]
#[test]
fn tty_test_output_redraws_a_live_block() {    if std::process::Command::new("script")
        .arg("--version")
        .output()
        .is_err()
    {
        eprintln!("skip: util-linux script is unavailable");
        return;
    }
    let root = proj(
        "ttytest",
        &[
            ("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n"),
            (
                "tests/t.rnx",
                "test fn test_ok() {\n    assert(true, \"ok\");\n}\n",
            ),
        ],
    );
    let command = format!("{} test", rnx());
    let out = std::process::Command::new("script")
        .args(["-qec", &command, "/dev/null"])
        .current_dir(root.path())
        .env("TERM", "xterm-256color")
        .env_remove("NO_COLOR")
        .output()
        .unwrap();
    let _ = std::fs::remove_dir_all(&root);
    let text = String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(
        out.status.success(),
        "status {:?}\n{text}",
        out.status
    );
    assert!(
        text.contains("tests: ")
            && text.contains("passed")
            && text.contains("failed in "),
        "{text}"
    );
    assert!(text.contains("\x1b["), "no color on a TTY: {text:?}");
    assert!(
        text.contains("\x1b[J") || text.contains("\x1b[2K"),
        "no live block redraw: {text:?}"
    );
}

#[test]
fn forced_color_keeps_a_pipe_line_based() {
    let root = proj(
        "forcedcolor",
        &[
            ("src/main.rnx", "fn Main(): Int {\n    return 0;\n}\n"),
            (
                "tests/t.rnx",
                "test fn test_ok() {\n    assert(true, \"ok\");\n}\n",
            ),
        ],
    );
    let (code, stdout, stderr) = run(&["--color=always", "test"], &root);
    let _ = std::fs::remove_dir_all(&root);
    assert_eq!(code, 0, "{stdout}{stderr}");
    assert!(stdout.contains("\x1b["), "forced color missing: {stdout:?}");
    assert!(
        !stdout.contains("\x1b[J") && !stdout.contains("\x1b[2K"),
        "cursor control leaked into a pipe: {stdout:?}"
    );
    assert!(!stdout.contains('\r'), "carriage return leaked: {stdout:?}");
}

#[test]
fn terminal_color_policy_is_explicit() {
    let dir = crate::scratch::dir("clap-color");
    let exec = |args: &[&str], envs: &[(&str, &str)]| {
        let mut cmd = std::process::Command::new(rnx());
        cmd.args(args).current_dir(dir.path());
        cmd.env_remove("NO_COLOR").env_remove("CLICOLOR").env_remove("CLICOLOR_FORCE").env_remove("FORCE_COLOR").env("TERM", "xterm-256color");
        for (k, v) in envs {
            cmd.env(k, v);
        }
        let out = cmd.output().unwrap();
        (
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    };
    let (stdout, stderr) = exec(&["--color=always", "explain", "E108"], &[]);
    assert!(stdout.contains("\x1b["), "{stdout}{stderr}");
    let (stdout, _) = exec(&["--color=never", "explain", "E108"], &[]);
    assert!(!stdout.contains('\x1b'), "{stdout}");
    let (stdout, _) = exec(&["explain", "E108"], &[("NO_COLOR", "1")]);
    assert!(!stdout.contains('\x1b'), "{stdout}");
    let (stdout, _) = exec(&["explain", "E108"], &[("TERM", "dumb")]);
    assert!(!stdout.contains('\x1b'), "{stdout}");
    let (stdout, _) = exec(&["explain", "E108"], &[("CLICOLOR_FORCE", "1")]);
    assert!(stdout.contains("\x1b["), "{stdout}");
}
