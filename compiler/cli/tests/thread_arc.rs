use std::path::PathBuf;

// Serializes this binary's tests: two of them assert on the
// process-global allocator live count, which concurrent sibling tests
// would perturb. Not a race workaround.
static LIVE_COUNT_GUARD: std::sync::Mutex<()> = std::sync::Mutex::new(());

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
    let _guard = LIVE_COUNT_GUARD.lock().unwrap_or_else(|e| e.into_inner());
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

const HANDOFF_SRC: &str = "import { Channel } from \"@std/sync\";\n\nclass Message {\n    let text: String;\n    let code: Int;\n    init(t: String, c: Int) {\n        this.text = t;\n        this.code = c;\n    }\n}\n\nfn worker(): Int {\n    let chan = Channel.byId(101);\n    let msg = chan.recv() as Message;\n    print(\"Worker received:\", msg.text, msg.code);\n    return msg.code;\n}\n\nfn Main(): Int {\n    let chan = Channel.byId(101);\n    let th = Thread.spawn(worker);\n    let payload = new Message(\"ping\", 42);\n    chan.send(payload);\n    th.join();\n    return 42;\n}\n";

#[test]
fn test_channel_object_handoff() {
    check_all_backends(HANDOFF_SRC, 42, &["Worker received: ping 42".to_string()], "handoff");
}

const STRESS_SRC: &str = "import { Channel } from \"@std/sync\";\n\nclass Box {\n    let val: Int;\n    let items: Array<Int>;\n    init(v: Int, items: Array<Int>) {\n        this.val = v;\n        this.items = items;\n    }\n}\n\nfn worker(): Int {\n    let inch = Channel.byId(202);\n    let outch = Channel.byId(203);\n    let arrch = Channel.byId(204);\n    let b = inch.recv() as Box;\n    let total = b.val + b.items.length;\n    let arr = arrch.recv() as Array<Int>;\n    outch.send(b);\n    arrch.send(arr);\n    return total;\n}\n\nfn runRound(): Int {\n    let inch = Channel.byId(202);\n    let outch = Channel.byId(203);\n    let arrch = Channel.byId(204);\n    let b = new Box(0, [1, 2, 3]);\n    let h1 = Thread.spawn(worker);\n    let h2 = Thread.spawn(worker);\n    let h3 = Thread.spawn(worker);\n    let h4 = Thread.spawn(worker);\n    inch.send(b);\n    inch.send(b);\n    inch.send(b);\n    inch.send(b);\n    arrch.send([10, 20]);\n    arrch.send([30, 40]);\n    arrch.send([50, 60]);\n    arrch.send([70, 80]);\n    let sum = h1.join().unwrap() + h2.join().unwrap() + h3.join().unwrap() + h4.join().unwrap();\n    let r1 = outch.recv() as Box;\n    let r2 = outch.recv() as Box;\n    let r3 = outch.recv() as Box;\n    let r4 = outch.recv() as Box;\n    let boxes = r1.val + r2.val + r3.val + r4.val;\n    let a1 = arrch.recv() as Array<Int>;\n    let a2 = arrch.recv() as Array<Int>;\n    let a3 = arrch.recv() as Array<Int>;\n    let a4 = arrch.recv() as Array<Int>;\n    let lens = a1.length + a2.length + a3.length + a4.length;\n    if sum == 12 && boxes == 0 && lens == 8 {\n        return 0;\n    }\n    return 1;\n}\n\nfn Main(): Int {\n    let before = __rnx_debug_live_count();\n    let code = runRound();\n    let after = __rnx_debug_live_count();\n    if code == 0 && after == before {\n        return 0;\n    }\n    return 1;\n}\n";

#[test]
fn test_cross_thread_arc_stress_race() {
    check_all_backends(STRESS_SRC, 0, &[], "stress");
}

const DROP_SRC: &str = "import { Channel } from \"@std/sync\";\n\nclass Message {\n    let text: String;\n    let code: Int;\n    init(t: String, c: Int) {\n        this.text = t;\n        this.code = c;\n    }\n}\n\nfn fill(chan: Channel) {\n    chan.send(new Message(\"a\", 1));\n    chan.send(new Message(\"b\", 2));\n    chan.send(new Message(\"c\", 3));\n    chan.send(new Message(\"d\", 4));\n    chan.send(new Message(\"e\", 5));\n}\n\nfn Main(): Int {\n    let chan = Channel.byId(303);\n    let before = __rnx_debug_live_count();\n    fill(chan);\n    if chan.len() != 5 {\n        return 1;\n    }\n    chan.close();\n    if chan.len() != 0 {\n        return 2;\n    }\n    let after = __rnx_debug_live_count();\n    if after == before {\n        return 0;\n    }\n    return 3;\n}\n";

#[test]
fn test_channel_drop_cleans_queued_objects() {
    check_all_backends(DROP_SRC, 0, &[], "dropclean");
}
