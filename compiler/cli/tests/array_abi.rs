use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-arrayabi-{tag}-{}", std::process::id()));
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

fn lower_err(src: &str, tag: &str) -> diagnostics::Diagnostic {
    let dir = std::env::temp_dir().join(format!("rnx-arrayabi-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let err = lir::lower::lower(&m).expect_err("expected lowering error");
    let _ = std::fs::remove_dir_all(&dir);
    err
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

    for release in [false, true] {
        let rnx = env!("CARGO_BIN_EXE_rnx");
        let mut cmd = std::process::Command::new(rnx);
        cmd.arg("build").arg(dir.join("main.rnx"));
        if release {
            cmd.arg("--release");
        }
        let build = cmd.output().unwrap();
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
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn direct_param_read_and_mutation() {
    check_all_backends(
        "fn mutate(arr: Array<Int>): Int {\n\
        arr[0] = 999;\n\
        return arr[0];\n\
        }\n\
        fn Main(): Int {\n\
        let arr: Array<Int> = [1, 2, 3];\n\
        let readback = mutate(arr);\n\
        assert(readback == 999, \"return\");\n\
        assert(arr[0] == 999, \"alias\");\n\
        print(\"abi-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["abi-ok".to_string()],
        "direct",
    );
}

#[test]
fn multiple_params_mixed_types() {
    check_all_backends(
        "fn copyFirst(src: Array<Int>, dest: Array<Int>, offset: Int): Int {\n\
        dest[offset] = src[0];\n\
        return 0;\n\
        }\n\
        fn Main(): Int {\n\
        let a: Array<Int> = [42];\n\
        let b: Array<Int> = [0, 0];\n\
        copyFirst(a, b, 1);\n\
        assert(b[0] == 0, \"untouched\");\n\
        assert(b[1] == 42, \"copied\");\n\
        assert(a[0] == 42, \"source intact\");\n\
        print(\"multi-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["multi-ok".to_string()],
        "multi",
    );
}

#[test]
fn chained_calls_three_levels() {
    check_all_backends(
        "fn levelC(xs: Array<Int>): Int {\n\
        xs[0] = xs[0] + 1;\n\
        return xs[0];\n\
        }\n\
        fn levelB(xs: Array<Int>): Int {\n\
        xs[0] = xs[0] + 10;\n\
        return levelC(xs);\n\
        }\n\
        fn levelA(xs: Array<Int>): Int {\n\
        xs[0] = xs[0] + 100;\n\
        return levelB(xs);\n\
        }\n\
        fn Main(): Int {\n\
        let v: Array<Int> = [0];\n\
        let r = levelA(v);\n\
        assert(r == 111, \"chain return\");\n\
        assert(v[0] == 111, \"chain alias\");\n\
        print(\"chain-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["chain-ok".to_string()],
        "chain",
    );
}

#[test]
fn bare_array_param_over_typed_array_rejected() {
    let err = lower_err(
        "fn bump(a: Array): Int {\n\
        a[0] = 42;\n\
        return a[0];\n\
        }\n\
        fn Main(): Int {\n\
        let cc = [0];\n\
        print(bump(cc));\n\
        print(cc[0]);\n\
        return 0;\n\
        }\n",
        "view",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
    assert!(err.message.contains("array element type mismatch"), "{err:?}");
}

#[test]
fn bare_array_alias_of_typed_array_rejected() {
    let err = lower_err(
        "fn Main(): Int {\n\
        let typedArr = [1, 2];\n\
        let wide: Array = typedArr;\n\
        print(wide[0]);\n\
        return 0;\n\
        }\n",
        "alias",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
    assert!(err.message.contains("array element type mismatch"), "{err:?}");
}

#[test]
fn fresh_literal_views_still_allowed() {
    check_all_backends(
        "fn sum(xs: Array<Int>): Int {\n\
        return xs[0] + xs[1];\n\
        }\n\
        fn Main(): Int {\n\
        assert(sum([3, 4]) == 7, \"fresh literal arg\");\n\
        let wide: Array = [1, 2];\n\
        assert(wide[0] == 1, \"fresh bare let\");\n\
        let out: Array<Int> = [];\n\
        out.push(9);\n\
        assert(out[0] == 9, \"empty literal annotate\");\n\
        print(\"fresh-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["fresh-ok".to_string()],
        "fresh",
    );
}
