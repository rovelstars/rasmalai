use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-cutover-{tag}-{}", std::process::id()));
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
    let bin_path = dir.join("cutover_bin");
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

fn check_src_fails(src: &str, tag: &str) -> (i32, String) {
    let dir = std::env::temp_dir().join(format!("rnx-cutover-neg-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    project: {\n        name: \"neg\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(dir.join("src/main.rnx"), src).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("check")
        .env("NO_COLOR", "1")
        .current_dir(&dir)
        .arg("src/main.rnx")
        .output()
        .unwrap();
    let text = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    let code = out.status.code().unwrap_or(-1);
    let _ = std::fs::remove_dir_all(&dir);
    (code, text)
}

#[test]
fn async_fn_result_flows_into_promise_typed_param() {
    check_all_backends(
        "fn take(p: Promise<Int>): Int {\n\
        let r: Int = p.wait().unwrap();\n\
        return r * 2;\n\
        }\n\
        async fn produce(x: Int): Int {\n\
        return x + 1;\n\
        }\n\
        fn Main(): Int {\n\
        let out = take(produce(20));\n\
        print(out);\n\
        return 0;\n\
        }\n",
        0,
        &["42".to_string()],
        "promise-param",
    );
}

#[test]
fn direct_poll_call_is_e111() {
    let (code, text) = check_src_fails(
        "async fn compute(): Int {\n    return 1;\n}\nfn Main(): Int {\n    let task = compute();\n    return task.poll();\n}\n",
        "poll",
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("E111"), "{text}");
    assert!(text.contains("poll()"), "{text}");
}

#[test]
fn poll_enum_reference_is_unresolved() {
    let (code, text) = check_src_fails(
        "fn Main(): Int {\n    let p = Poll.Pending;\n    return 0;\n}\n",
        "poll-enum",
    );
    assert_eq!(code, 1, "{text}");
    assert!(
        text.contains("E303") || text.contains("unresolved"),
        "{text}"
    );
}

#[test]
fn poll_type_declaration_is_e111() {
    let (code, text) = check_src_fails(
        "enum Poll {\n    Idle,\n}\nfn Main(): Int {\n    return 0;\n}\n",
        "poll-decl",
    );
    assert_eq!(code, 1, "{text}");
    assert!(text.contains("E111"), "{text}");
}

#[test]
fn multistep_await_chain_end_to_end() {
    check_all_backends(
        "async fn one(): Int {\n\
        return 1;\n\
        }\n\
        async fn add(a: Int, b: Int): Int {\n\
        let x = await one();\n\
        let y = await one();\n\
        return a + b + x + y;\n\
        }\n\
        async fn outer(): Int {\n\
        let m = await add(10, 20);\n\
        return m * 2;\n\
        }\n\
        fn Main(): Int {\n\
        let r: Int = outer().wait().unwrap();\n\
        print(r);\n\
        return 0;\n\
        }\n",
        0,
        &["64".to_string()],
        "chain",
    );
}
