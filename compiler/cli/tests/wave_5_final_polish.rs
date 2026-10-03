use std::path::PathBuf;
use std::process::Command;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn fresh_dir(tag: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-wave5-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    dir
}

fn write_file(path: &PathBuf, contents: &str) {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).unwrap();
    }
    std::fs::write(path, contents).unwrap();
}

fn run_backend(file: &PathBuf, backend: &str, extra: &[&str]) -> std::process::Output {
    let mut cmd = Command::new(rnx());
    cmd.arg("run").arg("--backend").arg(backend).arg(file);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

const CONST_SHARE_SRC: &str = "const dbPath = \"notes.txt\";\nconst retries = 3;\nfn load(): String {\n    return dbPath;\n}\nfn attempts(): Int {\n    return retries + 1;\n}\nfn Main(): Int {\n    print(load());\n    print(attempts());\n    print(dbPath.length());\n    return 0;\n}\n";

#[test]
fn module_const_visible_in_named_fn_on_all_backends() {
    let dir = fresh_dir("const");
    let file = dir.join("share.rnx");
    write_file(&file, CONST_SHARE_SRC);
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend(&file, backend, &[]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{backend}: exit, stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout.clone()).unwrap(),
            "notes.txt\n4\n9\n",
            "{backend}: const reads identically"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

const TOPLET_SRC: &str = "let n = 3;\nfn f(): Int { return n; }\nprint(f());\n";

#[test]
fn cross_function_let_read_names_sharing() {
    let dir = fresh_dir("toplet");
    let file = dir.join("toplet.rnx");
    write_file(&file, TOPLET_SRC);
    let out = Command::new(rnx()).arg("check").arg(&file).output().unwrap();
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stderr).into_owned();
    assert!(text.contains("E303"), "got:\n{text}");
    assert!(
        text.contains("share the value with `const`"),
        "hint must name the sharing fix, got:\n{text}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const RANGE_METHOD_SRC: &str = "fn Main(): Int {\n    for i in 0 .. 4 {\n        print(i.toString());\n    }\n    return 0;\n}\n";

#[test]
fn range_loop_binding_has_int_methods_on_all_backends() {
    let dir = fresh_dir("range");
    let file = dir.join("range.rnx");
    write_file(&file, RANGE_METHOD_SRC);
    for backend in ["interpreter", "cranelift", "llvm"] {
        let out = run_backend(&file, backend, &[]);
        assert_eq!(
            out.status.code(),
            Some(0),
            "{backend}: exit, stderr: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        assert_eq!(
            String::from_utf8(out.stdout.clone()).unwrap(),
            "0\n1\n2\n3\n",
            "{backend}: i.toString() output"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

const BRACE_JSON_SRC: &str =
    "let title = \"meeting\";\nprint(\"\\{\\\"title\\\": \\\"\" + title + \"\\\"}\");\n";

#[test]
fn brace_escape_writes_json_and_bare_brace_guides() {
    let dir = fresh_dir("brace");
    let ok = dir.join("ok.rnx");
    write_file(&ok, BRACE_JSON_SRC);
    let out = run_backend(&ok, "interpreter", &[]);
    assert_eq!(out.status.code(), Some(0), "{}", String::from_utf8_lossy(&out.stderr));
    assert_eq!(String::from_utf8(out.stdout.clone()).unwrap(), "{\"title\": \"meeting\"}\n");
    let bad = dir.join("bad.rnx");
    write_file(&bad, "print(\"{\\\"\");\n");
    let out = run_backend(&bad, "interpreter", &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("{...}"), "message must name interpolation, got:\n{text}");
    assert!(text.contains("\\{"), "message must name the brace escape, got:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

const NULL_LEN_SRC: &str = "let s: String? = null;\nprint(s.length());\n";

#[test]
fn null_method_fatal_is_human_with_span() {
    for with_sibling in [false, true] {
        let dir = fresh_dir(&format!("fatal-{with_sibling}"));
        write_file(&dir.join("nullstr.rnx"), NULL_LEN_SRC);
        if with_sibling {
            write_file(
                &dir.join("other.rnx"),
                "let u = 1;\nprint(u);\nprint(u);\nprint(u);\nprint(u);\nprint(u);\n",
            );
        }
        let out = Command::new(rnx())
            .arg("run")
            .arg("nullstr.rnx")
            .current_dir(&dir)
            .output()
            .unwrap();
        assert_eq!(out.status.code(), Some(1));
        let text = String::from_utf8_lossy(&out.stderr).into_owned()
            + &String::from_utf8_lossy(&out.stdout).into_owned();
        assert!(!text.contains("__rnx_"), "no internal symbols, got:\n{text}");
        assert!(
            text.contains("nullstr.rnx:2:7"),
            "span must survive sibling files, got:\n{text}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

const THROWS_Q_SRC: &str = "fn boom(): Int throws {\n    throw \"x\";\n}\nfn Main(): Int {\n    try {\n        let n = boom()?;\n        print(n);\n    } catch (e) {\n        print(\"caught\");\n    }\n    return 0;\n}\n";

#[test]
fn question_on_throws_names_the_fix() {
    let dir = fresh_dir("throwsq");
    let file = dir.join("q.rnx");
    write_file(&file, THROWS_Q_SRC);
    let out = run_backend(&file, "interpreter", &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("remove the `?`"), "got:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn typed_catch_names_bare_identifier_rule() {
    let dir = fresh_dir("catch");
    let file = dir.join("c.rnx");
    write_file(
        &file,
        "fn Main(): Int {\n    try {\n        throw 1;\n    } catch (e: Any) {\n        print(1);\n    }\n    return 0;\n}\n",
    );
    let out = run_backend(&file, "interpreter", &[]);
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("bare identifier"), "got:\n{text}");
    let _ = std::fs::remove_dir_all(&dir);
}
