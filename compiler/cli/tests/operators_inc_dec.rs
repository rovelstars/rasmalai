use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-incdec-{tag}-{}", std::process::id()));
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

fn sem_err(src: &str, tag: &str) -> diagnostics::Diagnostic {
    let dir = std::env::temp_dir().join(format!("rnx-incdec-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let errs: Vec<_> = frontend::semantic::check(&m)
        .into_iter()
        .filter(|x| !x.code.is_warning())
        .collect();
    let _ = std::fs::remove_dir_all(&dir);
    assert!(!errs.is_empty(), "{tag}: expected a semantic error");
    errs.into_iter().next().unwrap()
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
fn prefix_postfix_values() {
    check_all_backends(
        "fn Main(): Int {\n\
        let a = 10;\n\
        let b = a++;\n\
        assert(b == 10, \"post yields old\");\n\
        assert(a == 11, \"post stores new\");\n\
        let c = 10;\n\
        let d = ++c;\n\
        assert(d == 11, \"pre yields new\");\n\
        assert(c == 11, \"pre stores new\");\n\
        let x = 5;\n\
        assert(x-- == 5, \"postdec old\");\n\
        assert(x == 4, \"postdec store\");\n\
        assert(--x == 3, \"predec new\");\n\
        assert(x == 3, \"predec store\");\n\
        print(\"incdec-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["incdec-ok".to_string()],
        "values",
    );
}

#[test]
fn compound_assign_int() {
    check_all_backends(
        "fn Main(): Int {\n\
        let x = 20;\n\
        x += 5;\n\
        assert(x == 25, \"pluseq\");\n\
        x -= 10;\n\
        assert(x == 15, \"minuseq\");\n\
        print(\"compound-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["compound-ok".to_string()],
        "compound",
    );
}

#[test]
fn float_incdec_compound() {
    check_all_backends(
        "fn Main(): Int {\n\
        let f = 2.5;\n\
        f++;\n\
        assert(f == 3.5, \"float post\");\n\
        f += 1.5;\n\
        assert(f == 5.0, \"float pluseq\");\n\
        f -= 2.0;\n\
        assert(f == 3.0, \"float minuseq\");\n\
        --f;\n\
        assert(f == 2.0, \"float predec\");\n\
        print(\"float-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["float-ok".to_string()],
        "float",
    );
}

#[test]
fn index_member_single_eval() {
    check_all_backends(
        "class Box {\n\
        let v: Int;\n\
        init(v: Int) { this.v = v; }\n\
        }\n\
        fn Main(): Int {\n\
        let cc = [0];\n\
        let getIndex = (): Int => { cc[0]++; return 0; };\n\
        let items = [100, 200];\n\
        items[getIndex()]++;\n\
        assert(cc[0] == 1, \"index evaluated once\");\n\
        assert(items[0] == 101, \"index post\");\n\
        items[getIndex()] += 10;\n\
        assert(cc[0] == 2, \"compound index evaluated once\");\n\
        assert(items[0] == 111, \"index pluseq\");\n\
        let b = new Box(10);\n\
        b.v++;\n\
        assert(b.v == 11, \"member post\");\n\
        b.v += 5;\n\
        assert(b.v == 16, \"member pluseq\");\n\
        b.v -= 2;\n\
        assert(b.v == 14, \"member minuseq\");\n\
        let old = b.v++;\n\
        assert(old == 14, \"member post yields old\");\n\
        assert(b.v == 15, \"member post stores new\");\n\
        print(\"targets-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["targets-ok".to_string()],
        "targets",
    );
}

#[test]
fn deref_incdec() {
    check_all_backends(
        "import { ByteBuffer } from \"@std/bytes\";\n\
        fn Main(): Int {\n\
        let buf = ByteBuffer.allocate(32);\n\
        unsafe {\n\
        let ptr: Pointer<Int> = Pointer.fromAddress<Int>(buf.address());\n\
        *ptr = 10;\n\
        (*ptr)++;\n\
        assert(*ptr == 11, \"deref post\");\n\
        assert((*ptr)++ == 11, \"deref yields old\");\n\
        assert(*ptr == 12, \"deref stores\");\n\
        ++(*ptr);\n\
        assert(*ptr == 13, \"deref pre\");\n\
        }\n\
        print(\"deref-ok\");\n\
        return 0;\n\
        }\n",
        0,
        &["deref-ok".to_string()],
        "deref",
    );
}

#[test]
fn const_mutation_rejected() {
    let err = sem_err(
        "const LIMIT = 100;\nfn Main(): Int {\n    LIMIT++;\n    return 0;\n}\n",
        "constpost",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
    assert!(err.message.contains("const"), "{err:?}");
    let err = sem_err(
        "const LIMIT = 100;\nfn Main(): Int {\n    --LIMIT;\n    return 0;\n}\n",
        "constpre",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
}

#[test]
fn non_lvalue_rejected() {
    let err = sem_err(
        "fn Main(): Int {\n    let a = 1;\n    let b = 2;\n    (a + b)++;\n    return 0;\n}\n",
        "binpost",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
    let err = sem_err(
        "fn Main(): Int {\n    5--;\n    return 0;\n}\n",
        "litpost",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
    let err = sem_err(
        "fn foo(): Int {\n    return 1;\n}\nfn Main(): Int {\n    ++foo();\n    return 0;\n}\n",
        "callpre",
    );
    assert_eq!(err.code, diagnostics::Code::E108, "{err:?}");
}
