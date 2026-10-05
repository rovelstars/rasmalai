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

const PRODCONS_SRC: &str = "import { AtomicInt, Mutex, Condvar } from \"@std/sync\";\nfn worker(): Int {\n    let m = Mutex.byId(601);\n    let cv = Condvar.byId(601);\n    let produced = AtomicInt.byId(601);\n    let consumed = AtomicInt.byId(602);\n    let ready = AtomicInt.byId(603);\n    let my = 0;\n    m.lock();\n    ready.set(1);\n    while produced.get() == 0 {\n        cv.wait(m);\n    }\n    m.unlock();\n    while my < 5 {\n        m.lock();\n        while consumed.get() >= produced.get() {\n            cv.wait(m);\n        }\n        consumed.fetchAdd(1);\n        my = my + 1;\n        m.unlock();\n    }\n    return 0;\n}\nfn Main(): Int {\n    let m = Mutex.byId(601);\n    let cv = Condvar.byId(601);\n    let produced = AtomicInt.byId(601);\n    let consumed = AtomicInt.byId(602);\n    let ready = AtomicInt.byId(603);\n    produced.set(0);\n    consumed.set(0);\n    ready.set(0);\n    let h = Thread.spawn(worker);\n    while ready.get() == 0 {\n    }\n    let i = 0;\n    while i < 5 {\n        m.lock();\n        produced.fetchAdd(1);\n        cv.notifyOne();\n        m.unlock();\n        i = i + 1;\n    }\n    h.join();\n    if produced.get() == 5 && consumed.get() == 5 {\n        print(\"condvar ok\");\n        return 42;\n    }\n    return 0;\n}\n";

#[test]
fn test_condvar_producer_consumer() {
    check_all_backends(PRODCONS_SRC, 42, &["condvar ok".to_string()], "condprod");
}

const TIMEOUT_SRC: &str = "import { Mutex, Condvar } from \"@std/sync\";\nimport { Clock } from \"@std/time\";\nfn Main(): Int {\n    let m = Mutex.byId(604);\n    let cv = Condvar.byId(604);\n    m.lock();\n    let t0 = Clock.mono().toMillis();\n    let r = cv.waitTimeout(m, 20);\n    let dt = Clock.mono().toMillis() - t0;\n    m.unlock();\n    if r {\n        return 1;\n    }\n    if dt > 5000 {\n        return 2;\n    }\n    print(\"timeout ok\");\n    return 42;\n}\n";

#[test]
fn test_condvar_wait_timeout() {
    check_all_backends(TIMEOUT_SRC, 42, &["timeout ok".to_string()], "condtime");
}

const BARRIER_SRC: &str = "import { AtomicInt, Barrier } from \"@std/sync\";\nfn worker(): Int {\n    let b = Barrier.byId(10, 4);\n    let leaders = AtomicInt.byId(611);\n    let followers = AtomicInt.byId(612);\n    let arrived = AtomicInt.byId(613);\n    if b.wait() {\n        leaders.fetchAdd(1);\n    } else {\n        followers.fetchAdd(1);\n    }\n    arrived.fetchAdd(1);\n    return 0;\n}\nfn Main(): Int {\n    let leaders = AtomicInt.byId(611);\n    let followers = AtomicInt.byId(612);\n    let arrived = AtomicInt.byId(613);\n    leaders.set(0);\n    followers.set(0);\n    arrived.set(0);\n    let h1 = Thread.spawn(worker);\n    let h2 = Thread.spawn(worker);\n    let h3 = Thread.spawn(worker);\n    let h4 = Thread.spawn(worker);\n    h1.join();\n    h2.join();\n    h3.join();\n    h4.join();\n    if leaders.get() == 1 && followers.get() == 3 && arrived.get() == 4 {\n        print(\"barrier ok\");\n        return 42;\n    }\n    return 0;\n}\n";

#[test]
fn test_barrier_rendezvous() {
    check_all_backends(BARRIER_SRC, 42, &["barrier ok".to_string()], "barrier");
}

