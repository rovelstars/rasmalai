use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-stress-{tag}-{}", std::process::id()));
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
fn promise_resolve_churn_sequential() {
    check_all_backends(
        "async fn step(x: Int): Int {\n\
        let p = Promise.resolve(x * 2);\n\
        let got = await p;\n\
        return got + 1;\n\
        }\n\
        fn Main(): Int {\n\
        let total = 0;\n\
        let i = 0;\n\
        while i < 200 {\n\
        total = total + step(i).wait().unwrap();\n\
        i = i + 1;\n\
        }\n\
        print(total);\n\
        return 0;\n\
        }\n",
        0,
        &["40000".to_string()],
        "churn",
    );
}

#[test]
fn pool_submit_closure_churn_parallel() {
    check_all_backends(
        "import { AtomicInt } from \"@std/sync\";\n\
        fn Main(): Int {\n\
        AtomicInt.byId(9012).set(0);\n\
        let pool = ThreadPool.byId(9012, 8);\n\
        let w = 5;\n\
        let tag = \"s\";\n\
        let i = 0;\n\
        while i < 64 {\n\
        pool.submit((): Int => AtomicInt.byId(9012).fetchAdd(w + i + tag.len()));\n\
        i = i + 1;\n\
        }\n\
        pool.join();\n\
        pool.shutdown();\n\
        print(AtomicInt.byId(9012).get());\n\
        return 0;\n\
        }\n",
        0,
        &["2400".to_string()],
        "poolchurn",
    );
}

#[test]
fn spawn_join_closure_churn_sequential() {
    check_all_backends(
        "fn Main(): Int {\n\
        let total = 0;\n\
        let factor = 3;\n\
        let i = 0;\n\
        while i < 32 {\n\
        let handle = Thread.spawn(() => i * factor + 1);\n\
        let res = handle.join();\n\
        total = total + res.unwrap();\n\
        i = i + 1;\n\
        }\n\
        print(total);\n\
        return 0;\n\
        }\n",
        0,
        &["1520".to_string()],
        "spawnchurn",
    );
}
