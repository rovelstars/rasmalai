use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-sba-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("sba_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

// Borrowed params alias without retaining: reassigning the alias must not
// free the caller's string (release-old on a borrow).
#[test]
fn borrow_copy_reassign_keeps_caller_alive() {
    check_all_backends(
        "fn head(s: String): String {\n\
        let out = s;\n\
        out = out.slice(0, 1);\n\
        return out;\n\
        }\n\
        fn Main(): Int {\n\
        let owned = \"abc\" + \"def\";\n\
        let r = head(owned);\n\
        assert(r == \"a\", \"head slice\");\n\
        assert(owned == \"abcdef\", \"caller intact\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "reassign",
    );
}

// Returning a borrow must hand the caller a live reference.
#[test]
fn borrow_copy_return_stays_live() {
    check_all_backends(
        "fn ident(s: String): String {\n\
        print(s.length());\n\
        let out = s;\n\
        return out;\n\
        }\n\
        fn Main(): Int {\n\
        let owned = \"abc\" + \"def\";\n\
        let r = ident(owned);\n\
        assert(r == owned, \"round trip\");\n\
        assert(owned == \"abcdef\", \"caller intact\");\n\
        return 0;\n\
        }\n",
        0,
        &["6".to_string()],
        "return",
    );
}

// Catch bindings are Error locals; on the no-throw path the binding is
// never assigned, so it must read as null rather than stack garbage.
#[test]
fn unassigned_error_binding_reads_null() {
    check_all_backends(
        "fn maybe(fail: Bool): Int {\n\
        try {\n\
        if (fail) {\n\
        throw \"boom\";\n\
        }\n\
        return 200;\n\
        } catch (err) {\n\
        return 500;\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        assert(maybe(false) == 200, \"no-throw path\");\n\
        assert(maybe(true) == 500, \"throw path\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "errnull",
    );
}
