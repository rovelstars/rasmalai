use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-rtti-{tag}-{}", std::process::id()));
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
    let out = std::process::Command::new(rnx)
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

    let bin_path = dir.join("rtti_bin");
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
    let dir = std::env::temp_dir().join(format!("rnx-rtti-err-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), "export default {\n    project: {\n        name: \"m\",\n        version: \"0.1.0\"\n    }\n}\n").unwrap();
    std::fs::write(dir.join("src/main.rnx"), src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("build")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("src/main.rnx")
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

#[test]
fn rtti_typeof_values() {
    check_stdout_everywhere(
        "struct User {\n\
        let name: String;\n\
        }\n\
        fn Main(): Int {\n\
        print(typeOf(42));\n\
        print(typeOf(\"hi\"));\n\
        print(typeOf(1.5));\n\
        print(typeOf(true));\n\
        let u = User(\"Al\");\n\
        print(typeOf(u));\n\
        print(typeOf([1, 2]));\n\
        let a: Any = 7;\n\
        print(typeOf(a));\n\
        let s: Any = \"yo\";\n\
        print(typeOf(s));\n\
        print(typeOf(u));\n\
        return 0;\n\
        }\n",
        "Int\nString\nFloat\nBool\nUser\nArray\nInt\nString\nUser\n",
        "typeof",
    );
}

#[test]
fn rtti_is_predicates() {
    check_stdout_everywhere(
        "struct User {\n\
        let name: String;\n\
        }\n\
        struct Admin {\n\
        let level: Int;\n\
        }\n\
        fn Main(): Int {\n\
        print(7 is Int);\n\
        print(7 is String);\n\
        print(\"x\" is String);\n\
        let u = User(\"Al\");\n\
        print(u is User);\n\
        print(u is Admin);\n\
        let a: Any = 99;\n\
        print(a is Int);\n\
        print(a is String);\n\
        let b: Any = u;\n\
        print(b is User);\n\
        print(b is Admin);\n\
        return 0;\n\
        }\n",
        "true\nfalse\ntrue\ntrue\nfalse\ntrue\nfalse\ntrue\nfalse\n",
        "ispred",
    );
}

#[test]
fn rtti_switch_is() {
    check_stdout_everywhere(
        "struct User {\n\
        let name: String;\n\
        }\n\
        fn describe(v: Any): String {\n\
        switch v {\n\
        case is Int: return \"int\";\n\
        case is String: return \"str\";\n\
        case is User: return \"user\";\n\
        default: return \"other\";\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        print(describe(7));\n\
        print(describe(\"x\"));\n\
        let u = User(\"Al\");\n\
        print(describe(u));\n\
        print(describe(1.5));\n\
        print(describe([1]));\n\
        return 0;\n\
        }\n",
        "int\nstr\nuser\nother\nother\n",
        "switchis",
    );
}

#[test]
fn rtti_narrowing() {
    check_stdout_everywhere(
        "struct User {\n\
        let name: String;\n\
        let age: Int;\n\
        }\n\
        fn Main(): Int {\n\
        let u = User(\"Al\", 30);\n\
        if (u is User) {\n\
        print(u.name);\n\
        print(u.age + 1);\n\
        }\n\
        let a: Any = 41;\n\
        if (a is Int) {\n\
        print(a + 1);\n\
        }\n\
        let b: Any = User(\"Bo\", 7);\n\
        switch b {\n\
        case is User: print(\"got user\"); pass;\n\
        default: print(\"nope\"); pass;\n\
        }\n\
        return 0;\n\
        }\n",
        "Al\n31\n42\ngot user\n",
        "narrow",
    );
}

#[test]
fn err_is_unknown_type() {
    check_fails_with(
        "fn Main(): Int {\n\
        print(1 is Nope);\n\
        return 0;\n\
        }\n",
        &["E108", "unknown type"],
        "isunknown",
    );
}

#[test]
fn err_is_array_on_any() {
    check_fails_with(
        "fn Main(): Int {\n\
        let a: Any = [1];\n\
        print(a is Array);\n\
        return 0;\n\
        }\n",
        &["E108", "is Array"],
        "isarray",
    );
}

#[test]
fn range_first_class() {
    check_stdout_everywhere(
        "fn sum_range(r: Range): Int {\n\
        let t = 0;\n\
        for i in r {\n\
        t = t + i;\n\
        }\n\
        return t;\n\
        }\n\
        fn Main(): Int {\n\
        let r: Range = 1..5;\n\
        print(sum_range(r));\n\
        print(sum_range(10..=12));\n\
        let t = 0;\n\
        for i in 0..10 {\n\
        t = t + i;\n\
        }\n\
        for j in (0..10).stride(3) {\n\
        t = t + j;\n\
        }\n\
        print(t);\n\
        print(r is Range);\n\
        print(typeOf(r));\n\
        return 0;\n\
        }\n",
        "10\n33\n63\ntrue\nRange\n",
        "rangefn",
    );
}

#[test]
fn slice_indexing() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let a = [10, 20, 30, 40, 50];\n\
        let mid = a[1..4];\n\
        print(mid.length());\n\
        print(mid[0]);\n\
        print(mid[2]);\n\
        print(a[..2].length());\n\
        print(a[1..=2].length());\n\
        print(a[3..100].length());\n\
        let s = \"hello\";\n\
        print(s[1..4]);\n\
        print(s[..2]);\n\
        print(s[2..=3]);\n\
        return 0;\n\
        }\n",
        "3\n20\n40\n2\n2\n2\nell\nhe\nll\n",
        "slice",
    );
}

#[test]
fn err_slice_non_sliceable() {
    check_fails_with(
        "fn Main(): Int {\n\
        let x = 5;\n\
        print(x[1..2]);\n\
        return 0;\n\
        }\n",
        &["E108", "range index"],
        "sliceno",
    );
}

#[test]
fn err_range_bounds_must_be_int() {
    check_fails_with(
        "fn Main(): Int {\n\
        let r = 1..\"x\";\n\
        return 0;\n\
        }\n",
        &["E108", "range bounds"],
        "rangebound",
    );
}

#[test]
fn generic_structs() {
    check_stdout_everywhere(
        "struct Box<T> {\n\
        let value: T;\n\
        let tag: String;\n\
        }\n\
        struct Pair<A, B> {\n\
        let first: A;\n\
        let second: B;\n\
        }\n\
        fn Main(): Int {\n\
        let b = Box(42, \"answer\");\n\
        print(b.value);\n\
        print(b.tag);\n\
        print(typeOf(b));\n\
        print(b is Box);\n\
        let p = Pair(\"x\", 7);\n\
        print(p.first);\n\
        print(p.second);\n\
        let s = Box(\"hi\", \"greet\");\n\
        print(s.value);\n\
        return 0;\n\
        }\n",
        "42\nanswer\nBox\ntrue\nx\n7\nhi\n",
        "generics",
    );
}

#[test]
fn generic_struct_methods() {
    check_stdout_everywhere(
        "struct Box<T> {\n\
        let value: T;\n\
        fn get(): T {\n\
        return this.value;\n\
        }\n\
        fn set(v: T) {\n\
        this.value = v;\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let b = Box(1);\n\
        print(b.get());\n\
        b.set(99);\n\
        print(b.get());\n\
        let s = Box(\"a\");\n\
        print(s.get());\n\
        return 0;\n\
        }\n",
        "1\n99\na\n",
        "genmethods",
    );
}
