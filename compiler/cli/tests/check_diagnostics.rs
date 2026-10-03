use std::path::{Path, PathBuf};
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-diag-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_package(dir: &Path, name: &str, files: &[(&str, &str)]) {
    std::fs::write(
        dir.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n"),
    )
    .unwrap();
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    for (file, contents) in files {
        std::fs::write(src.join(file), contents).unwrap();
    }
}

fn run_check(dir: &Path) -> std::process::Output {
    Command::new(rnx())
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .arg("src/main.rnx")
        .output()
        .unwrap()
}

fn run_build(dir: &Path) -> std::process::Output {
    Command::new(rnx())
        .arg("build")
        .env("NO_COLOR", "1")
        .current_dir(dir)
        .arg("src/main.rnx")
        .arg("-o")
        .arg(dir.join("out_bin"))
        .output()
        .unwrap()
}

#[test]
fn test_return_type_mismatch_reports_e304() {
    let dir = fresh_dir("ret");
    write_package(
        &dir,
        "retpkg",
        &[("main.rnx", "fn test(): Int { return \"nope\"; }\nfn Main(): Int { return 0; }\n")],
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E304"), "{stderr}");
    assert!(stderr.contains("expected `Int`, got `String`"), "{stderr}");
    assert!(stderr.contains("1:"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_undefined_variable_reports_e303() {
    let dir = fresh_dir("undef");
    write_package(
        &dir,
        "undefpkg",
        &[("main.rnx", "fn Main(): Int { let x = undefined_var + 1; return x; }\n")],
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stderr));
    let stderr = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(stderr.contains("E303"), "{stderr}");
    assert!(stderr.contains("undefined variable `undefined_var`"), "{stderr}");
    assert!(stderr.contains("1:"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_matching_return_type_passes_check() {
    let dir = fresh_dir("retok");
    write_package(
        &dir,
        "retokpkg",
        &[("main.rnx", "fn test(): Int { return 41 + 1; }\nfn Main(): Int { return test(); }\n")],
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_coalesce_non_nullable_is_noop_value() {
    let dir = fresh_dir("coal");
    write_package(
        &dir,
        "coalpkg",
        &[("main.rnx", "fn Main(): Int {\nlet x = 5;\nlet y = x ?? 3;\nassert(y == 5, \"non-null lhs keeps value\");\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_option_type_reports_e303_with_hint() {
    let dir = fresh_dir("optrem");
    write_package(
        &dir,
        "optrempkg",
        &[("main.rnx", "fn Main(): Int {\nlet x: Option<Int> = Option.None;\nreturn 0;\n}\n")],
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    assert!(text.contains("E303"), "{text}");
    assert!(text.contains("has been removed"), "{text}");
    assert!(text.contains("T?"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_some_none_values_report_e303() {
    let dir = fresh_dir("somenone");
    write_package(
        &dir,
        "somenonepkg",
        &[("main.rnx", "fn Main(): Int {\nlet x = Some(1);\nlet y = None;\nreturn 0;\n}\n")],
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    assert!(text.contains("E303"), "{text}");
    assert!(text.contains("has been removed"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_uninit_let_parses_and_const_still_e206() {
    let dir = fresh_dir("uninitparse");
    write_package(
        &dir,
        "uninitparsepkg",
        &[("main.rnx", "fn Main(): Int {\nlet a;\nlet b: Int?;\nlet c: Int;\nreturn 0;\n}\n")],
    );
    let out = run_check(&dir);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stdout));
    let dir2 = fresh_dir("constinit");
    write_package(
        &dir2,
        "constinitpkg",
        &[("main.rnx", "fn Main(): Int {\nconst b;\nreturn 0;\n}\n")],
    );
    let out2 = run_check(&dir2);
    assert_eq!(out2.status.code(), Some(1), "{}", String::from_utf8_lossy(&out2.stdout));
    let mut text = String::from_utf8_lossy(&out2.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out2.stdout));
    assert!(text.contains("E206"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
    let _ = std::fs::remove_dir_all(&dir2);
}

#[test]
fn test_coalesce_mismatched_fallback_reports_e108() {
    let dir = fresh_dir("coalm");
    write_package(
        &dir,
        "coalmpkg",
        &[("main.rnx", "fn Main(): Int {\nlet o: Int? = null;\nlet y = o ?? \"s\";\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stderr = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stderr.contains("E108"), "{stderr}");
    assert!(stderr.contains("`??` fallback must match"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_optchain_non_option_method_reports_e108() {
    let dir = fresh_dir("optc");
    write_package(
        &dir,
        "optcpkg",
        &[("main.rnx", "fn Main(): Int {\nlet x = 5;\nlet y = x?.foo;\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stderr = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stderr.contains("E108"), "{stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_operator_undefined_reports_e108() {
    let dir = fresh_dir("opundef");
    write_package(
        &dir,
        "opundefpkg",
        &[("main.rnx", "struct P {\nlet x: Int;\n}\nfn Main(): Int {\nlet a = P(1);\nlet b = P(2);\nlet c = a + b;\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("E108"), "{stdout}");
    assert!(stdout.contains("operator `+` is not defined for type `P`"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_for_non_iterable_reports_e108() {
    let dir = fresh_dir("foriter");
    write_package(
        &dir,
        "foriterpkg",
        &[("main.rnx", "class C {\nlet x: Int;\n}\nfn Main(): Int {\nlet c = new C();\nfor (v in c) {\n}\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("E108"), "{stdout}");
    assert!(stdout.contains("must implement `Iterable`"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_indexof_arity_reports_e108() {
    for (tag, call) in [("idx0", "\"abc\".indexOf()"), ("idx3", "\"abc\".indexOf(\"b\", 0, 1)")] {
        let dir = fresh_dir(tag);
        write_package(
            &dir,
            "idxpkg",
            &[("main.rnx", &format!("fn Main(): Int {{\nprint({call});\nreturn 0;\n}}\n"))],
        );
        let out = run_build(&dir);
        assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
        let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(stdout.contains("E108"), "{stdout}");
        assert!(stdout.contains("takes 1..2 args"), "{stdout}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

#[test]
fn test_switch_arms_type_mismatch_reports_e108() {
    let dir = fresh_dir("swarms");
    write_package(
        &dir,
        "swarmspkg",
        &[("main.rnx", "fn Main(): Int {\nlet x = switch 1 {\ncase 1: 10\ndefault: \"s\"\n};\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("E108"), "{stdout}");
    assert!(stdout.contains("`switch` arms yield `Int` and `String`"), "{stdout}");
    assert!(stdout.contains("2:"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_string_sub_reports_e108() {
    let dir = fresh_dir("strsub");
    write_package(
        &dir,
        "strsubpkg",
        &[("main.rnx", "fn Main(): Int {\nlet s = \"a\" - \"b\";\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("E108"), "{stdout}");
    assert!(stdout.contains("strings support `+` only"), "{stdout}");
    assert!(stdout.contains("2:"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_bitwise_float_reports_e108() {
    let dir = fresh_dir("bitfloat");
    write_package(
        &dir,
        "bitfloatpkg",
        &[("main.rnx", "fn Main(): Int {\nlet b = 1.5 & 2;\nreturn 0;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("E108"), "{stdout}");
    assert!(stdout.contains("bitwise operations need `Int` operands"), "{stdout}");
    assert!(stdout.contains("2:"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_unreachable_statement_reports_e108() {
    let dir = fresh_dir("unreach");
    write_package(
        &dir,
        "unreachpkg",
        &[("main.rnx", "fn Main(): Int {\nreturn 0;\nlet z = 1;\nreturn z;\n}\n")],
    );
    let out = run_build(&dir);
    assert_eq!(out.status.code(), Some(1), "{}", String::from_utf8_lossy(&out.stdout));
    let stdout = String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(stdout.contains("E108"), "{stdout}");
    assert!(stdout.contains("unreachable statement"), "{stdout}");
    assert!(stdout.contains("3:"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
