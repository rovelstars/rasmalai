use std::path::PathBuf;

const MAIN_SRC: &str = "fn Main(): Int {\n    print(\"packed ok\");\n    return 0;\n}\n";

fn manifest(name: &str) -> String {
    format!(
        "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\",\n        description: \"Pack test.\"\n    }}\n}}\n"
    )
}

fn write_project(tag: &str, name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-pack-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), manifest(name)).unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), MAIN_SRC).unwrap();
    std::fs::write(dir.join("README.md"), format!("# {name}\n")).unwrap();
    dir
}

fn run_pack(dir: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.arg("pack").current_dir(dir);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

fn digest(path: &std::path::Path) -> String {
    frontend::checksum::Sha256::hexdigest(&std::fs::read(path).unwrap())
}

#[test]
fn test_pack_generates_archive_and_digest() {
    let dir = write_project("gen", "calc");
    let out = run_pack(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tar = dir.join("target").join("package").join("calc-0.1.0.tar");
    let sha = dir.join("target").join("package").join("calc-0.1.0.sha256");
    assert!(tar.is_file(), "archive missing");
    assert!(sha.is_file(), "digest missing");
    let text = std::fs::read_to_string(&sha).unwrap();
    assert_eq!(text, format!("{}  calc-0.1.0.tar\n", digest(&tar)), "digest mismatch");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_pack_deterministic_bytes() {
    let base = std::env::temp_dir().join(format!("rnx-packdet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for tag in ["a", "b"] {
        let dir = base.join(tag);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Project.config"), manifest("calc")).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), MAIN_SRC).unwrap();
    }
    for tag in ["a", "b"] {
        let out = run_pack(
            &base.join(tag),
            &["--out-dir", base.join(tag).join("out").to_str().unwrap()],
        );
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    let a = base.join("a").join("out").join("calc-0.1.0.tar");
    let b = base.join("b").join("out").join("calc-0.1.0.tar");
    assert_eq!(digest(&a), digest(&b), "digests differ");
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_pack_extract_and_run() {
    let dir = write_project("run", "calc");
    let out = run_pack(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tar = dir.join("target").join("package").join("calc-0.1.0.tar");
    let ext = dir.join("extracted");
    std::fs::create_dir_all(&ext).unwrap();
    let untar = std::process::Command::new("tar")
        .arg("-xf")
        .arg(&tar)
        .arg("-C")
        .arg(&ext)
        .output()
        .unwrap();
    assert!(untar.status.success(), "{}", String::from_utf8_lossy(&untar.stderr));
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").current_dir(&ext).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "packed ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_excluded_paths_not_in_tar() {
    let dir = write_project("exc", "calc");
    for d in [".git/refs", "target/foo", ".rnx/cache", ".rnx-cache/build", "tests"] {
        std::fs::create_dir_all(dir.join(d)).unwrap();
    }
    std::fs::write(dir.join(".git").join("HEAD"), "ref\n").unwrap();
    std::fs::write(dir.join("target").join("foo").join("x.o"), "obj").unwrap();
    std::fs::write(dir.join(".rnx").join("cache").join("y"), "cache").unwrap();
    std::fs::write(dir.join(".rnx-cache").join("build").join("stale.o"), "obj").unwrap();
    std::fs::write(dir.join("tests").join("t.rnx"), MAIN_SRC).unwrap();
    let out = run_pack(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let tar = dir.join("target").join("package").join("calc-0.1.0.tar");
    let list = std::process::Command::new("tar").arg("-tf").arg(&tar).output().unwrap();
    assert!(list.status.success());
    let text = String::from_utf8(list.stdout).unwrap();
    for line in text.lines() {
        let bad = line == ".git"
            || line.starts_with(".git/")
            || line == "target"
            || line.starts_with("target/")
            || line == ".rnx"
            || line.starts_with(".rnx/")
            || line == ".rnx-cache"
            || line.starts_with(".rnx-cache/")
            || line == "tests"
            || line.starts_with("tests/");
        assert!(!bad, "excluded entry packed: {line}");
    }
    assert!(!text.lines().any(|l| l == ".rnx/doc.json"), "generated doc.json packed");
    assert!(text.lines().any(|l| l == "Project.config"), "config missing:\n{text}");
    assert!(text.lines().any(|l| l == "src/main.rnx"), "source missing:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
