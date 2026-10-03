use std::path::PathBuf;

const MANIFEST: &str = "export default {\n    project: {\n        name: \"matrix\",\n        version: \"0.1.0\",\n        description: \"Toolchain audit package.\"\n    }\n}\n";

const MAIN_SRC: &str = "//! Matrix audit package.\nimport { AtomicInt, Mutex, Channel } from \"@std/sync\";\nimport { Vec4f, Vec4i } from \"@std/simd\";\nimport { blackBox } from \"@std/testing\";\n\n/// Adds two ints.\npub fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n\nclass Accum {\n    let total: Int = 0;\n    fn bump(x: Int): Int {\n        this.total = this.total + x;\n        return this.total;\n    }\n}\n\nfn worker(): Int {\n    let m = Mutex.byId(9101);\n    let c = AtomicInt.byId(9101);\n    let i = 0;\n    while i < 100 {\n        m.lock();\n        c.set(c.get() + 1);\n        m.unlock();\n        i = i + 1;\n    }\n    return 0;\n}\n\nfn Main(): Int {\n    let acc = new Accum();\n    acc.bump(20);\n    acc.bump(22);\n    let parts = [\"a\", \"b\"];\n    let s = \"x\" + parts[0] + \"-\" + parts[1];\n    AtomicInt.byId(9101).set(0);\n    let h1 = Thread.spawn(worker);\n    let h2 = Thread.spawn(worker);\n    h1.join();\n    h2.join();\n    let v = new Vec4f(1.0, 2.0, 3.0, 4.0) + new Vec4f(1.0, 1.0, 1.0, 1.0);\n    let w = new Vec4i(1, 2, 3, 4) * new Vec4i(1, 1, 1, 1);\n    let total = acc.bump(0) + Int(v.x()) + Int(v.y()) + Int(v.z()) + Int(v.w()) + w.get(0) + w.get(1) + w.get(2) + w.get(3);\n    let ch = Channel.byId(9102);\n    ch.send(7);\n    let got = ch.recv();\n    if AtomicInt.byId(9101).get() == 200 && total == 66 && got == 7 && parts.length == 2 {\n        print(\"matrix ok \" + s);\n        return 0;\n    }\n    print(\"matrix mismatch:\", total);\n    return 1;\n}\n\nbench \"vec add\" {\n    let v = new Vec4f(1.0, 2.0, 3.0, 4.0) + new Vec4f(1.0, 1.0, 1.0, 1.0);\n    blackBox(Int(v.x()));\n}\n\ntest fn test_accum_fresh() {\n    let a = new Accum();\n    assert(a.bump(0) == 0, \"fresh accum reads zero\");\n}\n\ntest fn test_pub_add() {\n    assert(add(40, 2) == 42, \"add math\");\n}\n";

const LIB_SRC: &str = "/// Adds two ints.\npub fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n\n/// Scales and shifts.\npub fn mulAdd(a: Float, b: Float, c: Float): Float {\n    return a * b + c;\n}\n";

const HOST_C: &str = "#include <stdio.h>\n#include \"libcalc.h\"\nint main(void) {\n    if (add(40, 2) != 42) { return 1; }\n    double m = mulAdd(2.0, 3.0, 4.0);\n    if (m < 9.99 || m > 10.01) { return 2; }\n    printf(\"host ok\\n\");\n    return 0;\n}\n";

fn write_matrix(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-matrix-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), MANIFEST).unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), MAIN_SRC).unwrap();
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

