use std::path::PathBuf;

const SUM_SRC: &str = "import { blackBox } from \"@std/testing\";\n\nbench \"sum_1000\" {\n    let s = 0;\n    let i = 0;\n    while i < 1000 {\n        s = s + i;\n        i = i + 1;\n    }\n    blackBox(s);\n}\n\nfn Main(): Int {\n    return 0;\n}\n";

const PAIR_SRC: &str = "import { blackBox } from \"@std/testing\";\n\nbench \"fast_op\" {\n    blackBox(1);\n}\n\nbench \"slow_op\" {\n    let s = 0;\n    let i = 0;\n    while i < 100000 {\n        s = s + i;\n        i = i + 1;\n    }\n    blackBox(s);\n}\n\nfn Main(): Int {\n    return 0;\n}\n";

fn manifest(name: &str) -> String {
    format!(
        "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n"
    )
}

fn write_project(tag: &str, name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-bench-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), manifest(name)).unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), src).unwrap();
    dir
}

fn run_bench(dir: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.arg("bench").current_dir(dir);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

fn line_with<'a>(out: &'a str, name: &str) -> Option<&'a str> {
    out.lines().find(|l| l.starts_with(&format!("bench {name} ")))
}

#[test]
fn test_bench_execution_and_output() {
    let dir = write_project("exec", "b1", SUM_SRC);
    let out = run_bench(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    let line = line_with(&stdout, "sum_1000").expect(format!("no line:\n{stdout}").as_str());
    assert!(line.contains("ns/iter"), "bad line: {line}");
    assert!(line.contains("iters]"), "bad line: {line}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bench_filter() {
    let dir = write_project("filter", "b1", PAIR_SRC);
    let out = run_bench(&dir, &["--filter", "fast"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let stdout = String::from_utf8(out.stdout).unwrap();
    assert!(line_with(&stdout, "fast_op").is_some(), "fast missing:\n{stdout}");
    assert!(line_with(&stdout, "slow_op").is_none(), "slow leaked:\n{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bench_cross_backend() {
    let dir = write_project("backends", "b1", SUM_SRC);
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_bench(&dir, &["--backend", backend]);
        assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
        let stdout = String::from_utf8(out.stdout).unwrap();
        let line = line_with(&stdout, "sum_1000")
            .expect(format!("{backend} no line:\n{stdout}").as_str());
        assert!(line.contains("ns/iter"), "{backend} bad line: {line}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bench_stripped_in_regular_build() {
    let dir = write_project("strip", "b1", SUM_SRC);
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg("--release")
        .arg("-o")
        .arg(dir.join("app"))
        .current_dir(&dir)
        .arg("src/main.rnx")
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let nm = std::process::Command::new("nm").arg(dir.join("app")).output().unwrap();
    let text = String::from_utf8_lossy(&nm.stdout).into_owned()
        + &String::from_utf8_lossy(&nm.stderr).into_owned();
    assert!(!text.contains("__rnx_bench_"), "bench symbol leaked:\n{text}");
    let run = std::process::Command::new(dir.join("app")).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    let _ = std::fs::remove_dir_all(&dir);
}
