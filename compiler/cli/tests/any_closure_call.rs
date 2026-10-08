use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-anyclo-{tag}-{}", std::process::id()));
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

    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0, "aot {tag} exit");
    assert_eq!(String::from_utf8(run.stdout).unwrap(), want, "aot {tag} stdout");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn any_closure_scalar_results_print() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let fi = (): Int => 42;\n\
        print(fi());\n\
        let fb = (): Bool => true;\n\
        print(fb());\n\
        let ff = (): Float => 2.5;\n\
        print(ff());\n\
        let fs = (): String => \"hi\";\n\
        print(fs());\n\
        return 0;\n\
        }\n",
        "42\ntrue\n2.5\nhi\n",
        "scalars",
    );
}

#[test]
fn any_closure_result_flows_through_locals() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let f = (): Int => 42;\n\
        let r = f();\n\
        print(r);\n\
        let a: Any = f();\n\
        print(a);\n\
        assert(f() == 42, \"boxed exactly once\");\n\
        print(((): Int => 7)());\n\
        return 0;\n\
        }\n",
        "42\n42\n7\n",
        "locals",
    );
}

#[test]
fn any_closure_result_nested_calls() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let f = (): Int => 42;\n\
        let g = (): Any => f();\n\
        print(g());\n\
        let h = (x: Any): Any => x;\n\
        print(h(f()));\n\
        let a: Array<Any> = [f(), g()];\n\
        print(a[0]);\n\
        print(a[1]);\n\
        return 0;\n\
        }\n",
        "42\n42\n42\n42\n",
        "nested",
    );
}

#[test]
fn any_closure_result_object_and_record() {
    check_stdout_everywhere(
        "class P {\n\
        let x: Int;\n\
        }\n\
        fn Main(): Int {\n\
        let f = (): P => new P(1);\n\
        print(f());\n\
        let r = { cb: (): Int => 7 };\n\
        print(r.cb());\n\
        return 0;\n\
        }\n",
        "P{x: 1}\n7\n",
        "objrec",
    );
}

#[test]
fn any_stored_closure_map_and_result_map() {
    check_stdout_everywhere(
        "fn Main(): Int {\n\
        let nums = [1, 2, 3];\n\
        let triple = (x: Int): Int => x * 3;\n\
        let out = nums.map(triple);\n\
        print(out[2]);\n\
        let ok: Result = Ok(7);\n\
        let m = ok.map((x: Int): Int => x + 1);\n\
        print(m.unwrap());\n\
        return 0;\n\
        }\n",
        "9\n8\n",
        "higher",
    );
}
