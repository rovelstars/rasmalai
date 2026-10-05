use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-tagunion-{tag}-{}", std::process::id()));
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

fn check_lower_error(src: &str, want_msg: &str, tag: &str) {
    let dir = std::env::temp_dir().join(format!("rnx-tagunion-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let err = lir::lower::lower(&m).expect_err("expected lowering error");
    assert!(err.message.contains(want_msg), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

const EXPR: &str = "enum Expr {\n\
    Num(Int),\n\
    Ident(String),\n\
    Add(Expr, Expr)\n\
    }\n\
    fn eval(e: Expr): Int {\n\
    switch (e) {\n\
    case .Num(n): return n;\n\
    case .Ident(s): return 0;\n\
    case .Add(lhs, rhs): return eval(lhs) + eval(rhs);\n\
    }\n\
    }\n";

#[test]
fn recursive_eval_with_payloads() {
    let src = format!(
        "{EXPR}\nfn Main(): Int {{\n\
        let one = Expr.Num(1);\n\
        let two = Expr.Num(2);\n\
        let sum = Expr.Add(one, two);\n\
        let res = eval(sum);\n\
        assert(res == 3, \"eval\");\n\
        print(\"eval-ok\");\n\
        return 0;\n\
        }}\n"
    );
    check_all_backends(&src, 0, &["eval-ok".to_string()], "eval");
}

#[test]
fn qualified_variant_patterns() {
    let src = "enum Expr {\n\
        Num(Int),\n\
        Ident(String),\n\
        Add(Expr, Expr)\n\
        }\n\
        fn eval(e: Expr): Int {\n\
        switch (e) {\n\
        case Expr.Num(n): return n;\n\
        case Expr.Ident(s): return 0;\n\
        case Expr.Add(lhs, rhs): return eval(lhs) + eval(rhs);\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let sum = Expr.Add(Expr.Num(1), Expr.Num(2));\n\
        assert(eval(sum) == 3, \"qualified\");\n\
        print(\"qual-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["qual-ok".to_string()], "qualified");
}

#[test]
fn same_variant_name_in_two_enums() {
    let src = "enum A {\n\
        X(Int)\n\
        }\n\
        enum B {\n\
        X(String)\n\
        }\n\
        fn Main(): Int {\n\
        let fa = switch (A.X(3)) {\n\
        case A.X(n): n,\n\
        default: -1,\n\
        };\n\
        let fb = switch (B.X(\"hi\")) {\n\
        case B.X(s): 7,\n\
        default: -1,\n\
        };\n\
        assert(fa == 3, \"a\");\n\
        assert(fb == 7, \"b\");\n\
        print(\"ambig-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["ambig-ok".to_string()], "ambig");
}

#[test]
fn unit_and_payload_variants_coexist() {
    let src = "enum Token {\n\
        Eof,\n\
        Number(Int),\n\
        Operator(String, Int)\n\
        }\n\
        fn Main(): Int {\n\
        let t = Token.Eof;\n\
        let n = Token.Number(42);\n\
        let op = Token.Operator(\"+\", 1);\n\
        let a = switch (t) {\n\
        case .Eof: 0,\n\
        default: -1,\n\
        };\n\
        let b = switch (n) {\n\
        case .Number(v): v,\n\
        default: -1,\n\
        };\n\
        let c = switch (op) {\n\
        case .Operator(sym, prec): prec,\n\
        default: -1,\n\
        };\n\
        assert(a == 0 && b == 42 && c == 1, \"tokens\");\n\
        print(\"token-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["token-ok".to_string()], "token");
}

#[test]
fn payload_churn_stress() {
    let src = format!(
        "{EXPR}\nfn Main(): Int {{\n\
        let i = 0;\n\
        let total = 0;\n\
        while (i < 20000) {{\n\
        let e = Expr.Add(Expr.Num(i), Expr.Ident(\"name\"));\n\
        total = total + eval(e);\n\
        i = i + 1;\n\
        }}\n\
        assert(total == 199990000, \"stress\");\n\
        print(\"stress-ok\");\n\
        return 0;\n\
        }}\n"
    );
    check_all_backends(&src, 0, &["stress-ok".to_string()], "stress");
}

#[test]
fn wrong_enum_qualifier_rejected() {
    check_lower_error(
        "enum A {\n    X(Int)\n    }\n\
        enum B {\n    Y(Int)\n    }\n\
        fn f(e: B): Int {\n\
        switch (e) {\n\
        case A.Y(n): return n;\n\
        default: return -1;\n\
        }\n\
        }\n\
        fn Main(): Int {\n    return f(B.Y(1));\n    }\n",
        "belongs to `B`, not `A`",
        "belongs",
    );
}

#[test]
fn payload_arity_mismatch_rejected() {
    check_lower_error(
        "enum E {\n    A(Int)\n    }\n\
        fn f(e: E): Int {\n\
        switch (e) {\n\
        case .A(x, y): return x;\n\
        default: return -1;\n\
        }\n\
        }\n\
        fn Main(): Int {\n    return 0;\n    }\n",
        "binds 1 payloads, got 2",
        "arity",
    );
}

#[test]
fn contextual_member_literals_in_call_args() {
    let src = "enum Color {\n\
        Red,\n\
        Green,\n\
        Blue\n\
        }\n\
        enum Mode {\n\
        Read,\n\
        Write\n\
        }\n\
        fn colorToInt(c: Color): Int {\n\
        switch c {\n\
        case .Red: return 1;\n\
        case .Green: return 2;\n\
        case .Blue: return 3;\n\
        }\n\
        }\n\
        fn paint(c: Color): Int {\n\
        return colorToInt(c);\n\
        }\n\
        class Canvas {\n\
        let mode: Int = 0;\n\
        fn setMode(c: Color): Int {\n\
        return colorToInt(c) * 10;\n\
        }\n\
        static fn withColor(c: Color): Int {\n\
        return colorToInt(c) * 100;\n\
        }\n\
        init(c: Color) {\n\
        this.mode = colorToInt(c);\n\
        }\n\
        }\n\
        class File {\n\
        static fn open(path: String, m: Mode): Int {\n\
        switch m {\n\
        case .Read: return 11;\n\
        case .Write: return 22;\n\
        }\n\
        }\n\
        }\n\
        enum Outcome {\n\
        Ok(Int),\n\
        Err(String)\n\
        }\n\
        fn show(r: Outcome): Int {\n\
        switch r {\n\
        case .Ok(n): return n;\n\
        case .Err(e): return 0 - 1;\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let a: Color = .Red;\n\
        assert(paint(a) == 1, \"annot\");\n\
        assert(paint(.Green) == 2, \"free\");\n\
        let canvas = new Canvas(.Green);\n\
        assert(canvas.mode == 2, \"init\");\n\
        assert(canvas.setMode(.Blue) == 30, \"method\");\n\
        assert(Canvas.withColor(.Red) == 100, \"static\");\n\
        assert(File.open(\"p\", .Read) == 11, \"static2\");\n\
        assert(show(.Ok(41)) == 41, \"payload\");\n\
        assert(show(.Err(\"x\")) == 0 - 1, \"payload2\");\n\
        print(\"implicit-args-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["implicit-args-ok".to_string()], "implicit-args");
}

#[test]
fn unknown_implicit_variant_in_arg_rejected() {
    let dir = std::env::temp_dir().join(format!("rnx-tagunion-implicit-neg-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.rnx"),
        "enum Color {\n    Red,\n    Green,\n    Blue\n    }\n\
        fn paint(c: Color): Int {\n    return 1;\n    }\n\
        fn Main(): Int {\n    return paint(.NonExistent);\n    }\n",
    )
    .unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let err = lir::lower::lower(&m).expect_err("expected lowering error");
    assert!(err.message.contains("unknown variant"), "{err:?}");
    assert!(!err.message.contains("pending"), "{err:?}");
    let _ = std::fs::remove_dir_all(&dir);
}
