use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-issue0045-{tag}-{}", std::process::id()));
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

fn run_backend(rnx: &str, dir: &PathBuf, backend: &str) -> String {
    let out = std::process::Command::new(rnx)
        .arg("run")
        .arg("--backend")
        .arg(backend)
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(out.status.success(), "{backend}: {}", String::from_utf8_lossy(&out.stderr));
    String::from_utf8(out.stdout).unwrap()
}

fn check_stdout_everywhere(src: &str, want: &str, tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(0) => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    assert_eq!(machine.output.join("\n") + "\n", want, "interpreter {tag} stdout");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    for backend in ["cranelift", "llvm"] {
        let got = run_backend(rnx, &dir, backend);
        assert_eq!(got, want, "{backend} {tag} stdout");
    }

    let bin_path = dir.join("issue0045_bin");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot {tag} exit");
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn closure_spawn_atomic_rounds_complete() {
    check_stdout_everywhere(
        "import { AtomicInt, Thread } from \"@std/sync\";\n\
        fn workerN(n: Int): Int {\n\
            return AtomicInt.byId(910).fetchAdd(1);\n\
        }\n\
        fn round(n: Int): Int {\n\
            AtomicInt.byId(910).set(0);\n\
            let hs: Array<Thread> = [];\n\
            for _ in 0..n { hs.push(Thread.spawn((): Int => workerN(n))); }\n\
            for t in hs { t.join(); }\n\
            return AtomicInt.byId(910).get();\n\
        }\n\
        fn Main(): Int {\n\
            let i = 0;\n\
            while i < 12 {\n\
                if round(16) != 16 { return 1; }\n\
                i++;\n\
            }\n\
            print(\"done\");\n\
            return 0;\n\
        }\n",
        "done\n",
        "closure-atomic",
    );
}
