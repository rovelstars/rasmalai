#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn run(command: &str, args: &[&str], dir: &std::path::Path) -> Option<String> {
    let output = std::process::Command::new(command)
        .args(args)
        .current_dir(dir)
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    Some(String::from_utf8_lossy(&output.stdout).into_owned())
}

#[cfg(unix)]
#[test]
fn lock_resolves_a_local_rust_crate_into_the_rust_graph() {
    let here = std::env::current_dir().unwrap();
    let Some(git) = run("git", &["--version"], &here) else {
        return;
    };
    assert!(git.starts_with("git version"), "{git}");
    let base = scratch::dir("rust-dep-lock");
    let crate_dir = base.join("crate");
    std::fs::create_dir_all(crate_dir.join("src")).unwrap();
    std::fs::write(
        crate_dir.join("Cargo.toml"),
        "[package]\nname = \"probe_dep\"\nversion = \"0.1.0\"\nedition = \"2021\"\n",
    )
    .unwrap();
    std::fs::write(
        crate_dir.join("src/lib.rs"),
        "/// Adds two numbers.\npub fn add_numbers(a: i32, b: i32) -> i32 { a + b }\n",
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
            "export default {{\n    project: {{ name: \"probe-app\", version: \"0.1.0\" }},\n    entries: {{ main: \"src/main.rnx\" }},\n    dependencies: {{ probe: {{ rust: {{ git: \"file://{}\", rev: \"{}\" }}, package: \"probe_dep\" }} }}\n}}\n",
            crate_dir.display(),
            rev
        ),
    )
    .unwrap();
    std::fs::write(app.join("src/main.rnx"), "fn Main(): Int {\n    return 0\n}\n").unwrap();

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let lock = std::process::Command::new(rnx)
        .arg("lock")
        .current_dir(&app)
        .output()
        .unwrap();
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    let text = std::fs::read_to_string(app.join("Project.deplock")).unwrap();
    assert!(text.contains("rust: {"), "{text}");
    assert!(text.contains("name: \"probe_dep\""), "{text}");
    assert!(text.contains("cargoLockDigest: \"sha256:"), "{text}");
    assert!(text.contains("bridgeAbi: 1"), "{text}");

    let bridge = app.join(".rnx-cache/rust-bridge");
    let manifests: Vec<std::path::PathBuf> = std::fs::read_dir(&bridge)
        .unwrap()
        .map(|entry| entry.unwrap().path().join("Cargo.toml"))
        .filter(|path| path.is_file())
        .collect();
    assert_eq!(manifests.len(), 1);
    let manifest = std::fs::read_to_string(&manifests[0]).unwrap();
    assert!(manifest.contains("probe_dep = { git = \"file://"), "{manifest}");
    let source = std::fs::read_to_string(manifests[0].parent().unwrap().join("src/lib.rs")).unwrap();
    assert!(source.contains("pub mod probe {"), "{source}");
    assert!(source.contains("pub use ::probe_dep::*;"), "{source}");
    assert!(source.contains("#[global_allocator]"), "{source}");

    let check = std::process::Command::new(rnx)
        .arg("check")
        .current_dir(&app)
        .output()
        .unwrap();
    assert!(
        check.status.success(),
        "{}",
        String::from_utf8_lossy(&check.stderr)
    );

    unsafe {
        std::env::set_var("RNX_RUSTC_LLVM", "23.1");
        std::env::set_var("RNX_STRICT_RUST_LLVM", "1");
    }
    let strict = std::process::Command::new(rnx)
        .arg("lock")
        .current_dir(&app)
        .output()
        .unwrap();
    unsafe {
        std::env::remove_var("RNX_RUSTC_LLVM");
        std::env::remove_var("RNX_STRICT_RUST_LLVM");
    }
    assert!(!strict.status.success());
    let strict_text = String::from_utf8_lossy(&strict.stderr);
    assert!(
        strict_text.contains("rustc reports LLVM 23.1 but rnx uses LLVM 22.1"),
        "{strict_text}"
    );
}
