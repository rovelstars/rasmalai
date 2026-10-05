use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-optres-{tag}-{}", std::process::id()));
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

fn check_all_backends(src: &str, want: i64, want_out: &[String], tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let got = machine.output.clone();
    let want_out: Vec<String> = want_out.to_vec();
    assert_eq!(got, want_out, "interpreter {tag} stdout");

    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), want, "cranelift {tag}");
    assert_eq!(llvm::codegen::execute(leaked, "Main").unwrap(), want, "llvm {tag}");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn null_contract_basics() {
    check_all_backends(
        "import { Map } from \"@std/collections\";\n\
        fn Main(): Int {\n\
        assert(typeOf(null) == \"Null\", \"typeof null\");\n\
        let m = new Map();\n\
        m.set(\"k\", \"v\");\n\
        assert(m.get(\"k\") == \"v\", \"present key\");\n\
        assert(m.get(\"missing\") == null, \"missing key is null\");\n\
        let n: String? = null;\n\
        assert(n == null, \"nullable binds null\");\n\
        assert((n ?? \"dflt\") == \"dflt\", \"coalesce null\");\n\
        let s: String? = \"hi\";\n\
        assert((s ?? \"dflt\") == \"hi\", \"coalesce value\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "null",
    );
}

#[test]
fn result_constructors_unwrap() {
    check_all_backends(
        "fn Main(): Int {\n\
        let r = Result.Ok(10);\n\
        assert(r.isOk(), \"is ok\");\n\
        assert(!r.isErr(), \"not err\");\n\
        assert(r.unwrap() == 10, \"ok value\");\n\
        assert(r.map((v: Int) => v * 2).unwrap() == 20, \"mapped\");\n\
        let e = Result.Err(\"bad\");\n\
        assert(e.isErr(), \"is err\");\n\
        assert(e.unwrapOr(3) == 3, \"or default\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "res",
    );
}

#[test]
fn array_pop_nullable() {
    check_all_backends(
        "fn Main(): Int {\n\
        assert([1].pop() == 1, \"pop value\");\n\
        assert([].pop() == null, \"pop empty is null\");\n\
        let arr = [1, 2, 3];\n\
        assert(arr.pop() == 3, \"pop tail\");\n\
        assert(arr.length() == 2, \"pop shrinks\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "pop",
    );
}

#[test]
fn null_branching() {
    check_all_backends(
        "fn pick(o: Int?): Int {\n\
        if o != null {\n\
            return o;\n\
        }\n\
        return -1;\n\
        }\n\
        fn Main(): Int {\n\
        assert(pick(3) == 3, \"value arm\");\n\
        assert(pick(null) == -1, \"null arm\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "branch",
    );
}

#[test]
fn uninit_let_defaults_to_null() {
    check_all_backends(
        "fn Main(): Int {\n\
        let a;\n\
        assert(a == null, \"untyped uninit is null\");\n\
        let b: Int?;\n\
        assert(b == null, \"nullable uninit is null\");\n\
        b = 10;\n\
        assert((b ?? -1) == 10, \"nullable assign\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "uninit",
    );
}

#[test]
fn uninit_nonnullable_reads_zero() {
    check_all_backends(
        "fn Main(): Int {\n\
        let c: Int;\n\
        assert(c == 0, \"int default zero\");\n\
        c = 7;\n\
        assert(c == 7, \"int assign\");\n\
        let f: Bool;\n\
        assert(!f, \"bool default false\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "uninitzero",
    );
}
