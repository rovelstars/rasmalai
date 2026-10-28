#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

const CALC_SRC: &str = "pub fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n\npub fn isPositive(x: Int): Bool {\n    return x > 0;\n}\n\npub fn mulAdd(a: Float, b: Float, c: Float): Float {\n    return a * b + c;\n}\n\nfn helper(x: Int): Int {\n    return x * 2;\n}\n";

fn write_src(src: &str, tag: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
    let dir = crate::scratch::dir(&format!("static-{tag}"));
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
    let (_g, main) = write_src(CALC_SRC, "gen");
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
}

const HOST_C: &str = "#include <stdio.h>\n#include \"libcalc.h\"\nint main(void) {\n    if (add(40, 2) != 42) { return 1; }\n    if (!isPositive(5)) { return 2; }\n    if (isPositive(-3)) { return 3; }\n    double m = mulAdd(2.0, 3.0, 4.0);\n    if (m < 9.99 || m > 10.01) { return 4; }\n    printf(\"host ok\\n\");\n    return 0;\n}\n";

#[test]
fn test_host_c_program_linking() {
    let (_g, main) = write_src(CALC_SRC, "host");
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
}

#[test]
fn test_private_functions_not_exported() {
    let (_g, main) = write_src(CALC_SRC, "nm");
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
}

const BAD_SRC: &str = "class Box {\n    let v: Int;\n    init(v: Int) {\n        this.v = v;\n    }\n}\n\npub fn unbox(b: Box): Int {\n    return b.v;\n}\n";

#[test]
fn test_invalid_pub_signature_rejected() {
    let (_g, main) = write_src(BAD_SRC, "bad");
    let build = build_lib(&main);
    assert!(!build.status.success(), "class param accepted");
    let err = String::from_utf8_lossy(&build.stdout).into_owned()
        + &String::from_utf8_lossy(&build.stderr).into_owned();
    assert!(err.contains("E108"), "missing E108:\n{err}");
}

fn check_src(src: &str, tag: &str) -> std::process::Output {
    let (_g, main) = write_src(src, tag);
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let _ = _g;
    std::process::Command::new(rnx).arg("check").arg(&main).output().unwrap()
}

fn check_text(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout).into_owned()
        + &String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn test_check_time_rule_accepts_clean_exports() {
    let src = "/** Adds. */\nexport fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n\nexport fn flag(x: Bool): Bool {\n    return x;\n}\n\nexport fn f(x: Float): Float {\n    return x;\n}\n\nexport fn v(): Void {\n}\n\nexport fn s(data: Pointer<Byte>, data_len: Int): Int {\n    return data_len;\n}\n\nexport fn g(name: String): Int {\n    return 1;\n}\n";
    let out = check_src(src, "check-accept");
    assert!(out.status.success(), "{}", check_text(&out));
}

#[test]
fn test_check_time_rule_rejects_each_shape() {
    let shapes: &[(&str, &str, &[&str])] = &[
        ("async", "export async fn a(): Int {\n    return 0;\n}\n", &["E108", "`a`", "async"]),
        ("throws", "export fn t(): Int throws {\n    return 0;\n}\n", &["E108", "`t`", "throw"]),
        ("unann", "export fn u(x): Int {\n    return 0;\n}\n", &["E108", "`u`", "`x`"]),
        ("voidp", "export fn v(x: Void): Int {\n    return 0;\n}\n", &["E108", "`v`", "`x`"]),
        ("classp", "class Box {\n    let v: Int;\n    init(v: Int) {\n        this.v = v;\n    }\n}\n\nexport fn unbox(b: Box): Int {\n    return b.v;\n}\n", &["E108", "`unbox`", "`b`"]),
        ("strret", "export fn s(): String {\n    return \"x\";\n}\n", &["E108", "`s`"]),
        ("ptrret", "export fn p(): Pointer<Byte> {\n    unsafe {\n        return Pointer.fromAddress<Byte>(0);\n    }\n}\n", &["E108", "`p`"]),
        ("missret", "export fn m(x: Int) {\n    return;\n}\n", &["E108", "`m`"]),
        ("ptrnolen", "export fn r(data: Pointer<Byte>): Int {\n    return 0;\n}\n", &["E108", "`r`", "data_len"]),
        ("dup", "export fn d(x: Int): Int {\n    return x;\n}\n\nexport fn d(x: Int): Int {\n    return x;\n}\n", &["E108", "duplicate", "`d`"]),
    ];
    for (tag, src, wants) in shapes {
        let out = check_src(src, &format!("check-{tag}"));
        assert!(!out.status.success(), "{tag} accepted");
        let text = check_text(&out);
        for w in *wants {
            assert!(text.contains(w), "{tag} missing `{w}`:\n{text}");
        }
    }
}