#[test]
fn test_full_subcommand_matrix_across_backends() {
    let dir = write_matrix("run");
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_rnx(&dir, &["run", "--backend", backend]);
        assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
        assert_eq!(out.status.code().unwrap(), 0, "{backend} exit");
        assert_eq!(
            String::from_utf8(out.stdout).unwrap(),
            "matrix ok xa-b\n",
            "{backend} stdout"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matrix_runner_green() {
    let dir = write_matrix("test");
    let out = run_rnx(&dir, &["test"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("tests: 2 passed, 0 failed in "), "count:\n{text}");
    assert!(text.contains("✓ test_accum_fresh"), "accum:\n{text}");
    assert!(text.contains("✓ test_pub_add"), "add:\n{text}");
    assert!(text.contains("tests: 2 passed, 0 failed in "), "summary:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matrix_bench_stats() {
    let dir = write_matrix("bench");
    let out = run_rnx(&dir, &["bench"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    let line = text
        .lines()
        .find(|l| l.starts_with("bench vec_add "))
        .expect(format!("no bench line:\n{text}").as_str());
    assert!(line.contains("ns/iter"), "no stats:\n{line}");
    assert!(line.contains("iters]"), "no iters:\n{line}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matrix_doc_site() {
    let dir = write_matrix("doc");
    let out = run_rnx(&dir, &["doc"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc = dir.join("target").join("doc");
    assert!(doc.join("index.html").is_file(), "index missing");
    assert!(doc.join("style.css").is_file(), "css missing");
    let index = std::fs::read_to_string(doc.join("index.html")).unwrap();
    assert!(index.contains("matrix"), "package name:\n{index}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matrix_pack_unpack_run() {
    let dir = write_matrix("pack");
    let out = run_rnx(&dir, &["pack", "--gzip"]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let gz = dir.join("target").join("package").join("matrix-0.1.0.tar.gz");
    let sha = dir.join("target").join("package").join("matrix-0.1.0.tar.gz.sha256");
    assert!(gz.is_file(), "archive missing");
    assert!(sha.is_file(), "digest missing");
    let digest = frontend::checksum::Sha256::hexdigest(&std::fs::read(&gz).unwrap());
    let text = std::fs::read_to_string(&sha).unwrap();
    assert_eq!(text, format!("{digest}  matrix-0.1.0.tar.gz\n"), "digest mismatch");
    let ext = dir.join("target").join("extracted");
    let unpack = run_rnx(&dir, &["unpack", gz.to_str().unwrap(), "--out-dir", ext.to_str().unwrap()]);
    assert!(unpack.status.success(), "{}", String::from_utf8_lossy(&unpack.stderr));
    assert!(ext.join("src").join("main.rnx").is_file(), "entry missing");
    let run = run_rnx(&ext, &["run"]);
    assert_eq!(run.status.code().unwrap(), 0, "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "matrix ok xa-b\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matrix_release_build() {
    let dir = write_matrix("release");
    let app = dir.join("target").join("matrix_app");
    std::fs::create_dir_all(app.parent().unwrap()).unwrap();
    let main = dir.join("src").join("main.rnx");
    let out = run_rnx(&dir, &["build", "--release", "-o", app.to_str().unwrap(), main.to_str().unwrap()]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(app.is_file(), "binary missing");
    let sections = std::process::Command::new("readelf")
        .arg("-S")
        .arg(&app)
        .output()
        .unwrap();
    assert!(sections.status.success(), "readelf failed");
    let text = String::from_utf8(sections.stdout).unwrap();
    assert!(!text.contains(".debug_info"), "debug leaked");
    let run = std::process::Command::new(&app).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "matrix ok xa-b\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matrix_static_lib_c_link() {
    let dir = std::env::temp_dir().join(format!("rnx-matrix-lib-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src = dir.join("calc.rnx");
    std::fs::write(&src, LIB_SRC).unwrap();
    let lib = dir.join("libcalc.a");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg("--lib")
        .arg(&src)
        .arg("-o")
        .arg(&lib)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    assert!(lib.is_file(), "archive missing");
    let header = lib.with_extension("h");
    assert!(header.is_file(), "header missing");
    let text = std::fs::read_to_string(&header).unwrap();
    assert!(text.contains("int64_t add(int64_t a, int64_t b);"), "add:\n{text}");
    std::fs::write(dir.join("main.c"), HOST_C).unwrap();
    std::fs::write(dir.join("libcalc.h"), std::fs::read(&header).unwrap()).unwrap();
    let cc = std::process::Command::new("cc")
        .arg(dir.join("main.c"))
        .arg(format!("-L{}", dir.display()))
        .arg("-lcalc")
        .arg("-lpthread")
        .arg("-ldl")
        .arg("-lm")
        .arg("-o")
        .arg(dir.join("host_app"))
        .output()
        .unwrap();
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    let run = std::process::Command::new(dir.join("host_app")).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "host ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matrix_cross_compile_aarch64() {
    let dir = write_matrix("cross");
    let obj = dir.join("target").join("matrix_arm.o");
    std::fs::create_dir_all(obj.parent().unwrap()).unwrap();
    let out = run_rnx(
        &dir,
        &[
            "build",
            "--emit-obj",
            "--target",
            "aarch64-unknown-linux-gnu",
            "-o",
            obj.to_str().unwrap(),
            dir.join("src").join("main.rnx").to_str().unwrap(),
        ],
    );
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    assert!(obj.is_file(), "object missing");
    let bytes = std::fs::read(&obj).unwrap();
    assert!(bytes.len() > 20, "object too small");
    assert_eq!(&bytes[0..4], b"\x7fELF", "not an ELF object");
    assert_eq!(u16::from_le_bytes([bytes[18], bytes[19]]), 183, "expected EM_AARCH64");
    let _ = std::fs::remove_dir_all(&dir);
}
