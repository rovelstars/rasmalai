use std::path::PathBuf;
use std::process::Command;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-geb-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("geb_bin");
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
    let dir = std::env::temp_dir().join(format!("rnx-geb-err-{tag}-{}", std::process::id()));
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

#[test]
fn bits_hex_bin_literals() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let mask = 0xFF;\n\
        let flags = 0b1010;\n\
        print(flags & 0b0010);\n\
        print(flags | 0b0101);\n\
        print(flags ^ 0b1111);\n\
        print(~0 & 0xFF);\n\
        print(mask);\n\
        print(flags);\n\
        print(0xFF_00);\n\
        print(0b1010_0110);\n\
        print(0x7FFF);\n\
        print(0B1111);\n\
        print((flags & 0b0010) == 2);\n\
        print((~5) + 6);\n\
        return 0;\n\
        }\n",
        "2\n15\n5\n255\n255\n10\n65280\n166\n32767\n15\ntrue\n0\n",
        "bits",
    );
}

#[test]
fn extension_methods() {
    check_stdout_everywhere(
        "extension String {\n\
        fn double(): String {\n\
            return this + this;\n\
        }\n\
        }\n\
        extension Int {\n\
        fn isEven(): Bool {\n\
            return (this & 1) == 0;\n\
        }\n\
        fn addN(n: Int): Int {\n\
            return this + n;\n\
        }\n\
        }\n\
        class Counter {\n\
        let n: Int;\n\
        init(n: Int) { this.n = n; }\n\
        }\n\
        extension Counter {\n\
        fn next(): Int {\n\
            return this.n + 1;\n\
        }\n\
        }\n\
        extension Array {\n\
        fn count(): Int {\n\
            return this.length();\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        print(\"hi\".double());\n\
        print(42.isEven());\n\
        print(5.isEven());\n\
        let num = 42;\n\
        print(num.isEven());\n\
        print(num.addN(8));\n\
        print(num.addN(n: 8));\n\
        let c = new Counter(41);\n\
        print(c.next());\n\
        let a: Array<Int> = [];\n\
        print(a.count());\n\
        return 0;\n\
        }\n",
        "hihi\ntrue\nfalse\ntrue\n50\n50\n42\n0\n",
        "extensions",
    );
}

#[test]
fn generic_inference() {
    check_stdout_everywhere(
        "fn identity<T>(val: T): T {\n\
        return val;\n\
        }\n\
        fn make_pair<A, B>(first: A, second: B): (A, B) {\n\
        return (first, second);\n\
        }\n\
        fn wrap<T>(val: T): (T, Bool) {\n\
        return (val, true);\n\
        }\n\
        fn Main(): Int {\n\
        print(identity(42));\n\
        print(identity<Int>(7));\n\
        print(identity(\"hi\"));\n\
        let p = make_pair(\"status\", 200);\n\
        print(p.0);\n\
        print(p.1);\n\
        let pair_int = wrap(100);\n\
        print(pair_int.0 == 100);\n\
        print(pair_int.1);\n\
        let pair_str = wrap(\"test\");\n\
        print(pair_str.0 == \"test\");\n\
        print(pair_str.1);\n\
        return 0;\n\
        }\n",
        "42\n7\nhi\nstatus\n200\ntrue\ntrue\ntrue\ntrue\n",
        "generics",
    );
}

#[test]
fn task_program_assert_style() {
    check_stdout_everywhere(
        "fn wrap<T>(val: T): (T, Bool) {\n\
        return (val, true);\n\
        }\n\
        extension String {\n\
        fn double(): String {\n\
            return this + this;\n\
        }\n\
        }\n\
        extension Int {\n\
        fn isEven(): Bool {\n\
            return (this & 1) == 0;\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let mask = 0xFF;\n\
        let flags = 0b1010;\n\
        assert((flags & 0b0010) == 2, \"bitwise AND\");\n\
        assert((flags | 0b0101) == 0b1111, \"bitwise OR\");\n\
        assert((flags ^ 0b1111) == 0b0101, \"bitwise XOR\");\n\
        assert((~0 & 0xFF) == 255, \"bitwise NOT with mask\");\n\
        assert(mask == 255, \"hex literal value\");\n\
        assert(flags == 10, \"binary literal value\");\n\
        let greeting = \"hi\".double();\n\
        assert(greeting == \"hihi\", \"string extension\");\n\
        let num = 42;\n\
        assert(num.isEven(), \"int extension method\");\n\
        assert(!5.isEven(), \"int literal extension method\");\n\
        let pair_int = wrap(100);\n\
        assert(pair_int.0 == 100 && pair_int.1, \"generic inference int\");\n\
        let pair_str = wrap(\"test\");\n\
        assert(pair_str.0 == \"test\" && pair_str.1, \"generic inference string\");\n\
        print(\"ok\");\n\
        return 0;\n\
        }\n",
        "ok\n",
        "task_program",
    );
}

#[test]
fn err_bitwise_non_int() {
    check_fails_with(
        "fn Main(): Int {\n    print(\"a\" & \"b\");\n    return 0;\n}\n",
        &["bitwise operations need `Int` operands"],
        "bits_str",
    );
}

#[test]
fn err_bitnot_non_int() {
    check_fails_with(
        "fn Main(): Int {\n    print(~true);\n    return 0;\n}\n",
        &["bitwise `~` needs an `Int` operand"],
        "bitnot_bool",
    );
}

#[test]
fn err_generic_conflict() {
    check_fails_with(
        "fn pick<T>(x: T, y: T): T {\n    return x;\n}\nfn Main(): Int {\n    print(pick(1, \"a\"));\n    return 0;\n}\n",
        &["conflicting types for `T`"],
        "generic_conflict",
    );
}

#[test]
fn err_generic_uninferrable() {
    check_fails_with(
        "fn maker<T>(): Int {\n    return 0;\n}\nfn Main(): Int {\n    print(maker());\n    return 0;\n}\n",
        &["cannot infer type parameter `T`"],
        "generic_noinfer",
    );
}

#[test]
fn err_generic_arity() {
    check_fails_with(
        "fn ident<T>(x: T): T {\n    return x;\n}\nfn Main(): Int {\n    print(ident<Int, String>(1));\n    return 0;\n}\n",
        &["takes 1 type arguments but 2 were given"],
        "generic_arity",
    );
}

#[test]
fn err_targs_on_plain_fn() {
    check_fails_with(
        "fn f(x: Int): Int {\n    return x;\n}\nfn Main(): Int {\n    print(f<Int>(1));\n    return 0;\n}\n",
        &["is not generic"],
        "targs_plain",
    );
}

#[test]
fn err_extend_unknown() {
    check_fails_with(
        "extension Ghost {\n    fn boo(): Int {\n        return 1;\n    }\n}\nfn Main(): Int {\n    return 0;\n}\n",
        &["cannot extend unknown type `Ghost`"],
        "ext_unknown",
    );
}

#[test]
fn err_ext_duplicate() {
    check_fails_with(
        "extension String {\n    fn dup(): String {\n        return this;\n    }\n}\nextension String {\n    fn dup(): String {\n        return this;\n    }\n}\nfn Main(): Int {\n    return 0;\n}\n",
        &["duplicate extension method"],
        "ext_dup",
    );
}
