use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-anybox-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("anybox_bin");
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
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot {tag} stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn any_scalars_print_across_backends() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let opt_int: Int? = 42;\n\
        print(opt_int ?? -1);\n\
        let opt_bool: Bool? = true;\n\
        print(opt_bool ?? false);\n\
        let opt_float: Float? = 2.5;\n\
        print(opt_float ?? 0.0);\n\
        let arr = [100, 200];\n\
        let popped = arr.pop();\n\
        print(popped ?? -1);\n\
        let res: Result = Ok(7);\n\
        print(res.unwrap());\n\
        return 0;\n\
        }\n",
        "42\ntrue\n2.5\n200\n7\n",
        "scalars",
    );
}

#[test]
fn any_arithmetic_and_equality_still_work() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let o: Int? = 40;\n\
        let v = o ?? -1;\n\
        print(v + 2);\n\
        print(v == 40);\n\
        let w: Int = v;\n\
        print(w + 1);\n\
        return 0;\n\
        }\n",
        "42\ntrue\n41\n",
        "arith",
    );
}

#[test]
fn any_thread_and_channel_values_print() {
    check_stdout_everywhere(
        "import { Channel } from \"@std/sync\";\n\
        fn Main(): Int {\n\
        let handle = Thread.spawn(() => 1337);\n\
        let res = handle.join();\n\
        print(res.unwrap());\n\
        let ch = Channel.byId(77);\n\
        ch.send(41);\n\
        let v: Any = ch.recv();\n\
        print(v);\n\
        let pool = ThreadPool.new(2);\n\
        let t = pool.submit(() => 7);\n\
        print(t.join().unwrap());\n\
        pool.shutdown();\n\
        return 0;\n\
        }\n",
        "1337\n41\n7\n",
        "threads",
    );
}

#[test]
fn any_no_leak_on_roundtrip() {
    check_stdout_everywhere(
        "fn one(n: Int): Int {\n\
        let o: Int? = n;\n\
        let v = o ?? -1;\n\
        assert(v == n, \"roundtrip\");\n\
        return v;\n\
        }\n\
        fn Main(): Int {\n\
        let before = __rnx_debug_live_count();\n\
        let i = 0;\n\
        while i < 50 {\n\
            assert(one(i) == i, \"call\");\n\
            i = i + 1;\n\
        }\n\
        let after = __rnx_debug_live_count();\n\
        print(after - before);\n\
        return 0;\n\
        }\n",
        "0\n",
        "noleak",
    );
}
