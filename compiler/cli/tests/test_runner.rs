fn proj(tag: &str, name: &str, files: &[(&str, &str)]) -> std::path::PathBuf {
    let base = std::env::temp_dir().join(format!("rnx-test-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    let root = base.join(name);
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(
        root.join("Project.config"),
        format!("export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}\n}}\n"),
    )
    .unwrap();
    for (rel, src) in files {
        let p = root.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, src).unwrap();
    }
    root
}

fn rnx_test(dir: &std::path::Path, args: &[&str]) -> (i32, String) {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.arg("test").current_dir(dir);
    for a in args {
        cmd.arg(a);
    }
    let out = cmd.output().unwrap();
    let code = out.status.code().unwrap();
    let text =
        String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    (code, text)
}

const MAIN: &str = "fn Main(): Int {\n    return 0;\n}\n";

#[test]
fn test_rnx_test_passing() {
    let root = proj(
        "pass",
        "calc",
        &[
            ("src/main.rnx", MAIN),
            (
                "tests/math.rnx",
                "test fn test_add() {\n    assert(1 + 1 == 2, \"add broken\");\n}\n\ntest fn test_sub() {\n    assert(5 - 3 == 2, \"sub broken\");\n}\n",
            ),
        ],
    );
    let (code, text) = rnx_test(&root, &[]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("tests/math.rnx\n"), "{text}");
    assert!(text.contains("✓ test_add"), "{text}");
    assert!(text.contains("✓ test_sub"), "{text}");
    assert!(text.contains("tests: 2 passed, 0 failed in "), "{text}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn test_rnx_test_failing() {
    let root = proj(
        "fail",
        "calc",
        &[
            ("src/main.rnx", MAIN),
            (
                "tests/math.rnx",
                "test fn test_math() {\n    assert(1 == 2, \"math broken\");\n}\n",
            ),
        ],
    );
    let (code, text) = rnx_test(&root, &[]);
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("✗ test_math"), "{text}");
    assert!(text.contains("math broken"), "{text}");
    assert!(text.contains("failure in tests/math.rnx > test_math"), "{text}");
    assert!(text.contains("tests: 0 passed, 1 failed in "), "{text}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn test_rnx_test_filter() {
    let root = proj(
        "filter",
        "calc",
        &[
            ("src/main.rnx", MAIN),
            (
                "tests/three.rnx",
                "test fn test_alpha() {\n    assert(true, \"no\");\n}\ntest fn test_beta() {\n    assert(true, \"no\");\n}\ntest fn test_gamma() {\n    assert(true, \"no\");\n}\n",
            ),
        ],
    );
    let (code, text) = rnx_test(&root, &["beta"]);
    assert_eq!(code, 0, "{text}");
    assert!(text.contains("tests/three.rnx\n"), "{text}");
    assert!(text.contains("✓ test_beta"), "{text}");
    assert!(!text.contains("test_alpha"), "{text}");
    assert!(!text.contains("test_gamma"), "{text}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn test_rnx_test_backends() {
    let root = proj(
        "backends",
        "calc",
        &[
            ("src/main.rnx", MAIN),
            (
                "tests/math.rnx",
                "test fn test_add() {\n    assert(1 + 1 == 2, \"add broken\");\n}\n",
            ),
        ],
    );
    for backend in ["interpreter", "cranelift", "llvm"] {
        let (code, text) = rnx_test(&root, &["--backend", backend]);
        assert_eq!(code, 0, "{backend}: {text}");
        assert!(
            text.contains("tests: 1 passed, 0 failed in "),
            "{backend}: {text}"
        );
    }
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn test_workspace_test_p_flag() {
    let base = std::env::temp_dir().join(format!("rnx-test-ws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    std::fs::create_dir_all(base.join("calc").join("src")).unwrap();
    std::fs::create_dir_all(base.join("calc").join("tests")).unwrap();
    std::fs::create_dir_all(base.join("plain").join("src")).unwrap();
    std::fs::write(base.join("Project.config"), "export default {\n    workspace: {\n        members: [\"calc\", \"plain\"]\n    }\n}\n").unwrap();
    std::fs::write(
        base.join("calc").join("Project.config"),
        "export default {\n    project: {\n        name: \"calc\",\n        version: \"0.1.0\",\n        entry: \"src/lib.rnx\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(base.join("calc").join("src").join("lib.rnx"), "fn add(a: Int, b: Int): Int {\n    return a + b;\n}\n").unwrap();
    std::fs::write(
        base.join("calc").join("tests").join("calc.rnx"),
        "import { add } from \"../src/lib\";\ntest fn test_add() {\n    assert(add(20, 22) == 42, \"add broken\");\n}\n",
    )
    .unwrap();
    std::fs::write(
        base.join("plain").join("Project.config"),
        "export default {\n    project: {\n        name: \"plain\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(base.join("plain").join("src").join("main.rnx"), MAIN).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("test")
        .arg("-p")
        .arg("calc")
        .current_dir(&base)
        .output()
        .unwrap();
    assert_eq!(out.status.code().unwrap(), 0, "{}", String::from_utf8_lossy(&out.stderr));
    let text = String::from_utf8(out.stdout).unwrap();
    assert!(text.contains("tests/calc.rnx\n"), "{text}");
    assert!(text.contains("✓ test_add"), "{text}");
    assert!(text.contains("tests: 1 passed, 0 failed in "), "{text}");
    let _ = std::fs::remove_dir_all(&base);
}

#[test]
fn test_run_error_attributes_owning_file() {
    let root = proj(
        "attr",
        "calc",
        &[
            ("src/main.rnx", "//! Main module docs here.\nimport {\n    helper\n} from \"./util\";\n\nfn Main(): Int {\n    return helper();\n}\n"),
            ("src/util.rnx", "//! Utility module.\nimport {\n    missing\n} from \"@std/nope \";\n\nfn helper(): Int {\n    return 1;\n}\n"),
        ],
    );
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("run")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("src/util.rnx"), "wrong file:\n{text}");
    assert!(text.contains("fix:"), "missing fix footer:\n{text}");
    assert!(!text.contains("(63.."), "raw span leaked:\n{text}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}

#[test]
fn test_run_fatal_shows_source_context() {
    let root = proj(
        "fatalctx",
        "calc",
        &[(
            "src/main.rnx",
            "fn half(x: Int): Int {\n    return x / 0;\n}\n\nfn Main(): Int {\n    let v = half(3);\n    return v;\n}\n",
        )],
    );
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("run")
        .current_dir(&root)
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("fatal: division by zero"), "{text}");
    assert!(text.contains("src/main.rnx:2:12"), "{text}");
    assert!(text.contains("▲"), "{text}");
    assert!(!text.contains("(63.."), "{text}");
    let _ = std::fs::remove_dir_all(root.parent().unwrap());
}
