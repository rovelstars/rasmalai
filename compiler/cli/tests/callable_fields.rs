use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-callfield-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("callfield_bin");
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

#[test]
fn record_closure_field_direct_call() {
    let src = "fn Main(): Int {\n\
        let worker = { id: 1, run: () => { return 42; } };\n\
        assert(worker.run() == 42, \"direct\");\n\
        assert(worker.id == 1, \"field\");\n\
        let r = worker.run;\n\
        assert(r() == 42, \"extracted\");\n\
        print(\"call-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["call-ok".to_string()], "direct");
}

#[test]
fn class_closure_field_direct_call() {
    let src = "class Worker {\n\
        let run: fn(): Int;\n\
        init(f: fn(): Int) { this.run = f; }\n\
        fn id(): Int { return 1; }\n\
        }\n\
        fn Main(): Int {\n\
        let w = new Worker(() => { return 7; });\n\
        assert(w.run() == 7, \"field call\");\n\
        assert(w.id() == 1, \"method\");\n\
        print(\"class-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["class-ok".to_string()], "class");
}

#[test]
fn record_pretty_print() {
    let src = "fn Main(): Int {\n\
        let worker = { id: 1, name: \"worker\" };\n\
        print(worker);\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["{ id: 1, name: \"worker\" }".to_string()], "pretty");
}

#[test]
fn record_pretty_print_nested_and_fn() {
    let src = "fn Main(): Int {\n\
        let worker = { id: 1, name: \"reactor-1\", run: () => { return 42; } };\n\
        print(worker);\n\
        let nested = { a: { b: 42 }, flag: true };\n\
        print(nested);\n\
        return 0;\n\
        }\n";
    check_all_backends(
        src,
        0,
        &[
            "{ id: 1, name: \"reactor-1\", run: <fn> }".to_string(),
            "{ a: { b: 42 }, flag: true }".to_string(),
        ],
        "nested",
    );
}
