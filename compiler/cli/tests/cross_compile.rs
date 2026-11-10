#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;
#[path = "../../testkit/pack.rs"]
mod pack_fixture;

const MINI_SRC: &str = "fn Main(): Int { return 42; }\n";

fn write_src(src: &str, tag: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
    let dir = crate::scratch::dir(&format!("cross-{tag}"));
    std::fs::create_dir_all(dir.join("target")).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    (dir, main)
}

fn build(args: &[&str]) -> std::process::Output {
    build_in(args, None)
}

fn build_in(
    args: &[&str],
    sysroot: Option<&std::path::Path>,
) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let cache = crate::scratch::dir("cross-cache");
    let mut cmd = std::process::Command::new(rnx);
    cmd.arg("build")
        .args(args)
        .env("RNX_CACHE_HOME", cache.path());
    match sysroot {
        Some(root) => {
            cmd.env("RNX_SYSROOT", root);
        }
        None => {
            cmd.env_remove("RNX_SYSROOT");
        }
    }
    cmd.output().unwrap()
}

fn pack(tag: &str, triple: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
    let dir = crate::scratch::dir(&format!("cross-pack-{tag}"));
    crate::pack_fixture::write_pack(&dir, triple, "cross");
    let path = dir.path().to_path_buf();
    (dir, path)
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
    let (_g, main) = write_src(MINI_SRC, "x86");
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
}

#[test]
fn test_emit_obj_aarch64() {
    let (_g, main) = write_src(MINI_SRC, "arm");
    let (_pack, sysroot) = pack("arm", "aarch64-unknown-linux-gnu");
    let build = build_in(
        &[
            main.to_str().unwrap(),
            "--emit-obj",
            "--target",
            "aarch64-unknown-linux-gnu",
        ],
        Some(&sysroot),
    );
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    assert!(std::path::Path::new(&bin).is_file(), "object missing");
    assert_eq!(machine_of(std::path::Path::new(&bin)), 183, "expected EM_AARCH64");
}

#[test]
fn test_unsupported_target_rejected() {
    let (_g, main) = write_src(MINI_SRC, "bad");
    let build = build(&[
        main.to_str().unwrap(),
        "--target",
        "riscv64gc-unknown-linux-gnu",
    ]);
    assert!(!build.status.success(), "unsupported target accepted");
    let err = String::from_utf8_lossy(&build.stdout).into_owned()
        + &String::from_utf8_lossy(&build.stderr).into_owned();
    assert!(err.contains("E108"), "missing E108:\n{err}");
}

#[test]
fn test_cross_target_determinism() {
    let (dir, main) = write_src(MINI_SRC, "det");
    let a = dir.join("target").join("a.o");
    let b = dir.join("target").join("b.o");
    let (_pack, sysroot) = pack("det", "aarch64-unknown-linux-gnu");
    for dst in [&a, &b] {
        let build = build_in(
            &[
                main.to_str().unwrap(),
                "--emit-obj",
                "--target",
                "aarch64-unknown-linux-gnu",
            ],
            Some(&sysroot),
        );
        assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
        let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
        std::fs::copy(&bin, dst).unwrap();
        std::fs::remove_file(&bin).unwrap();
    }
    assert_eq!(sha256(&a), sha256(&b), "cross digests differ");
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
}
