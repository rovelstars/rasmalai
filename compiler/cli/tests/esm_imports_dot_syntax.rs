use std::path::PathBuf;
use std::process::Command;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-esm-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    (out, dir)
}

fn run_backend(rnx: &str, dir: &PathBuf, backend: &str) -> String {
    let out = Command::new(rnx)
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn check_stdout_everywhere(src: &str, want: &str, tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(0) => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    assert_eq!(machine.output.join("\n") + "\n", want, "interpreter {tag} stdout");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    for backend in ["cranelift", "llvm"] {
        let got = run_backend(rnx, &dir, backend);
        assert_eq!(got, want, "{backend} {tag} stdout");
    }

    let bin_path = dir.join("esm_bin");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot {tag} exit");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot {tag} stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

fn check_fails_with(src: &str, want: &[&str], tag: &str) {
    let dir = std::env::temp_dir().join(format!("rnx-esm-err-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("build")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("main.rnx")
        .arg("-o")
        .arg(dir.join("should_not_exist_bin"))
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1), "expected failure");
    let mut text = String::from_utf8_lossy(&out.stderr).into_owned();
    text.push_str(&String::from_utf8_lossy(&out.stdout));
    for w in want {
        assert!(text.contains(w), "missing `{w}` in: {text}");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

fn write_proj(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-esm-proj-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    for (rel, content) in files {
        let p = dir.join(rel);
        std::fs::create_dir_all(p.parent().unwrap()).unwrap();
        std::fs::write(&p, content).unwrap();
    }
    std::fs::write(dir.join("Project.config"), "export default {\n    project: {\n        name: \"m\",\n        version: \"0.1.0\"\n    }\n}\n").unwrap();
    dir
}

fn run_proj(dir: &PathBuf, backend: &str) -> (bool, String) {
    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("src/main.rnx"))
        .output()
        .unwrap();
    (out.status.success(), String::from_utf8_lossy(&out.stdout).into_owned())
}

#[test]
fn esm_default_and_namespace() {
    check_stdout_everywhere(
        "import testing from \"std/testing\";\n\
        import * as t2 from \"std/testing\";\n\
        import t3, { blackBox } from \"std/testing\";\n\
        fn Main(): Int {\n\
        testing.blackBox(1);\n\
        t2.blackBox(2);\n\
        t3.blackBox(3);\n\
        print(blackBox(4));\n\
        return 0;\n\
        }\n",
        "4\n",
        "esm_ns",
    );
}

#[test]
fn esm_bare_std_prefix() {
    check_stdout_everywhere(
        "import { blackBox } from \"std/testing\";\n\
        fn Main(): Int {\n\
        print(blackBox(9));\n\
        return 0;\n\
        }\n",
        "9\n",
        "esm_bare",
    );
}

#[test]
fn esm_relative_and_side_effect() {
    let dir = write_proj(
        "rel",
        &[
            ("src/util.rnx", "pub fn helper(): Int {\n    return 41;\n}\n"),
            (
                "src/main.rnx",
                "import \"./util\";\nimport { helper } from \"./util\";\nfn Main(): Int {\n    print(helper() + 1);\n    return 0;\n}\n",
            ),
        ],
    );
    for backend in ["interpreter", "cranelift", "llvm"] {
        let (ok, out) = run_proj(&dir, backend);
        assert!(ok, "{backend} failed: {out}");
        assert_eq!(out, "42\n", "{backend} stdout");
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn dot_enum_variants() {
    check_stdout_everywhere(
        "enum Status {\n\
        Idle,\n\
        Running(Int),\n\
        Failed(String),\n\
        }\n\
        fn check_status(s: Status): String {\n\
        switch s {\n\
        case .Idle: return \"idle\";\n\
        case .Running(pid): return \"running\";\n\
        case .Failed(msg): return \"error\";\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let s1 = Status.Idle;\n\
        let s2 = Status.Running(1024);\n\
        let s3 = Status.Failed(\"oom\");\n\
        print(check_status(s1));\n\
        print(check_status(s2));\n\
        print(check_status(s3));\n\
        let opt: Int? = 42;\n\
        print(opt ?? -1);\n\
        let flag: Int? = 100;\n\
        print(flag ?? -1);\n\
        return 0;\n\
        }\n",
        "idle\nrunning\nerror\n42\n100\n",
        "dot_enum",
    );
}

#[test]
fn generic_no_turbofish() {
    check_stdout_everywhere(
        "fn identity<T>(val: T): T {\n\
        return val;\n\
        }\n\
        fn Main(): Int {\n\
        print(identity(42));\n\
        print(identity<String>(\"clean\"));\n\
        return 0;\n\
        }\n",
        "42\nclean\n",
        "generic_noturbo",
    );
}

#[test]
fn err_double_colon() {
    check_fails_with(
        "fn Main(): Int {\n    let opt = Option::Some(42);\n    return 0;\n}\n",
        &["E005", "'::' is not valid syntax"],
        "coloncolon_enum",
    );
}

#[test]
fn err_turbofish() {
    check_fails_with(
        "fn identity<T>(val: T): T {\n    return val;\n}\nfn Main(): Int {\n    print(identity::<Int>(1));\n    return 0;\n}\n",
        &["E005"],
        "turbofish",
    );
}

#[test]
fn err_import_double_colon() {
    check_fails_with(
        "import { x } from \"std::fs\";\nfn Main(): Int {\n    return 0;\n}\n",
        &["cannot resolve module"],
        "import_colon",
    );
}
