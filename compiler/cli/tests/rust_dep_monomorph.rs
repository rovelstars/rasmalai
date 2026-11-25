#![cfg(all(unix, target_os = "linux"))]

#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::Path;
use std::process::Command;

fn run(command: &str, args: &[&str], dir: &Path) -> Option<String> {
    let output = Command::new(command).args(args).current_dir(dir).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[test]
fn native_generic_rust_function_is_monomorphized_with_stable_names() {
    let here = std::env::current_dir().unwrap();
    if run("git", &["--version"], &here).is_none() {
        return;
    }
    let dir = scratch::dir("rust-dep-monomorph");
    let crate_dir = dir.join("crate");
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"probe_dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        "/// Scales a value with a native trait bound.\npub fn scale<T: std::ops::Mul<Output = T> + Copy>(value: T, by: T) -> T { value * by }\n",
    )
    .unwrap();
    assert!(run("git", &["init", "-q"], &crate_dir).is_some());
    assert!(run("git", &["add", "."], &crate_dir).is_some());
    let commit = [
        "-c",
        "user.email=test@example.invalid",
        "-c",
        "user.name=test",
        "commit",
        "-qm",
        "init",
    ];
    assert!(run("git", &commit, &crate_dir).is_some());
    let rev = run("git", &["rev-parse", "HEAD"], &crate_dir)
        .unwrap()
        .trim()
        .to_string();

    let app = dir.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::write(
        app.join("Project.config"),
        format!(
            "export default {{\n    project: {{ name: \"probe-app\", version: \"0.1.0\" }},\n    entries: {{ main: \"src/main.rnx\" }},\n    permissions: [\"term:write\", \"unsafe:ffi\", \"unsafe:raw_memory\"],\n    dependencies: {{ probe: {{ rust: {{ git: \"file://{}\", rev: \"{}\" }}, package: \"probe_dep\", monomorphs: [[\"i32\"], [\"f64\"]] }} }}\n}}\n",
            crate_dir.display(),
            rev
        ),
    )
    .unwrap();
    std::fs::write(
        app.join("src/main.rnx"),
        "import { scaleI32, scaleF64 } from \"probe\"\n\nfn Main(): Int {\n    let a = scaleI32(6, 7)\n    let b = scaleF64(1.5, 2.0)\n    println!(a)\n    assert(b == 3.0, \"float monomorph\")\n    return a\n}\n",
    )
    .unwrap();

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let lock = Command::new(rnx).arg("lock").current_dir(&app).output().unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    let build = Command::new(rnx).arg("build").current_dir(&app).output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let exe = app.join(".rnx-cache/build/release/probe-app");
    let output = Command::new(&exe).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");

    let wrapper = std::fs::read_to_string(
        app.join(".rnx-cache/rust-bridge/wrappers/probe.rnx"),
    )
    .unwrap();
    assert!(wrapper.contains("export fn scaleI32(value: Int, by: Int): Int {"));
    assert!(wrapper.contains("export fn scaleF64(value: Float, by: Float): Float {"));
    let bridge = std::fs::read_to_string(
        app.join(".rnx-cache/rust-bridge/wrappers/probe.rnx"),
    )
    .unwrap();
    assert!(bridge.contains("fn rnx_probe_scaleF64("));
}
