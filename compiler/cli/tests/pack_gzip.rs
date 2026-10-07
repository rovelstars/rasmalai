use std::path::PathBuf;

const MAIN_SRC: &str = "fn Main(): Int {\n    print(\"packed ok\");\n    return 0;\n}\n";

fn manifest(name: &str) -> String {
    format!(
        "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\",\n        description: \"Pack test.\"\n    }}\n}}\n"
    )
}

fn write_project(tag: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-gz-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), manifest(name)).unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), MAIN_SRC).unwrap();
    std::fs::write(dir.join("README.md"), "# calc\n").unwrap();
    dir
}

fn run_rnx(dir: &std::path::Path, args: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.current_dir(dir);
    for a in args {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

fn digest(path: &std::path::Path) -> String {
    frontend::checksum::Sha256::hexdigest(&std::fs::read(path).unwrap())
}

#[test]
fn test_pack_gzip_validity_with_system_tools() {
    let dir = write_project("sys", "calc");
    let out = run_rnx(&dir, &["pack", "--gzip"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let gz = dir.join("target").join("package").join("calc-0.1.0.tar.gz");
    let sha = dir.join("target").join("package").join("calc-0.1.0.tar.gz.sha256");
    assert!(gz.is_file(), "archive missing");
    assert!(sha.is_file(), "digest missing");
    let text = std::fs::read_to_string(&sha).unwrap();
    assert_eq!(text, format!("{}  calc-0.1.0.tar.gz\n", digest(&gz)));
    let tar = std::process::Command::new("tar").arg("-tzf").arg(&gz).output().unwrap();
    assert!(tar.status.success(), "{}", String::from_utf8_lossy(&tar.stderr));
    let listing = String::from_utf8(tar.stdout).unwrap();
    assert!(listing.lines().any(|l| l == "src/main.rnx"), "source missing:\n{listing}");
    let check = std::process::Command::new("gzip").arg("-t").arg(&gz).output().unwrap();
    assert!(check.status.success(), "gzip -t failed");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_pack_gzip_byte_determinism() {
    let base = std::env::temp_dir().join(format!("rnx-gzdet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for tag in ["a", "b"] {
        let dir = base.join(tag);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Project.config"), manifest("calc")).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), MAIN_SRC).unwrap();
    }
    for tag in ["a", "b"] {
        let out = run_rnx(
            &base.join(tag),
            &["pack", "--gzip", "--out-dir", base.join(tag).join("out").to_str().unwrap()],
        );
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    let a = base.join("a").join("out").join("calc-0.1.0.tar.gz");
    let b = base.join("b").join("out").join("calc-0.1.0.tar.gz");
    assert_eq!(digest(&a), digest(&b), "digests differ");
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_parallel_unpack_and_run() {
    let dir = write_project("up", "calc");
    let out = run_rnx(&dir, &["pack", "--gzip"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let gz = dir.join("target").join("package").join("calc-0.1.0.tar.gz");
    let ext = dir.join("target").join("extracted_app");
    let unpack = run_rnx(
        &dir,
        &["unpack", gz.to_str().unwrap(), "--out-dir", ext.to_str().unwrap()],
    );
    assert!(unpack.status.success(), "{}", String::from_utf8_lossy(&unpack.stderr));
    let stdout = String::from_utf8_lossy(&unpack.stdout).into_owned();
    assert!(stdout.contains("Extracted 3 files"), "summary:\n{stdout}");
    assert!(ext.join("src").join("main.rnx").is_file());
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").current_dir(&ext).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "packed ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

fn tar_bytes(name: &str, content: &[u8]) -> Vec<u8> {
    let mut head = [0u8; 512];
    head[0..name.len()].copy_from_slice(name.as_bytes());
    head[100..108].copy_from_slice(b"0000644\0");
    head[108..116].copy_from_slice(b"0000000\0");
    head[116..124].copy_from_slice(b"0000000\0");
    let size = format!("{:011o}\0", content.len());
    head[124..136].copy_from_slice(size.as_bytes());
    head[136..148].copy_from_slice(b"00000000000\0");
    for i in 148..156 {
        head[i] = b' ';
    }
    head[156] = b'0';
    head[257..263].copy_from_slice(b"ustar\0");
    head[263..265].copy_from_slice(b"00");
    head[265..268].copy_from_slice(b"rnx");
    head[297..300].copy_from_slice(b"rnx");
    let sum: u32 = head.iter().map(|b| *b as u32).sum();
    let text = format!("{sum:06o}");
    head[148..154].copy_from_slice(text.as_bytes());
    head[154] = 0;
    head[155] = b' ';
    let mut out = head.to_vec();
    out.extend(content);
    out.extend(vec![0u8; (512 - content.len() % 512) % 512]);
    out.extend(vec![0u8; 1024]);
    out
}

#[test]
fn test_unpack_path_traversal_safety() {
    let dir = write_project("evil", "calc");
    let bad = dir.join("evil.tar");
    std::fs::write(&bad, tar_bytes("../evil.txt", b"evil")).unwrap();
    let out = run_rnx(&dir, &["unpack", bad.to_str().unwrap()]);
    assert!(!out.status.success(), "traversal accepted");
    let err = String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(err.contains("E108"), "missing E108:\n{err}");
    assert!(!dir.join("evil.txt").exists(), "file escaped");
    let _ = std::fs::remove_dir_all(&dir);
}