fn write_project(tag: &str, manifest: &str, files: &[(&str, &str)]) -> (crate::scratch::Scratch, std::path::PathBuf) {
    let dir = crate::scratch::dir(&format!("static-{tag}"));
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), manifest).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    let root: std::path::PathBuf = dir.as_ref().to_path_buf();
    (dir, root)
}

fn artifact_of(out: &std::process::Output) -> String {
    String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find_map(|l| l.strip_prefix("artifact: "))
        .map(|s| s.trim().trim_end_matches(" (fresh)").to_string())
        .expect("build prints artifact path")
}

#[test]
fn test_entries_lib_used_when_different_path_passed() {
    let manifest = "export default {\n    project: {\n        name: \"calc\",\n        version: \"0.1.0\",\n    },\n    entries: {\n        lib: \"src/lib.rnx\",\n    },\n}\n";
    let (_g, root) = write_project(
        "entries-lib",
        manifest,
        &[
            ("src/lib.rnx", "export fn libOnly(x: Int): Int {\n    return x + 1;\n}\n"),
            ("src/other.rnx", "export fn otherOnly(x: Int): Int {\n    return x + 2;\n}\n"),
        ],
    );
    let _ = _g;
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(root.join("src").join("other.rnx"))
        .arg("--lib")
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = artifact_of(&build);
    let header = std::fs::read_to_string(std::path::Path::new(&bin).with_extension("h")).unwrap();
    assert!(header.contains("libOnly"), "lib entry missing:\n{header}");
    assert!(!header.contains("otherOnly"), "passed path leaked:\n{header}");
}

#[test]
fn test_lib_prefix_version_and_header_docs_golden() {
    let (_g, main) = write_src("/** Adds two ints. */\nexport fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n", "prefix");
    let _ = _g;
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("--lib")
        .arg("--lib-prefix")
        .arg("mycalc")
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = artifact_of(&build);
    let header = std::fs::read_to_string(std::path::Path::new(&bin).with_extension("h")).unwrap();
    assert!(header.contains("#define RNX_ABI_VERSION 1"), "version:\n{header}");
    assert!(header.contains("void rnx_init(int argc, const char *argv[]);"), "init:\n{header}");
    assert!(header.contains("const char *rnx_last_error(void);"), "last_error:\n{header}");
    assert!(header.contains("// Adds two ints."), "docs:\n{header}");
    assert!(header.contains("int64_t mycalc_add(int64_t a, int64_t b);"), "prefixed:\n{header}");
    assert!(!header.contains("int64_t add("), "bare leaked:\n{header}");
    let nm = std::process::Command::new("nm").arg(&bin).output().unwrap();
    assert!(nm.status.success());
    let text = String::from_utf8_lossy(&nm.stdout);
    assert!(text.lines().any(|l| l.contains(" T mycalc_add")), "nm:\n{text}");
    assert!(!text.lines().any(|l| l.contains(" T add")), "bare in nm:\n{text}");

    let bad = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("--lib")
        .arg("--lib-prefix")
        .arg("9bad")
        .output()
        .unwrap();
    assert!(!bad.status.success(), "bad prefix accepted");
    let err = String::from_utf8_lossy(&bad.stdout).into_owned()
        + &String::from_utf8_lossy(&bad.stderr).into_owned();
    assert!(err.contains("E108") && err.contains("9bad"), "prefix error:\n{err}");
}

