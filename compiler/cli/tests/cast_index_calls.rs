use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-castidx-{tag}-{}", std::process::id()));
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

    let bin_path = dir.join("castidx_bin");
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
    match run.status.code() {
        Some(code) => assert_eq!(code, want as i32, "aot {tag} exit"),
        None => panic!("aot {tag} killed by signal: {run:?}"),
    }
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const CAST_CHAIN_SRC: &str = "import { Channel } from \"@std/sync\";\nconst CH_ID: Int = 6201;\nfn Main(): Int {\n    let ch = Channel.byId(CH_ID);\n    ch.send(7);\n    let v = Channel.byId(CH_ID).recv() as Int;\n    print(v);\n    return 0;\n}\n";

#[test]
fn test_cast_on_imported_static_call_chain() {
    check_all_backends(CAST_CHAIN_SRC, 0, &["7".to_string()], "castchain");
}

const INDEX_CALL_SRC: &str = "class Task {\n    let id: Int;\n    init(id: Int) { this.id = id; }\n    run(): Int { return this.id; }\n}\nfn makeTask(id: Int): Task { return new Task(id); }\nfn Main(): Int {\n    let tasks: Array<Task> = [makeTask(1), makeTask(2)];\n    let r = tasks[0].run() + tasks[1].run();\n    print(r);\n    return 0;\n}\n";

#[test]
fn test_method_call_on_array_element() {
    check_all_backends(INDEX_CALL_SRC, 0, &["3".to_string()], "indexcall");
}

const ANY_INDEX_SRC: &str = "class Task {\n    let id: Int;\n    init(id: Int) { this.id = id; }\n    run(): Int { return this.id; }\n}\nfn makeTask(id: Int): Task { return new Task(id); }\nfn Main(): Int {\n    let hs = [];\n    hs.push(makeTask(1));\n    let r = hs[0].run();\n    print(r);\n    return 0;\n}\n";

#[test]
fn test_untyped_array_element_call_reports_true_span() {
    let dir = std::env::temp_dir().join(format!("rnx-castidx-anyspan-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, ANY_INDEX_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(dir.join("any_bin"))
        .env("NO_COLOR", "1")
        .output()
        .unwrap();
    assert!(!build.status.success(), "untyped element call should fail native builds");
    let stdout = String::from_utf8_lossy(&build.stdout).into_owned();
    let stderr = String::from_utf8_lossy(&build.stderr).into_owned();
    let stderr = stdout + &stderr;
    assert!(stderr.contains("`run`"), "diagnostic must name the method: {stderr}");
    assert!(stderr.contains("10:13"), "diagnostic must point at the call, not 1:1: {stderr}");
    assert!(!stderr.contains("1:1"), "stale import-line span: {stderr}");
    let _ = std::fs::remove_dir_all(&dir);
}
