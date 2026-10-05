use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-exc-{tag}-{}", std::process::id()));
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
fn cross_function_throws_propagate() {
    check_all_backends(
        "fn leaf(ok: Bool): Int throws {\n\
        if ok { return 42; }\n\
        throw 0;\n\
        }\n\
        fn mid(ok: Bool): Int throws {\n\
        return leaf(ok);\n\
        }\n\
        fn root(ok: Bool): Int throws {\n\
        return mid(ok);\n\
        }\n\
        fn Main(): Int {\n\
        try {\n\
        print(root(true));\n\
        } catch (err) {\n\
        print(\"failed\");\n\
        }\n\
        try {\n\
        print(root(false));\n\
        } catch (err) {\n\
        print(\"failed\");\n\
        }\n\
        return 0;\n\
        }\n",
        0,
        &["42".to_string(), "failed".to_string()],
        "depth3",
    );
}

#[test]
fn class_payload_is_dispatch() {
    check_all_backends(
        "class ParseError {\n\
        let message: String;\n\
        init(message: String) { this.message = message; }\n\
        }\n\
        class IoError {\n\
        let message: String;\n\
        init(message: String) { this.message = message; }\n\
        }\n\
        fn Main(): Int {\n\
        try {\n\
        throw new ParseError(\"bad header\");\n\
        } catch (err) {\n\
        if err is ParseError {\n\
        print(\"parse\");\n\
        } else {\n\
        print(\"other\");\n\
        }\n\
        }\n\
        try {\n\
        throw new IoError(\"disk gone\");\n\
        } catch (err) {\n\
        if err is ParseError {\n\
        print(\"parse\");\n\
        } else if err is IoError {\n\
        print(\"io\");\n\
        } else {\n\
        print(\"other\");\n\
        }\n\
        }\n\
        return 0;\n\
        }\n",
        0,
        &["parse".to_string(), "io".to_string()],
        "is-dispatch",
    );
}

#[test]
fn caught_fields_accessible_after_narrow() {
    check_all_backends(
        "class ValidationError {\n\
        let field: String;\n\
        let code: Int;\n\
        init(field: String, code: Int) { this.field = field; this.code = code; }\n\
        }\n\
        fn check(v: Int): Int throws {\n\
        if v < 0 {\n\
        throw new ValidationError(\"v\", v);\n\
        }\n\
        return v;\n\
        }\n\
        fn Main(): Int {\n\
        try {\n\
        print(check(-7));\n\
        } catch (err) {\n\
        if err is ValidationError {\n\
        print(err.field);\n\
        print(err.code);\n\
        }\n\
        }\n\
        print(check(3));\n\
        return 0;\n\
        }\n",
        0,
        &["v".to_string(), "-7".to_string(), "3".to_string()],
        "fields",
    );
}

#[test]
fn thrown_subclass_matches_base_check() {
    check_all_backends(
        "class Base {\n\
        let x: Int;\n\
        init(x: Int) { this.x = x; }\n\
        }\n\
        class Child extends Base {\n\
        init(x: Int) { super(x); }\n\
        }\n\
        fn Main(): Int {\n\
        try {\n\
        throw new Child(9);\n\
        } catch (err) {\n\
        if err is Base {\n\
        print(\"base\");\n\
        }\n\
        if err is Child {\n\
        print(\"child\");\n\
        }\n\
        }\n\
        return 0;\n\
        }\n",
        0,
        &["base".to_string(), "child".to_string()],
        "subtype",
    );
}

#[test]
fn top_level_try_catch() {
    check_all_backends(
        "print(\"before\");\n\
        try {\n\
        throw \"down\";\n\
        } catch (err) {\n\
        print(\"in catch\");\n\
        }\n\
        print(\"after\");\n",
        0,
        &[
            "before".to_string(),
            "in catch".to_string(),
            "after".to_string(),
        ],
        "toplevel",
    );
}

#[test]
fn async_try_catch() {
    check_all_backends(
        "async fn worker(ok: Bool): Int {\n\
        try {\n\
        if ok {\n\
        return 7;\n\
        }\n\
        throw \"down\";\n\
        } catch (err) {\n\
        return 99;\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        print(worker(true).wait().unwrapOr(-1));\n\
        print(worker(false).wait().unwrapOr(-1));\n\
        return 0;\n\
        }\n",
        0,
        &["7".to_string(), "99".to_string()],
        "async-catch",
    );
}

#[test]
fn string_concat_over_caught_error() {
    check_all_backends(
        "fn Main(): Int {\n\
        try {\n\
        throw \"down\";\n\
        } catch (err) {\n\
        print(\"caught: \" + err);\n\
        }\n\
        return 0;\n\
        }\n",
        0,
        &["caught: down".to_string()],
        "concat",
    );
}

#[test]
fn uncaught_string_error_fails_everywhere() {
    let src = "fn boom(): Int throws {\n\
        throw \"kaboom\";\n\
        }\n\
        fn Main(): Int {\n\
        print(boom());\n\
        return 0;\n\
        }\n";
    let (module, dir) = resolve_src(src, "uncaught");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    assert!(machine.call("Main", vec![]).is_err(), "interpreter uncaught");
    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    let err = jit.call("Main", &[]).expect_err("cranelift uncaught");
    assert!(err.message.contains("kaboom"), "{err:?}");
    let err = llvm::codegen::execute(leaked, "Main").expect_err("llvm uncaught");
    assert!(err.message.contains("kaboom"), "{err:?}");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 1, "aot uncaught exit");
    assert!(
        String::from_utf8(run.stderr).unwrap().contains("kaboom"),
        "aot uncaught stderr"
    );
    let _ = std::fs::remove_dir_all(&dir);
}
