use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-corelang-{tag}-{}", std::process::id()));
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

    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot {tag} exit");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot {tag} stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

fn check_fails_with(src: &str, want: &[&str], tag: &str) {
    let dir = std::env::temp_dir().join(format!("rnx-corelang-err-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), "export default {\n    project: {\n        name: \"m\",\n        version: \"0.1.0\"\n    }\n}\n").unwrap();
    std::fs::write(dir.join("src/main.rnx"), src).unwrap();
    let out = std::process::Command::new(env!("CARGO_BIN_EXE_rnx"))
        .arg("build")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("src/main.rnx")
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
fn tuple_div_rem_destructure() {
    check_stdout_everywhere(
        "fn div_rem(num: Int, den: Int): (Int, Int) {\n\
        return (num / den, num % den);\n\
        }\n\
        fn Main(): Int {\n\
        let (q, r) = div_rem(10, 3);\n\
        print(q);\n\
        print(r);\n\
        return 0;\n\
        }\n",
        "3\n1\n",
        "divrem",
    );
}

#[test]
fn tuple_swap_param_and_return() {
    check_stdout_everywhere(
        "fn swap(p: (Int, String)): (String, Int) {\n\
        let (a, b) = p;\n\
        return (b, a);\n\
        }\n\
        fn format_pair(p: (String, Int)): String {\n\
        let (name, age) = p;\n\
        return \"${name}: ${age}\";\n\
        }\n\
        fn Main(): Int {\n\
        let res = swap((42, \"hello\"));\n\
        assert(res.0 == \"hello\" && res.1 == 42, \"tuple param and return\");\n\
        print(format_pair((\"Bob\", 25)));\n\
        let (q, r) = res;\n\
        print(q);\n\
        return r - 42;\n\
        }\n",
        "Bob: 25\nhello\n",
        "swap",
    );
}

#[test]
fn tuple_three_wide() {
    check_stdout_everywhere(
        "fn triple(): (Int, String, Bool) {\n\
        return (7, \"seven\", true);\n\
        }\n\
        fn first(p: (Int, String, Bool)): Int {\n\
        return p.0;\n\
        }\n\
        fn Main(): Int {\n\
        let (a, b, c) = triple();\n\
        print(a);\n\
        print(b);\n\
        print(first((1, \"x\", false)) + 41);\n\
        if (c) { print(\"yes\"); }\n\
        return 0;\n\
        }\n",
        "7\nseven\n42\nyes\n",
        "triple",
    );
}

#[test]
fn err_tuple_nested_sig() {
    check_fails_with(
        "fn f(p: ((Int, Int), String)): Int {\n\
        return 0;\n\
        }\n\
        fn Main(): Int {\n\
        return 0;\n\
        }\n",
        &["E108", "nested tuples"],
        "nested",
    );
}

#[test]
fn err_tuple_arity_mismatch() {
    check_fails_with(
        "fn div_rem(num: Int, den: Int): (Int, Int) {\n\
        return (num / den, num % den);\n\
        }\n\
        fn Main(): Int {\n\
        let (q, r, s) = div_rem(10, 3);\n\
        return 0;\n\
        }\n",
        &["E108"],
        "arity",
    );
}

#[test]
fn defer_lifo_return_and_propagate() {
    check_stdout_everywhere(
        "fn test_defer(fail: Bool): Int? {\n\
        defer print(\"d1\");\n\
        defer print(\"d2\");\n\
        if (fail) {\n\
        return null;\n\
        }\n\
        defer print(\"d3\");\n\
        return 100;\n\
        }\n\
        fn Main(): Int {\n\
        print(test_defer(false) ?? -1);\n\
        print(\"---\");\n\
        let b = test_defer(true);\n\
        if (b != null) { print(b ?? -1); } else { print(\"early\"); }\n\
        defer print(\"last\");\n\
        defer { print(\"first\"); }\n\
        return 0;\n\
        }\n",
        "d3\nd2\nd1\n100\n---\nd2\nd1\nearly\nfirst\nlast\n",
        "deferlife",
    );
}

#[test]
fn defer_conditional() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let c = true;\n\
        if (c) {\n\
        defer print(\"cond\");\n\
        }\n\
        defer print(\"always\");\n\
        let d = false;\n\
        if (d) {\n\
        defer print(\"never\");\n\
        }\n\
        print(\"body\");\n\
        return 0;\n\
        }\n",
        "body\nalways\ncond\n",
        "defercond",
    );
}

#[test]
fn defer_loop_break() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let i = 0;\n\
        while (true) {\n\
        defer print(\"w\");\n\
        i = i + 1;\n\
        if (i >= 2) { break; }\n\
        }\n\
        print(\"body\");\n\
        return 0;\n\
        }\n",
        "w\nw\nbody\n",
        "deferloop",
    );
}

#[test]
fn defer_mutating_assign_form() {
    check_stdout_everywhere(
        "fn fill(log: Array<Int>): Int {\n\
        defer log.push(1);\n\
        defer log.push(2);\n\
        defer log.push(3);\n\
        return 0;\n\
        }\n\
        fn Main(): Int {\n\
        let log: Array<Int> = [];\n\
        fill(log);\n\
        print(log.length());\n\
        print(log[0]);\n\
        print(log[2]);\n\
        return 0;\n\
        }\n",
        "3\n3\n1\n",
        "deferassign",
    );
}

#[test]
fn spread_combined_and_rest() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let a = [1, 2];\n\
        let b = [3, 4];\n\
        let combined = [0, ...a, ...b, 5];\n\
        assert(combined.length() == 6, \"array spread length\");\n\
        assert(combined[2] == 2 && combined[4] == 4, \"array spread elements\");\n\
        let [first, ...remainder] = combined;\n\
        assert(first == 0 && remainder.length() == 5, \"array destructure rest\");\n\
        print(first);\n\
        print(remainder.length());\n\
        print(remainder[0]);\n\
        return 0;\n\
        }\n",
        "0\n5\n1\n",
        "spread",
    );
}

#[test]
fn spread_empty_and_oob_defaults() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let e: Array<Int> = [];\n\
        let full = [...e, 9, ...e];\n\
        print(full.length());\n\
        let [x, y, ...tail] = full;\n\
        print(x);\n\
        print(y);\n\
        print(tail.length());\n\
        return 0;\n\
        }\n",
        "1\n9\n0\n0\n",
        "spreadempty",
    );
}

