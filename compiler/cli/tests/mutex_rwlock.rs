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

    let bin_path = dir.join("lock_bin");
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

const COUNTER_SRC: &str = "import { AtomicInt, Mutex } from \"@std/sync\";\nfn worker(): Int {\n    let c = AtomicInt.byId(511);\n    let m = Mutex.byId(511);\n    let i = 0;\n    while i < 500 {\n        m.lock();\n        c.set(c.get() + 1);\n        m.unlock();\n        i = i + 1;\n    }\n    return 0;\n}\nfn Main(): Int {\n    let c = AtomicInt.byId(511);\n    c.set(0);\n    let h1 = Thread.spawn(worker);\n    let h2 = Thread.spawn(worker);\n    let h3 = Thread.spawn(worker);\n    let h4 = Thread.spawn(worker);\n    h1.join();\n    h2.join();\n    h3.join();\n    h4.join();\n    if c.get() == 2000 {\n        print(\"mutex counter:\", c.get());\n        return 42;\n    }\n    return 0;\n}\n";

#[test]
fn test_mutex_concurrent_counter() {
    check_all_backends(COUNTER_SRC, 42, &["mutex counter: 2000".to_string()], "mutexctr");
}

const TRY_SRC: &str = "import { AtomicInt, Mutex } from \"@std/sync\";\nfn helper(): Int {\n    let m = Mutex.byId(512);\n    let sig = AtomicInt.byId(512);\n    let first = m.tryLock();\n    if first {\n        sig.set(1);\n    } else {\n        sig.set(2);\n    }\n    while sig.get() == 2 {\n    }\n    let second = m.tryLock();\n    if second {\n        m.unlock();\n        sig.set(4);\n    } else {\n        sig.set(5);\n    }\n    return 0;\n}\nfn Main(): Int {\n    let m = Mutex.byId(512);\n    let sig = AtomicInt.byId(512);\n    sig.set(0);\n    m.lock();\n    let h = Thread.spawn(helper);\n    while sig.get() == 0 {\n    }\n    if sig.get() != 2 {\n        m.unlock();\n        h.join();\n        return 1;\n    }\n    m.unlock();\n    sig.set(3);\n    while sig.get() == 3 {\n    }\n    h.join();\n    if sig.get() == 4 {\n        print(\"mutex trylock ok\");\n        return 42;\n    }\n    return 2;\n}\n";

#[test]
fn test_mutex_try_lock() {
    check_all_backends(TRY_SRC, 42, &["mutex trylock ok".to_string()], "mutextry");
}

const READERS_SRC: &str = "import { AtomicInt, RwLock, Channel } from \"@std/sync\";\nfn reader(): Int {\n    let rw = RwLock.byId(513);\n    let live = AtomicInt.byId(513);\n    rw.readLock();\n    live.fetchAdd(1);\n    while live.get() < 4 {\n    }\n    rw.readUnlock();\n    return 0;\n}\nfn writer(): Int {\n    let rw = RwLock.byId(514);\n    let ch = Channel.byId(515);\n    rw.writeLock();\n    ch.send(1);\n    rw.writeUnlock();\n    return 0;\n}\nfn Main(): Int {\n    let live = AtomicInt.byId(513);\n    live.set(0);\n    let h1 = Thread.spawn(reader);\n    let h2 = Thread.spawn(reader);\n    let h3 = Thread.spawn(reader);\n    let h4 = Thread.spawn(reader);\n    h1.join();\n    h2.join();\n    h3.join();\n    h4.join();\n    if live.get() != 4 {\n        return 1;\n    }\n    let rw = RwLock.byId(514);\n    let ch = Channel.byId(515);\n    rw.readLock();\n    let hw = Thread.spawn(writer);\n    if ch.len() != 0 {\n        rw.readUnlock();\n        hw.join();\n        return 2;\n    }\n    rw.readUnlock();\n    let v = ch.recv();\n    hw.join();\n    if v == 1 {\n        print(\"rwlock ok\");\n        return 42;\n    }\n    return 3;\n}\n";

#[test]
fn test_rwlock_concurrent_readers() {
    check_all_backends(READERS_SRC, 42, &["rwlock ok".to_string()], "rwreaders");
}
