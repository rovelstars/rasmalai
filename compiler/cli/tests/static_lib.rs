use std::path::PathBuf;

const CALC_SRC: &str = "pub fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n\npub fn isPositive(x: Int): Bool {\n    return x > 0;\n}\n\npub fn mulAdd(a: Float, b: Float, c: Float): Float {\n    return a * b + c;\n}\n\nfn helper(x: Int): Int {\n    return x * 2;\n}\n";

fn write_src(src: &str, tag: &str) -> (PathBuf, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-static-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::create_dir_all(dir.join("target").join("test_lib")).unwrap();
    let main = dir.join("calc.rnx");
    std::fs::write(&main, src).unwrap();
    (dir, main)
}

fn build_lib(main: &std::path::Path) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    std::process::Command::new(rnx).arg("build").arg(main).arg("--lib").output().unwrap()
}

#[test]
fn test_static_library_and_header_generation() {
    let (dir, main) = write_src(CALC_SRC, "gen");
    let build = build_lib(&main);
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    assert!(std::path::Path::new(&bin).is_file(), "archive missing");
    let header = std::path::Path::new(&bin).with_extension("h");
    assert!(header.is_file(), "header missing");
    let text = std::fs::read_to_string(&header).unwrap();
    assert!(text.contains("#ifndef RNX_CALC_H"), "guard:\n{text}");
    assert!(text.contains("#include <stdint.h>"), "stdint:\n{text}");
    assert!(text.contains("int64_t add(int64_t a, int64_t b);"), "add:\n{text}");
    assert!(text.contains("bool isPositive(int64_t x);"), "isPositive:\n{text}");
    assert!(text.contains("double mulAdd(double a, double b, double c);"), "mulAdd:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

const HOST_C: &str = "#include <stdio.h>\n#include \"libcalc.h\"\nint main(void) {\n    if (add(40, 2) != 42) { return 1; }\n    if (!isPositive(5)) { return 2; }\n    if (isPositive(-3)) { return 3; }\n    double m = mulAdd(2.0, 3.0, 4.0);\n    if (m < 9.99 || m > 10.01) { return 4; }\n    printf(\"host ok\\n\");\n    return 0;\n}\n";

#[test]
fn test_host_c_program_linking() {
    let (dir, main) = write_src(CALC_SRC, "host");
    let build = build_lib(&main);
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let libdir = std::path::Path::new(&bin).parent().unwrap().to_path_buf();
    std::fs::write(libdir.join("main.c"), HOST_C).unwrap();
    std::fs::write(libdir.join("libcalc.h"), std::fs::read(std::path::Path::new(&bin).with_extension("h")).unwrap()).unwrap();
    let cc = std::process::Command::new("cc")
        .arg(libdir.join("main.c"))
        .arg(format!("-L{}", libdir.display()))
        .arg("-lcalc")
        .arg("-lpthread")
        .arg("-ldl")
        .arg("-lm")
        .arg("-o")
        .arg(libdir.join("host_app"))
        .output()
        .unwrap();
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    let run = std::process::Command::new(libdir.join("host_app")).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "host ok\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_private_functions_not_exported() {
    let (dir, main) = write_src(CALC_SRC, "nm");
    let build = build_lib(&main);
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let nm = std::process::Command::new("nm").arg(&bin).output().unwrap();
    assert!(nm.status.success());
    let text = String::from_utf8_lossy(&nm.stdout);
    let ours: Vec<&str> = text.lines().filter(|l| l.contains("add") || l.contains("isPositive") || l.contains("helper")).collect();
    assert!(ours.iter().any(|l| l.contains(" T add")), "add global:\n{}", ours.join("\n"));
    assert!(ours.iter().any(|l| l.contains(" T isPositive")), "isPositive global:\n{}", ours.join("\n"));
    assert!(!ours.iter().any(|l| l.contains(" T helper")), "helper leaked:\n{}", ours.join("\n"));
    let header = std::fs::read_to_string(std::path::Path::new(&bin).with_extension("h")).unwrap();
    assert!(!header.contains("helper"), "helper in header");
    let _ = std::fs::remove_dir_all(&dir);
}

const BAD_SRC: &str = "class Box {\n    let v: Int;\n    init(v: Int) {\n        this.v = v;\n    }\n}\n\npub fn unbox(b: Box): Int {\n    return b.v;\n}\n";

#[test]
fn test_invalid_pub_signature_rejected() {
    let (dir, main) = write_src(BAD_SRC, "bad");
    let build = build_lib(&main);
    assert!(!build.status.success(), "class param accepted");
    let err = String::from_utf8_lossy(&build.stdout).into_owned()
        + &String::from_utf8_lossy(&build.stderr).into_owned();
    assert!(err.contains("E108"), "missing E108:\n{err}");
    let _ = std::fs::remove_dir_all(&dir);
}