const PTR_LIB_SRC: &str = "/** Sums borrowed bytes. */\nexport fn sumBytes(data: Pointer<Byte>, data_len: Int): Int {\n    let total = 0;\n    let i = 0;\n    while i < data_len {\n        unsafe {\n            let p = data + i;\n            total = total + p.read();\n        }\n        i = i + 1;\n    }\n    return total;\n}\n\nexport fn trapDiv(x: Int): Int {\n    return 100 / (x - 41);\n}\n\nexport fn thrower(flag: Int): Int {\n    if flag != 0 {\n        throw \"thrower-marker-7\";\n    }\n    return flag;\n}\n";

const PTR_HOST_C: &str = "#include <stdio.h>\n#include <string.h>\n#include <stdint.h>\n#include \"libptr.h\"\nint main(void) {\n    if (sumBytes((const uint8_t*)\"\", 0) != 0) { printf(\"empty FAIL\\n\"); return 1; }\n    const uint8_t bytes[] = {10, 20, 30, 40};\n    if (sumBytes(bytes, 4) != 100) { printf(\"sum FAIL\\n\"); return 2; }\n    if (trapDiv(43) != 50) { printf(\"div FAIL\\n\"); return 3; }\n    if (trapDiv(41) != INT64_MIN) { printf(\"trap sentinel FAIL\\n\"); return 4; }\n    if (strstr(rnx_last_error(), \"division\") == 0) { printf(\"trap msg FAIL: %s\\n\", rnx_last_error()); return 5; }\n    if (thrower(0) != 0) { printf(\"thrower0 FAIL\\n\"); return 6; }\n    if (thrower(1) != INT64_MIN) { printf(\"throw sentinel FAIL\\n\"); return 7; }\n    if (strstr(rnx_last_error(), \"thrower-marker-7\") == 0) { printf(\"throw msg FAIL: %s\\n\", rnx_last_error()); return 8; }\n    if (trapDiv(44) != 33) { printf(\"reuse FAIL\\n\"); return 9; }\n    if (rnx_last_error()[0] != '\\0') { printf(\"clear FAIL\\n\"); return 10; }\n    printf(\"host ok\\n\");\n    return 0;\n}\n";

#[test]
fn test_pointer_roundtrip_auto_init_and_containment() {
    let (_g, main) = write_src(PTR_LIB_SRC, "ptrhost");
    let _ = _g;
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx).arg("build").arg(&main).arg("--lib").output().unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = artifact_of(&build);
    let libdir = std::path::Path::new(&bin).parent().unwrap().to_path_buf();
    std::fs::write(libdir.join("host.c"), PTR_HOST_C).unwrap();
    std::fs::write(libdir.join("libptr.h"), std::fs::read(std::path::Path::new(&bin).with_extension("h")).unwrap()).unwrap();
    let cc = std::process::Command::new("cc")
        .arg(libdir.join("host.c"))
        .arg(libdir.join(std::path::Path::new(&bin).file_name().unwrap()))
        .arg("-lpthread")
        .arg("-ldl")
        .arg("-lm")
        .arg("-o")
        .arg(libdir.join("host_app"))
        .output()
        .unwrap();
    assert!(cc.status.success(), "{}", String::from_utf8_lossy(&cc.stderr));
    let run = std::process::Command::new(libdir.join("host_app")).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "host stdout:\n{}host stderr:\n{}", String::from_utf8_lossy(&run.stdout), String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "host ok\n");
}

#[test]
fn test_archives_byte_identical() {
    let (_g, main) = write_src(CALC_SRC, "det");
    let _ = _g;
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let first = std::process::Command::new(rnx).arg("build").arg(&main).arg("--lib").output().unwrap();
    assert!(first.status.success(), "{}", String::from_utf8_lossy(&first.stderr));
    let bin = artifact_of(&first);
    let a = std::fs::read(&bin).unwrap();
    let second = std::process::Command::new(rnx).arg("build").arg(&main).arg("--lib").output().unwrap();
    assert!(second.status.success(), "{}", String::from_utf8_lossy(&second.stderr));
    let bin2 = artifact_of(&second);
    let b = std::fs::read(&bin2).unwrap();
    assert_eq!(bin, bin2);
    assert_eq!(a, b, "archive not byte-identical across builds");
}
