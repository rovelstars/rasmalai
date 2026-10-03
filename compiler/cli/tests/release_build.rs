use std::path::PathBuf;

fn write_src(src: &str, tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-rel-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    (dir.clone(), main)
}

fn build_release(main: &std::path::Path, out: &std::path::Path) {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(main)
        .arg("--release")
        .arg("-o")
        .arg(out)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
}

fn build_debug(main: &std::path::Path, out: &std::path::Path) {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(main)
        .arg("-o")
        .arg(out)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
}

fn sha256(path: &std::path::Path) -> String {
    let out = std::process::Command::new("sha256sum").arg(path).output().unwrap();
    assert!(out.status.success(), "sha256sum failed");
    String::from_utf8(out.stdout).unwrap().split_whitespace().next().unwrap().to_string()
}

const EXEC_SRC: &str = "import { AtomicInt } from \"@std/sync\";\n\nfn Main(): Int {\n    let a = AtomicInt.byId(810);\n    a.set(40);\n    a.fetchAdd(2);\n    let n = a.get();\n    print(\"release:\", n);\n    if n == 42 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_release_build_execution() {
    let (dir, main) = write_src(EXEC_SRC, "exec");
    let out = dir.join("app_release");
    build_release(&main, &out);
    let run = std::process::Command::new(&out).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "release: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

const MINI_SRC: &str = "fn Main(): Int { return 42; }\n";

fn ldd_needed(path: &std::path::Path) -> String {
    let out = std::process::Command::new("ldd").arg(path).output().unwrap();
    assert!(out.status.success(), "ldd failed");
    String::from_utf8(out.stdout).unwrap()
}

#[test]
fn test_dev_dynamic_release_static() {
    let (dir, main) = write_src(MINI_SRC, "size");
    let dbg = dir.join("app_debug");
    let rel = dir.join("app_release");
    build_debug(&main, &dbg);
    build_release(&main, &rel);
    let dbg_size = std::fs::metadata(&dbg).unwrap().len();
    let rel_size = std::fs::metadata(&rel).unwrap().len();
    assert!(
        dbg_size < rel_size,
        "dynamic dev {dbg_size} should be smaller than static release {rel_size}"
    );
    assert!(
        ldd_needed(&dbg).contains("libruntime_native.so"),
        "dev binary must dynamically link libruntime_native"
    );
    assert!(
        !ldd_needed(&rel).contains("libruntime_native"),
        "release binary must stay fully static"
    );
    let run = std::process::Command::new(&rel).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    let run_dbg = std::process::Command::new(&dbg).output().unwrap();
    assert_eq!(run_dbg.status.code().unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bit_identical_reproducibility() {
    let (dir, main) = write_src(EXEC_SRC, "repro");
    let a = dir.join("app_a");
    let b = dir.join("app_b");
    build_release(&main, &a);
    build_release(&main, &b);
    let ha = sha256(&a);
    let hb = sha256(&b);
    assert_eq!(ha, hb, "release digests differ");
    assert_eq!(std::fs::read(&a).unwrap(), std::fs::read(&b).unwrap());
    let _ = std::fs::remove_dir_all(&dir);
}

const MATRIX_SRC: &str = "import { AtomicInt, Mutex } from \"@std/sync\";\nimport { Vec4f } from \"@std/simd\";\n\nfn task() {\n    let m = Mutex.byId(801);\n    m.lock();\n    AtomicInt.byId(800).fetchAdd(1);\n    m.unlock();\n}\n\nfn Main(): Int {\n    let d = Int(new Vec4f(1.0, 2.0, 3.0, 4.0).dot(new Vec4f(2.0, 3.0, 4.0, 5.0)));\n    let pool = ThreadPool.byId(806, 4);\n    let i = 0;\n    while i < 8 {\n        pool.submit(task);\n        i = i + 1;\n    }\n    pool.join();\n    pool.shutdown();\n    let n = AtomicInt.byId(800).get();\n    print(\"release matrix:\", d, n);\n    if d == 40 && n == 8 {\n        return 0;\n    }\n    return 1;\n}\n";

#[test]
fn test_release_runtime_matrix() {
    let (dir, main) = write_src(MATRIX_SRC, "matrix");
    let out = dir.join("app_release");
    build_release(&main, &out);
    let run = std::process::Command::new(&out).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "release matrix: 40 8\n");
    let _ = std::fs::remove_dir_all(&dir);
}
