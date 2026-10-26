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

#[test]
fn test_short_version_is_single_line() {
    let dir = crate::scratch::dir("version-short");
    let (code, stdout, _) = run(&["-V"], &dir);
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(stdout, format!("rnx {}\n", env!("CARGO_PKG_VERSION")));
}

#[test]
fn test_long_version_has_build_record() {
    let dir = crate::scratch::dir("version-long");
    let (code, stdout, _) = run(&["--version"], &dir);
    assert_eq!(code, 0, "{stdout}");
    let mut lines = stdout.lines();
    assert_eq!(
        lines.next().unwrap(),
        format!("rnx {}", env!("CARGO_PKG_VERSION")),
        "{stdout}"
    );
    for key in ["commit: ", "built: ", "install: ", "target: ", "llvm: "] {
        assert!(stdout.contains(key), "{stdout}");
    }
    let install = stdout
        .lines()
        .find_map(|l| l.strip_prefix("install: "))
        .expect("install line");
    assert!(
        install == "standalone" || install.starts_with("managed:"),
        "{stdout}"
    );
}
