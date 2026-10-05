use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
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

const BASIC_SRC: &str = "import { AtomicInt } from \"@std/sync\";\n\nfn inc() {\n    AtomicInt.byId(701).fetchAdd(1);\n}\n\nfn Main(): Int {\n    let pool = ThreadPool.byId(701, 4);\n    AtomicInt.byId(701).set(0);\n    let i = 0;\n    while i < 10 {\n        pool.submit(inc);\n        i = i + 1;\n    }\n    pool.join();\n    pool.shutdown();\n    return AtomicInt.byId(701).get();\n}\n";

#[test]
fn test_thread_pool_basic_tasks() {
    check_all_backends(BASIC_SRC, 10, &[], "poolbasic");
}

const PARFOR_SRC: &str = "import { AtomicInt } from \"@std/sync\";\n\nfn squareInto(i: Int) {\n    AtomicInt.byId(800 + i).set(i * i);\n}\n\nfn Main(): Int {\n    let pool = ThreadPool.byId(702, 4);\n    let k = 0;\n    while k < 1000 {\n        AtomicInt.byId(800 + k).set(0);\n        k = k + 1;\n    }\n    pool.parallelFor(0, 1000, 100, squareInto);\n    pool.join();\n    pool.shutdown();\n    let sum = 0;\n    let j = 0;\n    while j < 1000 {\n        sum = sum + AtomicInt.byId(800 + j).get();\n        j = j + 1;\n    }\n    if sum == 332833500 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_thread_pool_parallel_for() {
    check_all_backends(PARFOR_SRC, 42, &[], "poolfor");
}

const REUSE_SRC: &str = "import { AtomicInt } from \"@std/sync\";\n\nfn bump() {\n    AtomicInt.byId(703).fetchAdd(1);\n}\n\nfn runFive(): Int {\n    let pool = ThreadPool.byId(703, 2);\n    let i = 0;\n    while i < 5 {\n        pool.submit(bump);\n        i = i + 1;\n    }\n    pool.join();\n    pool.shutdown();\n    return AtomicInt.byId(703).get();\n}\n\nfn Main(): Int {\n    AtomicInt.byId(703).set(0);\n    let a = runFive();\n    let b = runFive();\n    if a == 5 && b == 10 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_thread_pool_shutdown_and_reuse() {
    // Deadline guard: pool join/shutdown must never hang the runner under
    // workspace contention. 60s is far above healthy runtime (AOT link
    // included) and only fires on genuine worker-drain starvation.
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let outcome = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            check_all_backends(REUSE_SRC, 42, &[], "poolreuse");
        }));
        let _ = tx.send(outcome);
    });
    match rx.recv_timeout(std::time::Duration::from_secs(60)) {
        Ok(Ok(())) => {}
        Ok(Err(payload)) => std::panic::resume_unwind(payload),
        Err(_) => panic!("Threadpool shutdown timed out after 60s - suspected worker drain starvation under heavy load"),
    }
}

const MATRIX_SRC: &str = "import { AtomicInt } from \"@std/sync\";\n\nfn doubleInto(i: Int) {\n    AtomicInt.byId(3000 + i).set(i * 2);\n}\n\nfn Main(): Int {\n    let pool = ThreadPool.byId(704, 4);\n    pool.parallelFor(0, 50, 10, doubleInto);\n    pool.join();\n    let s = 0;\n    let j = 0;\n    while j < 50 {\n        s = s + AtomicInt.byId(3000 + j).get();\n        j = j + 1;\n    }\n    pool.shutdown();\n    print(\"pool matrix:\", s);\n    if s == 2450 {\n        return 42;\n    }\n    return 1;\n}\n";

#[test]
fn test_cross_backend_pool_matrix() {
    check_all_backends(
        MATRIX_SRC,
        42,
        &["pool matrix: 2450".to_string()],
        "poolmatrix",
    );
}
