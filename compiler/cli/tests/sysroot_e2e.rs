#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn run(args: &[&str], dir: &std::path::Path, sysroot: Option<&std::path::Path>) -> (i32, String, String) {
    let mut cmd = std::process::Command::new(rnx());
    cmd.args(args).current_dir(dir);
    match sysroot {
        Some(root) => {
            cmd.env("RNX_SYSROOT", root);
        }
        None => {
            cmd.env_remove("RNX_SYSROOT");
        }
    }
    cmd.env_remove("RNX_VENDOR_DIR");
    cmd.env("RNX_REGISTRY", "http://127.0.0.1:9");
    let out = cmd.output().unwrap();
    (
        out.status.code().unwrap(),
        String::from_utf8(out.stdout).unwrap(),
        String::from_utf8(out.stderr).unwrap(),
    )
}

fn sysroot_with_prelude(tag: &str) -> (crate::scratch::Scratch, std::path::PathBuf, String) {
    let dir = crate::scratch::dir(&format!("sysroot-{tag}"));
    let root: std::path::PathBuf = dir.as_ref().to_path_buf();
    let std_dir = dir.join("share").join("rnx").join("std");
    std::fs::create_dir_all(&std_dir).unwrap();
    let version = env!("CARGO_PKG_VERSION");
    std::fs::write(
        dir.join("sysroot.json"),
        format!("{{\"version\": 1, \"std_version\": \"{version}\"}}"),
    )
    .unwrap();
    let mut checkout = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    checkout.pop();
    checkout.push("stdlib");
    checkout.push("src");
    checkout.push("prelude");
    checkout.push("lib.rnx");
    let mut custom = std::fs::read_to_string(&checkout).expect("checkout prelude must exist");
    custom.push_str("\nexport fn sysrootMarker(): Int {\n    return 42;\n}\n");
    std::fs::write(std_dir.join("prelude.rnx"), &custom).unwrap();
    std::fs::write(
        std_dir.join("bareonly.rnx"),
        "export fn bareValue(): Int {\n    return 7;\n}\n",
    )
    .unwrap();
    (dir, root, custom)
}

fn proj(tag: &str, main: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
    let root = crate::scratch::dir(&format!("sysroot-proj-{tag}"));
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: {\n        name: \"sysroot-probe\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let src = root.join("src").join("main.rnx");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, main).unwrap();
    (root, src)
}

fn artifact_of(stdout: &str) -> std::path::PathBuf {
    let bin = stdout
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("artifact: "))
        .map(|s| s.trim().trim_end_matches(" (fresh)").to_string())
        .expect("build prints artifact path");
    std::path::PathBuf::from(bin)
}

#[test]
fn test_print_sysroot_reports_host_and_custom() {
    let dir = crate::scratch::dir("sysroot-print");
    let (code, stdout, _) = run(&["--print", "sysroot"], &dir, None);
    assert_eq!(code, 0, "{stdout}");
    assert!(!stdout.trim().is_empty(), "host sysroot must print a root");
    let (_sys, root, _) = sysroot_with_prelude("print");
    let (code, stdout, _) = run(&["--print", "sysroot"], &dir, Some(&root));
    assert_eq!(code, 0, "{stdout}");
    assert_eq!(stdout.trim(), root.to_string_lossy().as_ref());
    let (code, _, stderr) = run(&["--print", "bogus"], &dir, None);
    assert_eq!(code, 2, "{stderr}");
    let missing = dir.join("empty");
    std::fs::create_dir_all(&missing).unwrap();
    let (code, _, stderr) = run(&["--print", "sysroot"], &dir, Some(&missing));
    assert_eq!(code, 1, "{stderr}");
    assert!(stderr.contains("sysroot.json"), "{stderr}");
}

#[test]
fn test_custom_prelude_builds_and_runs() {
    let (_sys, root, _) = sysroot_with_prelude("e2e");
    let (_proj, main) = proj(
        "prelude",
        "import { sysrootMarker } from \"@std/prelude\";\n\nfn Main(): Int {\n    return sysrootMarker();\n}\n",
    );
    let (code, stdout, stderr) = run(&["build", main.to_str().unwrap()], &_proj, Some(&root));
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let bin = artifact_of(&stdout);
    assert!(bin.is_file(), "artifact missing: {}", bin.display());
    let exec = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(exec.status.code(), Some(42), "custom prelude marker must run");
}

#[test]
fn test_sysroot_only_module_builds_and_runs() {
    let (_sys, root, _) = sysroot_with_prelude("bare");
    let (_proj, main) = proj(
        "bareonly",
        "import { bareValue } from \"@std/bareonly\";\n\nfn Main(): Int {\n    return bareValue();\n}\n",
    );
    let (code, stdout, stderr) = run(&["build", main.to_str().unwrap()], &_proj, Some(&root));
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let bin = artifact_of(&stdout);
    let exec = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(exec.status.code(), Some(7), "sysroot-only module must run");
}

#[test]
fn test_broken_sysroot_fails_closed() {
    let dir = crate::scratch::dir("sysroot-broken");
    let (_proj, main) = proj("broken", "fn Main(): Int {\n    return 0;\n}\n");
    let (code, _, stderr) = run(&["build", main.to_str().unwrap()], &_proj, Some(&dir));
    assert_ne!(code, 0, "broken sysroot must fail the build");
    let combined = stderr.clone();
    assert!(
        combined.contains("invalid sysroot") || combined.contains("sysroot.json"),
        "{combined}"
    );
}

#[test]
fn test_musl_emit_obj_and_link_gate() {
    let dir = crate::scratch::dir("sysroot-musl-cli");
    let (_proj, main) = proj("musl", "fn Main(): Int {\n    return 42;\n}\n");
    let (code, stdout, stderr) = run(
        &["build", main.to_str().unwrap(), "--emit-obj", "--target", "x86_64-unknown-linux-musl"],
        &_proj,
        None,
    );
    assert_eq!(code, 0, "stdout: {stdout}\nstderr: {stderr}");
    let obj = artifact_of(&stdout);
    let bytes = std::fs::read(&obj).unwrap();
    assert_eq!(&bytes[0..4], b"\x7fELF", "not an ELF object");
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 62, "expected EM_X86_64");
    let (code, _, stderr) = run(
        &["build", main.to_str().unwrap(), "--target", "x86_64-unknown-linux-musl"],
        &_proj,
        None,
    );
    assert_ne!(code, 0, "musl link without musl libraries must fail");
    assert!(
        stderr.contains("x86_64-unknown-linux-musl"),
        "link failure must name the target: {stderr}"
    );
    let _ = dir;
}
