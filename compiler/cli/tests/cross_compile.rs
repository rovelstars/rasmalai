use std::path::PathBuf;

const MINI_SRC: &str = "fn Main(): Int { return 42; }\n";

fn write_src(src: &str, tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-cross-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("target")).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    (dir, main)
}

fn build(args: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    std::process::Command::new(rnx).arg("build").args(args).output().unwrap()
}

fn machine_of(path: &std::path::Path) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    assert!(bytes.len() > 20, "object too small");
    assert_eq!(&bytes[0..4], b"\x7fELF", "not an ELF object");
    u16::from_le_bytes([bytes[18], bytes[19]])
}

fn sha256(path: &std::path::Path) -> String {
    let out = std::process::Command::new("sha256sum").arg(path).output().unwrap();
    assert!(out.status.success(), "sha256sum failed");
    String::from_utf8(out.stdout).unwrap().split_whitespace().next().unwrap().to_string()
}

#[test]
fn test_emit_obj_x86_64() {
    let (dir, main) = write_src(MINI_SRC, "x86");
    let build = build(&[
        main.to_str().unwrap(),
        "--emit-obj",
        "--target",
        "x86_64-unknown-linux-gnu",
    ]);
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    assert!(std::path::Path::new(&bin).is_file(), "object missing");
    assert_eq!(machine_of(std::path::Path::new(&bin)), 62, "expected EM_X86_64");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_emit_obj_aarch64() {
    let (dir, main) = write_src(MINI_SRC, "arm");
    let build = build(&[
        main.to_str().unwrap(),
        "--emit-obj",
        "--target",
        "aarch64-unknown-linux-gnu",
    ]);
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    assert!(std::path::Path::new(&bin).is_file(), "object missing");
    assert_eq!(machine_of(std::path::Path::new(&bin)), 183, "expected EM_AARCH64");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_unsupported_target_rejected() {
    let (dir, main) = write_src(MINI_SRC, "bad");
    let build = build(&[
        main.to_str().unwrap(),
        "--target",
        "riscv64gc-unknown-linux-gnu",
    ]);
    assert!(!build.status.success(), "unsupported target accepted");
    let err = String::from_utf8_lossy(&build.stdout).into_owned()
        + &String::from_utf8_lossy(&build.stderr).into_owned();
    assert!(err.contains("E108"), "missing E108:\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_cross_target_determinism() {
    let (dir, main) = write_src(MINI_SRC, "det");
    let a = dir.join("target").join("a.o");
    let b = dir.join("target").join("b.o");
    for dst in [&a, &b] {
        let build = build(&[
            main.to_str().unwrap(),
            "--emit-obj",
            "--target",
            "aarch64-unknown-linux-gnu",
        ]);
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
        std::fs::copy(&bin, dst).unwrap();
        std::fs::remove_file(&bin).unwrap();
    }
    assert_eq!(sha256(&a), sha256(&b), "cross digests differ");
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}