#[test]
fn record_literal_and_rest() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let user = { id: 1, name: \"Alice\", role: \"admin\", active: true };\n\
        let { id, ...meta } = user;\n\
        print(id);\n\
        print(meta.name);\n\
        print(meta.role);\n\
        if (meta.active) { print(\"on\"); }\n\
        let nested = { a: { b: 42 } };\n\
        print(nested.a.b);\n\
        return 0;\n\
        }\n",
        "1\nAlice\nadmin\non\n42\n",
        "recordrest",
    );
}

#[test]
fn record_rest_on_struct() {
    check_stdout_everywhere(
        "struct User {\n\
        let id: Int;\n\
        let name: String;\n\
        let role: String;\n\
        }\n\
        fn Main(): Int {\n\
        let u = User(7, \"Zed\", \"root\");\n\
        let { id, ...meta } = u;\n\
        print(id);\n\
        print(meta.name);\n\
        print(meta.role);\n\
        return 0;\n\
        }\n",
        "7\nZed\nroot\n",
        "structrest",
    );
}

#[test]
fn err_spread_non_array() {
    check_fails_with(
        "fn Main(): Int {\n\
        let x = [...5];\n\
        return 0;\n\
        }\n",
        &["E108", "spread"],
        "spreadint",
    );
}

#[test]
fn err_array_rest_not_last() {
    check_fails_with(
        "fn Main(): Int {\n\
        let a = [1];\n\
        let [...r, x] = a;\n\
        return 0;\n\
        }\n",
        &["E108", "must be last"],
        "restorder",
    );
}

#[test]
fn err_record_dup_field() {
    check_fails_with(
        "fn Main(): Int {\n\
        let u = { a: 1, a: 2 };\n\
        return 0;\n\
        }\n",
        &["E108", "duplicate field"],
        "dupfield",
    );
}

#[test]
fn err_array_destructure_non_array() {
    check_fails_with(
        "fn Main(): Int {\n\
        let [a] = 5;\n\
        return 0;\n\
        }\n",
        &["E108", "non-array"],
        "noarray",
    );
}

#[test]
fn err_record_unknown_field() {
    check_fails_with(
        "fn Main(): Int {\n\
        let u = { a: 1 };\n\
        print(u.b);\n\
        return 0;\n\
        }\n",
        &["E108", "unknown field"],
        "nofield",
    );
}
