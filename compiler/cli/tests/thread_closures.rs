use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-thrclo-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("thrclo_bin");
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
fn thread_spawn_closure_join_result() {
    check_all_backends(
        "fn Main(): Int {\n\
        let factor = 2;\n\
        let handle = Thread.spawn(() => 21 * factor);\n\
        let res = handle.join();\n\
        assert(res.isOk(), \"thread ok\");\n\
        assert(res.unwrap() == 42, \"thread calculated value\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "spawn",
    );
}

#[test]
fn thread_spawn_fn_join_result() {
    check_all_backends(
        "fn worker(): Int {\n\
        return 7;\n\
        }\n\
        fn Main(): Int {\n\
        let handle = Thread.spawn(worker);\n\
        let res = handle.join();\n\
        assert(res.isOk(), \"fn ok\");\n\
        assert(res.unwrap() == 7, \"fn value\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "spawnfn",
    );
}

#[test]
fn pool_submit_closure_await_result() {
    check_all_backends(
        "fn Main(): Int {\n\
        let pool = ThreadPool.new(4);\n\
        let task = pool.submit(() => \"done\".concat(\"!\"));\n\
        let out = task.join();\n\
        assert(out.isOk(), \"pool ok\");\n\
        let s: String = out.unwrap();\n\
        assert(s == \"done!\", \"pool finished\");\n\
        pool.shutdown();\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "pool",
    );
}

#[test]
fn pool_submit_fn_await_result() {
    check_all_backends(
        "fn work(): Int {\n\
        return 11;\n\
        }\n\
        fn Main(): Int {\n\
        let pool = ThreadPool.byId(4242, 2);\n\
        let task = pool.submit(work);\n\
        let out = task.join();\n\
        assert(out.unwrap() == 11, \"fn task\");\n\
        pool.shutdown();\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "poolfn",
    );
}

#[test]
fn pool_parallel_for_closure() {
    check_all_backends(
        "import { AtomicInt } from \"@std/sync\";\n\
        fn Main(): Int {\n\
        let pool = ThreadPool.new(4);\n\
        AtomicInt.byId(7171).set(0);\n\
        pool.parallelFor(0, 10, 5, (i: Int) => { AtomicInt.byId(7171).fetchAdd(1); });\n\
        pool.shutdown();\n\
        assert(AtomicInt.byId(7171).get() == 10, \"parfor\");\n\
        return 0;\n\
        }\n",
        0,
        &[],
        "parfor",
    );
}
