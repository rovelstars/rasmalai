#![cfg(all(unix, target_os = "linux"))]

#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::Command;

fn run(command: &str, args: &[&str], dir: &Path) -> Option<String> {
    let output = Command::new(command).args(args).current_dir(dir).output().ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

struct Probe {
    _guard: scratch::Scratch,
    app: PathBuf,
    lock_stderr: String,
}

fn make_project(tag: &str) -> Option<Probe> {
    let here = std::env::current_dir().unwrap();
    if run("git", &["--version"], &here).is_none() {
        return None;
    }
    let base = scratch::dir(tag);
    let crate_dir = base.join("crate");
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"probe_dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[profile.release]\npanic = \"abort\"\n",
    )
    .unwrap();
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        "/// Adds two numbers.\n#[no_mangle]\npub extern \"C\" fn add_numbers(a: i32, b: i32) -> i32 { a + b }\n",
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
    let app = base.join("app");
    std::fs::create_dir_all(app.join("src")).unwrap();
    std::fs::write(
        app.join("Project.config"),
        format!(
            "export default {{\n    project: {{ name: \"probe-app\", version: \"0.1.0\" }},\n    entries: {{ main: \"src/main.rnx\" }},\n    permissions: [\"term:write\", \"unsafe:ffi\", \"unsafe:raw_memory\"],\n    dependencies: {{ probe: {{ rust: {{ git: \"file://{}\", rev: \"{}\" }}, package: \"probe_dep\" }} }}\n}}\n",
            crate_dir.display(),
            rev
        ),
    )
    .unwrap();
    std::fs::write(
        app.join("src/main.rnx"),
        "import { addNumbers } from \"probe\"\n\nfn Main(): Int {\n    let value = addNumbers(20, 22)\n    println!(value)\n    return value\n}\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let lock = Command::new(rnx).arg("lock").current_dir(&app).output().unwrap();
    assert!(lock.status.success(), "{}", String::from_utf8_lossy(&lock.stderr));
    Some(Probe {
        _guard: base,
        app,
        lock_stderr: String::from_utf8_lossy(&lock.stderr).into_owned(),
    })
}

fn check_probe(probe: &Probe, expect_internalized: bool) {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = Command::new(rnx).arg("build").current_dir(&probe.app).output().unwrap();
    assert!(
        build.status.success(),
        "build failed: {}",
        String::from_utf8_lossy(&build.stderr)
    );
    let exe = probe.app.join(".rnx-cache/build/release/probe-app");
    let output = Command::new(&exe).output().unwrap();
    assert_eq!(output.status.code(), Some(42), "{}", String::from_utf8_lossy(&output.stderr));
    assert_eq!(String::from_utf8_lossy(&output.stdout).trim(), "42");

    let bridge = probe.app.join(".rnx-cache/rust-bridge");
    let entries: Vec<_> = std::fs::read_dir(&bridge).unwrap().map(|e| e.unwrap().path()).collect();
    let built = entries
        .iter()
        .find(|dir| {
            dir.join("target")
                .join("x86_64-unknown-linux-gnu")
                .join("release")
                .join("librnx_rust_bridge.a")
                .is_file()
        })
        .expect("release staticlib artifact");
    let target = built.join("target").join("x86_64-unknown-linux-gnu").join("release");
    assert!(target.join("librnx_rust_bridge.so").is_file());
    assert_eq!(
        built.join("librnx_rust_bridge_internal.a").is_file(),
        expect_internalized
    );
    let bridge_source = std::fs::read_to_string(built.join("src/lib.rs")).unwrap();
    assert!(bridge_source.contains("pub unsafe extern \"C\" fn rnx_probe_add_numbers"));
    let wrapper = std::fs::read_to_string(built.join("wrappers/probe.rnx")).unwrap();
    assert!(wrapper.contains("as rustAddNumbers"));
    assert!(wrapper.contains("export fn addNumbers(a: Int, b: Int): Int {"));
}

#[test]
fn lto_build_links_bitcode_and_calls_the_rust_function() {
    let Some(probe) = make_project("rust-dep-lto") else {
        return;
    };
    check_probe(&probe, false);
}

#[test]
fn native_fallback_links_an_internalized_archive() {
    let Some(probe) = make_project("rust-dep-fallback") else {
        return;
    };
    unsafe { std::env::set_var("RNX_RUST_LTO", "0") };
    check_probe(&probe, true);
    unsafe { std::env::remove_var("RNX_RUST_LTO") };
}

#[test]
fn llvm_mismatch_warns_and_uses_native_fallback() {
    unsafe { std::env::set_var("RNX_RUSTC_LLVM", "23.1") };
    let probe = make_project("rust-dep-mismatch");
    let Some(probe) = probe else {
        unsafe { std::env::remove_var("RNX_RUSTC_LLVM") };
        return;
    };
    assert!(
        probe.lock_stderr.contains("rustc reports LLVM 23.1 but rnx uses LLVM 22.1"),
        "{}",
        probe.lock_stderr
    );
    check_probe(&probe, true);
    unsafe { std::env::remove_var("RNX_RUSTC_LLVM") };
}
