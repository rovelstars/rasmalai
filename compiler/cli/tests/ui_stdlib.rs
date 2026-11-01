use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .parent()
        .and_then(|p| p.parent())
        .map(|p| p.to_path_buf())
        .expect("repo root")
}

fn run(cwd: &PathBuf, args: &[&str]) -> std::process::Output {
    Command::new(rnx())
        .args(args)
        .current_dir(cwd)
        .env("NO_COLOR", "1")
        .env("RNX_VENDOR_DIR", repo().join("compiler/stdlib/src"))
        .output()
        .expect("run rnx")
}

#[test]
fn stdlib_ui_tests_pass() {
    let out = run(&repo().join("compiler/stdlib/src/ui"), &["test"]);
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}{stdout}");
    assert!(stdout.contains("0 failed"), "{stderr}{stdout}");
}

#[test]
fn ui_demo_runs_headless() {
    let out = run(
        &repo().join("examples/ui-demo"),
        &["run", "--backend", "interpreter"],
    );
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    assert!(out.status.success(), "{stderr}{stdout}");
    assert!(stdout.contains("button Reset"), "{stderr}{stdout}");
    assert!(stdout.contains("checkbox Reduce motion"), "{stderr}{stdout}");
    assert!(stdout.contains("query: theme"), "{stderr}{stdout}");
    assert!(stdout.contains("scheme after pointer: light"), "{stderr}{stdout}");
    assert!(stdout.contains("confirm: false"), "{stderr}{stdout}");
}