const MATRIX_SRC: &str = "import { AtomicInt, Mutex, Condvar, Barrier } from \"@std/sync\";\nfn worker(): Int {\n    let m = Mutex.byId(621);\n    let cv = Condvar.byId(621);\n    let b = Barrier.byId(622, 2);\n    let total = AtomicInt.byId(621);\n    let ready = AtomicInt.byId(623);\n    m.lock();\n    ready.set(1);\n    while total.get() == 0 {\n        cv.wait(m);\n    }\n    m.unlock();\n    if b.wait() {\n        total.fetchAdd(100);\n    } else {\n        total.fetchAdd(10);\n    }\n    return 0;\n}\nfn Main(): Int {\n    let m = Mutex.byId(621);\n    let cv = Condvar.byId(621);\n    let b = Barrier.byId(622, 2);\n    let total = AtomicInt.byId(621);\n    let ready = AtomicInt.byId(623);\n    total.set(0);\n    ready.set(0);\n    let h = Thread.spawn(worker);\n    while ready.get() == 0 {\n    }\n    m.lock();\n    total.fetchAdd(1);\n    cv.notifyAll();\n    m.unlock();\n    if b.wait() {\n        total.fetchAdd(1000);\n    } else {\n        total.fetchAdd(1);\n    }\n    h.join();\n    if total.get() == 102 || total.get() == 1011 {\n        print(\"sync matrix: 42\");\n        return 42;\n    }\n    return 0;\n}\n";

#[test]
fn test_sync_matrix_cross_backend() {
    check_all_backends(MATRIX_SRC, 42, &["sync matrix: 42".to_string()], "syncmat");
}

const BARRIER_CYCLIC_SRC: &str = "import { AtomicInt, Barrier } from \"@std/sync\";\nfn worker(): Int {\n    let b = Barrier.byId(940, 2);\n    let leaders = AtomicInt.byId(941);\n    let i = 0;\n    while i < 3 {\n        if b.wait() {\n            leaders.fetchAdd(1);\n        }\n        i = i + 1;\n    }\n    return 0;\n}\nfn Main(): Int {\n    let b = Barrier.byId(940, 2);\n    let leaders = AtomicInt.byId(941);\n    leaders.set(0);\n    let h = Thread.spawn(worker);\n    let i = 0;\n    while i < 3 {\n        if b.wait() {\n            leaders.fetchAdd(1);\n        }\n        i = i + 1;\n    }\n    h.join();\n    if leaders.get() == 3 {\n        print(\"barrier cyclic ok\");\n        return 42;\n    }\n    return 0;\n}\n";

#[test]
fn test_barrier_cyclic_reset() {
    check_all_backends(BARRIER_CYCLIC_SRC, 42, &["barrier cyclic ok".to_string()], "barcyc");
}

#[test]
fn test_barrier_zero_and_negative_rejected() {
    for (tag, count) in [("barzero", 0), ("barneg", -2)] {
        let dir = std::env::temp_dir().join(format!("rnx-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let main = dir.join("main.rnx");
        let src = format!("import {{ Barrier }} from \"@std/sync\";\nfn Main(): Int {{\n    let b = Barrier.byId(941, {count});\n    print(b.wait());\n    return 0;\n}}\n");
        std::fs::write(&main, src).unwrap();
        let rnx = env!("CARGO_BIN_EXE_rnx");
        let out = std::process::Command::new(rnx)
            .arg("run")
            .arg(&main)
            .output()
            .unwrap();
        let text = String::from_utf8_lossy(&out.stdout).into_owned()
            + &String::from_utf8_lossy(&out.stderr);
        assert!(
            text.contains("Barrier parties must be greater than 0, got"),
            "{tag}: expected parties validation, got {text:?}"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}

const THREAD_ARRAY_SRC: &str = "import { Thread } from \"@std/sync\";\nfn worker(): Int {\n    return 40 + 2;\n}\nfn Main(): Int {\n    let workers: Array<Thread> = [];\n    workers.push(Thread.spawn(worker));\n    workers.push(Thread.spawn(worker));\n    workers.push(Thread.spawn(worker));\n    let total = 0;\n    for t in workers {\n        total = total + (t.join().unwrap() as Int);\n    }\n    if total == 126 {\n        print(\"thread array ok\");\n        return 42;\n    }\n    return 0;\n}\n";

#[test]
fn test_thread_array_typed_join() {
    check_all_backends(THREAD_ARRAY_SRC, 42, &["thread array ok".to_string()], "thdarr");
}
