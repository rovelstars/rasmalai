use cli::{check_source, is_ok, run_source, RunOutcome};
use runtime::value::Value;

#[test]
fn check_accepts_self_contained() {
    for src in [
        "fn Main(): Int { return 1 + 2 }",
        "class P { let x: Int init(x: Int) { this.x = x } fn get(): Int { return this.x } } fn Main(): Int { let p = new P(3) return p.get() }",
        "fn F(x: Int): Int { try { return 1 } catch (err) { return 0 } } fn Main(): Int { return F(0) }",
    ] {
        let r = check_source(src);
        assert!(is_ok(&r), "{:?}", r.errors);
    }
}

#[test]
fn check_rejects_e110_and_e305() {
    let r = check_source("fn F() { for (let i = 0; i < 3; i += 1) { } }");
    assert!(r.errors.iter().any(|d| d.code.as_str() == "E110"));
    let r = check_source("fn F(a: Float, b: FastFloat): Float { return a * b }");
    assert!(r.errors.iter().any(|d| d.code.as_str() == "E305"));
}

#[test]
fn run_fib_e2e() {
    let src = "fn Main(n: Int): Int { let a = 0 let b = 1 let i = 0 while i < n { let t = a + b a = b b = t i += 1 } return a }";
    let out = run_source(src, "Main", vec![Value::Int(10)]);
    match out.outcome {
        RunOutcome::Value(Value::Int(55)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn run_print_and_throw() {
    let src = "fn Main(): Int { print(\"hi\") throw 3 }";
    let out = run_source(src, "Main", vec![]);
    assert_eq!(out.output, vec!["hi".to_string()]);
    assert!(matches!(out.outcome, RunOutcome::Thrown(_)));
}

#[test]
fn run_arrays_task_program() {
    let src = "fn Main(): Int { let list = [10, 20, 30]; list.push(40); let sum = 0; for x in list { sum = sum + x; } if sum == 100 && list.length == 4 { return 42; } return 0; }";
    let out = run_source(src, "Main", vec![]);
    match out.outcome {
        RunOutcome::Value(Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn run_float_task_program() {
    let src = "fn Main(): Int { let a: Float = 10.5; let b: Float = 20.25; let c = a * 2.0 + b; let fast_c = c.asFast(); let scaled = fast_c * 2.0.asFast(); let result = Int(scaled.asStrict()) / 2; if result == 41 { print(\"Float math verified:\", c); return 42; } return 0; }";
    let out = run_source(src, "Main", vec![]);
    match out.outcome {
        RunOutcome::Value(Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    assert!(out.output.iter().any(|l| l.contains("41.25")));
}

#[test]
fn check_rejects_float_fast_mix() {
    let rep = check_source("fn F(a: Float, b: FastFloat): Float { return a + b }");
    assert!(!is_ok(&rep));
    assert!(rep.errors.iter().any(|d| d.code == diagnostics::Code::E305));
}

const BINARY_SRC: &str = "fn Main(): Int { let arr = [10, 20, 12]; let sum = 0; for x in arr { sum = sum + x; } print(\"Binary execution success:\", sum); return sum; }";

fn build_and_run(release: bool, tag: &str) -> (i32, String) {
    let dir = std::env::temp_dir().join(format!("rnx-build-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("test_binary.rnx");
    std::fs::write(&src_path, BINARY_SRC).unwrap();
    let out_path = dir.join("test_binary");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.arg("build").arg(&src_path).arg("-o").arg(&out_path);
    if release {
        cmd.arg("--release");
    }
    let build = cmd.output().unwrap();
    assert!(build.status.success(), "build failed: {}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    let code = run.status.code().unwrap();
    let stdout = String::from_utf8(run.stdout).unwrap();
    let _ = std::fs::remove_dir_all(&dir);
    (code, stdout)
}

#[test]
fn build_native_binary_exit_and_stdout() {
    let (code, stdout) = build_and_run(false, "dev");
    assert_eq!(code, 42);
    assert_eq!(stdout, "Binary execution success: 42\n");
}

#[test]
fn build_native_binary_release() {
    let (code, stdout) = build_and_run(true, "release");
    assert_eq!(code, 42);
    assert_eq!(stdout, "Binary execution success: 42\n");
}

fn write_mod_tree(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-mods-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(
        dir.join("src").join("math.rnx"),
        "public fn add(a: Int, b: Int): Int {\n    return a + b;\n}\nprivate fn internalHelper(): Int { return 10; }\n",
    )
    .unwrap();
    std::fs::write(
        dir.join("src").join("main.rnx"),
        "import { add } from \"./math\";\n\nfn Main(): Int {\n    let val = add(20, 22);\n    print(\"Module import success:\", val);\n    return val;\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn build_multi_file_module_binary() {
    let dir = write_mod_tree("ok");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let main = dir.join("src").join("main.rnx");
    let out = dir.join("multi_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&main)
        .arg("-o")
        .arg(&out)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Module import success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn check_multi_file_module_run() {
    let dir = write_mod_tree("run");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let main = dir.join("src").join("main.rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(&main).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert!(String::from_utf8(run.stdout).unwrap().contains("Module import success: 42"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn import_private_item_fails_e203() {    let dir = write_mod_tree("priv");
    std::fs::write(
        dir.join("src").join("bad.rnx"),
        "import { internalHelper } from \"./math\";\nfn Main(): Int { return internalHelper(); }\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx)
        .arg("check")
        .arg(dir.join("src").join("bad.rnx"))
        .output()
        .unwrap();
    assert!(!out.status.success());
    let text =
        String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("E203"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn own_class_private_field_across_files_runs() {
    let dir = std::env::temp_dir().join(format!("rnx-privown-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("vault.rnx"),
        "class Vault {\n    private let n: Int = 0;\n    init() {}\n    fn add(v: Int) {\n        this.n = this.n + v;\n    }\n    fn total(): Int { return this.n; }\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("main.rnx"),
        "import { Vault } from \"./vault\";\n\nlet v = new Vault();\nv.add(32);\nprint(v.total());\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg(src.join("main.rnx"))
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "32\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_mutual_imports_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-mods-mutual-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("scene.rnx"),
        "import { Entity } from \"./entity\";\n\nclass Scene {\n    let name: String;\n    init(name: String) { this.name = name; }\n    spawn(): Entity {\n        return new Entity(this, 42);\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("entity.rnx"),
        "import { Scene } from \"./scene\";\n\nclass Entity {\n    let parent: Scene;\n    let id: Int;\n    init(parent: Scene, id: Int) {\n        this.parent = parent;\n        this.id = id;\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("main.rnx"),
        "import { Scene } from \"./scene\";\n\nfn Main(): Int {\n    let world = new Scene(\"MainScene\");\n    let ent = world.spawn();\n    print(\"Mutual import success:\", ent.parent.name, ent.id);\n    return ent.id;\n}\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("mutual_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(src.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Mutual import success: MainScene 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

fn write_nway_tree(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-mods-nway-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("node_a.rnx"),
        "import { NodeB } from \"./node_b\";\n\nclass NodeA {\n    let val: Int;\n    init(v: Int) { this.val = v; }\n    createNext(): NodeB {\n        return new NodeB(this.val + 10);\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("node_b.rnx"),
        "import { NodeC } from \"./node_c\";\n\nclass NodeB {\n    let val: Int;\n    init(v: Int) { this.val = v; }\n    createNext(): NodeC {\n        return new NodeC(this.val + 20);\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("node_c.rnx"),
        "import { NodeA } from \"./node_a\";\n\nclass NodeC {\n    let val: Int;\n    init(v: Int) { this.val = v; }\n    createNext(): NodeA {\n        return new NodeA(this.val + 12);\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("main.rnx"),
        "import { NodeA } from \"./node_a\";\n\nfn Main(): Int {\n    let a = new NodeA(0);\n    let b = a.createNext();\n    let c = b.createNext();\n    let a2 = c.createNext();\n    print(\"N-way cyclic import success:\", a2.val);\n    return a2.val;\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn build_nway_cyclic_imports_binary() {
    let dir = write_nway_tree("ok");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("cyclic_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("src").join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "N-way cyclic import success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_nway_cyclic_matches_build() {
    let dir = write_nway_tree("run");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg(dir.join("src").join("main.rnx"))
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert!(String::from_utf8(run.stdout).unwrap().contains("N-way cyclic import success: 42"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_object_bytes_are_deterministic() {
    let dir = write_nway_tree("det");
    let main = dir.join("src").join("main.rnx");
    let a = cli::build_files(main.to_str().unwrap(), "Main", false).unwrap();
    let b = cli::build_files(main.to_str().unwrap(), "Main", false).unwrap();
    assert_eq!(a, b);
    assert!(a.len() > 1000);
    let _ = std::fs::remove_dir_all(&dir);
}

fn write_vecbox_tree(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-mods-vb-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    let src = dir.join("src");
    std::fs::create_dir_all(&src).unwrap();
    std::fs::write(
        src.join("geo.rnx"),
        "import { wrap } from \"./box\";\n\nrecord Vec2(x: Float, y: Float)\n\nfn origin(): Vec2 {\n    return Vec2(0.0, 0.0);\n}\n\nfn originScale(): Int {\n    return wrap(origin());\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("box.rnx"),
        "import { Vec2 } from \"./geo\";\n\nenum Shape { Empty, Circle(Vec2) }\n\nfn wrap(v: Vec2): Int {\n    let s = Shape.Circle(v);\n    switch s {\n        case .Empty: return 0;\n        case .Circle(p): return 42;\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        src.join("main.rnx"),
        "import { originScale } from \"./geo\";\n\nfn Main(): Int {\n    let val = originScale();\n    print(\"record enum cross-cycle success:\", val);\n    return 0;\n}\n",
    )
    .unwrap();
    dir
}

#[test]
fn run_cross_cycle_record_enum() {
    let dir = write_vecbox_tree("run");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg(dir.join("src").join("main.rnx"))
        .output()
        .unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8(run.stdout).unwrap().contains("record enum cross-cycle success: 42"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_enum_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-enum-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.rnx"),
        "record Vec2(x: Float, y: Float)\n\nenum Shape {\n    None,\n    Circle(Vec2),\n    Box(Vec2)\n}\n\nfn evaluate(s: Shape): Int {\n    switch s {\n        case .None: return 0;\n        case .Circle(v): return Int(v.x + v.y);\n        case .Box(v): return Int(v.x * v.y);\n    }\n}\n\nfn Main(): Int {\n    let s = Shape.Circle(Vec2(20.0, 22.0));\n    let res = evaluate(s);\n    print(\"Enum native binary success:\", res);\n    return res;\n}\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("enumtest");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Enum native binary success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_thread_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-thread-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.rnx"),
        "fn worker(): Int {\n    let sum = 0;\n    let i = 0;\n    while i < 1000 {\n        sum = sum + 1;\n        i = i + 1;\n    }\n    return sum;\n}\n\nfn Main(): Int {\n    let handle = Thread.spawn(worker);\n    let res: Int = handle.join().unwrap();\n    if res == 1000 {\n        print(\"Thread execution success:\", res);\n        return 42;\n    }\n    return 0;\n}\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("threadtest");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Thread execution success: 1000\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_async_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-async-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.rnx"),
        "async fn subtask(x: Int): Int {\n    return x + 10;\n}\n\nasync fn compute(): Int {\n    let a = 12;\n    let b = await subtask(20);\n    return a + b;\n}\n\nfn Main(): Int {\n    let result: Int = compute().wait().unwrap();\n    print(\"Async promise success:\", result);\n    return result;\n}\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("asynctest");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Async promise success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn build_async_main_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-asyncmain-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("main.rnx"),
        "async fn worker(depth: Int): Int {\n    if depth <= 0 {\n        return 1;\n    }\n    let next = await worker(depth - 1);\n    return next + 10;\n}\n\nasync fn Main(): Int {\n    let res = await worker(4);\n    let finalVal = res + 1;\n    print(\"Top-level async main success:\", finalVal);\n    return finalVal;\n}\n",
    )
    .unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("asynctest");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "Top-level async main success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

const STD_TIME_SRC: &str = "import { Clock } from \"@std/time\";\n\nfn Main(): Int {\n    let vclock = Clock.virtual(1000000);\n    vclock.tick(500000);\n    let d1 = vclock.now();\n    let ms = d1.toMillis();\n    let t0 = Clock.mono();\n    let t1 = Clock.mono();\n    if d1.nanos == 1500000 && ms == 1 && t1.nanos >= t0.nanos {\n        print(\"@std/time verification success:\", d1.nanos);\n        return 0;\n    }\n    return 0;\n}\n";

#[test]
fn build_std_time_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-stdtime-{tag}", tag = "bin"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_TIME_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("time_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "@std/time verification success: 1500000\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_std_time_interpreter() {
    let dir = std::env::temp_dir().join(format!("rnx-stdtime-{tag}", tag = "run"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_TIME_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(dir.join("main.rnx")).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    assert!(String::from_utf8(run.stdout).unwrap().contains("@std/time verification success: 1500000"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn unknown_std_module_is_e108() {
    let dir = std::env::temp_dir().join(format!("rnx-stdtime-{tag}", tag = "bad"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), "import { X } from \"@std/bogus\";\nfn Main(): Int { return 0; }\n").unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out = std::process::Command::new(rnx).arg("check").arg(dir.join("main.rnx")).output().unwrap();
    assert!(!out.status.success());
    let text = String::from_utf8(out.stdout).unwrap() + &String::from_utf8(out.stderr).unwrap();
    assert!(text.contains("unknown standard library module"), "{text}");
    let _ = std::fs::remove_dir_all(&dir);
}

const STD_RANDOM_SRC: &str = "import { Rng, CryptoRng } from \"@std/random\";\n\nfn Main(): Int {\n    let rng1 = Rng.seeded(1337);\n    let rng2 = Rng.seeded(1337);\n    let a1 = rng1.next();\n    let a2 = rng1.next();\n    let b1 = rng2.next();\n    let b2 = rng2.next();\n    let bounded = rng1.int(10, 20);\n    let inRange = bounded >= 10 && bounded <= 20;\n    let c = CryptoRng.int();\n    if a1 == b1 && a2 == b2 && inRange {\n        print(\"@std/random verification success:\", bounded);\n        return 0;\n    }\n    return 0;\n}\n";

#[test]
fn build_std_random_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-stdrandom-{tag}", tag = "bin"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_RANDOM_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("random_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.starts_with("@std/random verification success:"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_std_random_interpreter() {
    let dir = std::env::temp_dir().join(format!("rnx-stdrandom-{tag}", tag = "run"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_RANDOM_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(dir.join("main.rnx")).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.starts_with("@std/random verification success:"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

fn std_fs_src(path: &str) -> String {
    format!("import fs, {{ File, OpenMode, Path }} from \"@std/fs\";\n\nfn Main(): Int {{\n    let testPath = \"{path}\";\n    if Path.exists(testPath) {{\n        fs.remove(testPath);\n    }}\n    let fileOut = File.open(testPath, OpenMode.Write);\n    fileOut.writeText(\"Rasmalai IO 42\");\n    fileOut.close();\n    let existsAfterWrite = Path.exists(testPath);\n    let fileIn = File.open(testPath, OpenMode.Read);\n    let content = fileIn.readText().unwrapOr(\"\");\n    fileIn.close();\n    let removed = fs.remove(testPath).unwrapOr(false);\n    let existsAfterRemove = Path.exists(testPath);\n    if existsAfterWrite && content == \"Rasmalai IO 42\" && removed && !existsAfterRemove {{\n        print(\"@std/fs verification success:\", content);\n        return 0;\n    }}\n    return 0;\n}}\n")
}

#[test]
fn build_std_fs_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-stdfs-{tag}", tag = "bin"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("tmp_io_test.txt");
    std::fs::write(dir.join("main.rnx"), std_fs_src(&target.to_string_lossy())).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("io_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        "@std/fs verification success: Rasmalai IO 42\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_std_fs_interpreter() {
    let dir = std::env::temp_dir().join(format!("rnx-stdfs-{tag}", tag = "run"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let target = dir.join("tmp_io_test.txt");
    std::fs::write(dir.join("main.rnx"), std_fs_src(&target.to_string_lossy())).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(dir.join("main.rnx")).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("@std/fs verification success: Rasmalai IO 42"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

const STD_ENV_SRC: &str = "import { Env } from \"@std/env\";\n\nfn Main(): Int {\n    Env.set(\"RNX_TEST_KEY\", \"42_VALUE\");\n    let val = Env.get(\"RNX_TEST_KEY\");\n    let cwd = Env.cwd();\n    let args = Env.args();\n    if val == \"42_VALUE\" && cwd != \"\" && args.length >= 1 {\n        print(\"@std/env verification success:\", val);\n        return 0;\n    }\n    return 0;\n}\n";

#[test]
fn build_std_env_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-stdenv-{tag}", tag = "bin"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_ENV_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("env_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).arg("extra").output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        "@std/env verification success: 42_VALUE\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_std_env_interpreter() {
    let dir = std::env::temp_dir().join(format!("rnx-stdenv-{tag}", tag = "run"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_ENV_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(dir.join("main.rnx")).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("@std/env verification success: 42_VALUE"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

const STD_MATH_SRC: &str = "import { Math, Vec2 } from \"@std/math\";\n\nfn Main(): Int {\n    let root = Math.sqrt(16.0);\n    let clamped = Math.clamp(10.0, 0.0, 5.0);\n    let p = Math.pow(2.0, 3.0);\n    let v1 = new Vec2(3.0, 4.0);\n    let len = v1.length();\n    let dotProd = v1.dot(new Vec2(2.0, 3.0));\n    let sum = Int(root + clamped + p + len + dotProd);\n    let finalVal = sum + 2;\n    if finalVal == 42 {\n        print(\"@std/math verification success:\", finalVal);\n        return 0;\n    }\n    return 0;\n}\n";

#[test]
fn build_std_math_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-stdmath-{tag}", tag = "bin"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_MATH_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("math_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        "@std/math verification success: 42\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_std_math_interpreter() {
    let dir = std::env::temp_dir().join(format!("rnx-stdmath-{tag}", tag = "run"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_MATH_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(dir.join("main.rnx")).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("@std/math verification success: 42"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

const STD_COLLECTIONS_SRC: &str = "import { Map, Set } from \"@std/collections\";\n\nfn Main(): Int {\n    let m = new Map();\n    m.set(\"alpha\", 10);\n    m.set(\"beta\", 20);\n    m.set(\"gamma\", 30);\n    let hasBeta = m.has(\"beta\");\n    let valGamma = m.get(\"gamma\") ?? -1;\n    m.delete(\"beta\");\n    let hasBetaAfter = m.has(\"beta\");\n    let mapLen = m.len();\n    let keys = m.keys();\n    let orderOk = keys.length == 2 && keys[0] == \"alpha\" && keys[1] == \"gamma\";\n    let s = new Set();\n    s.add(\"entity_1\");\n    s.add(\"entity_2\");\n    let setOk = s.has(\"entity_1\") && !s.has(\"entity_3\") && s.len() == 2;\n    if hasBeta && !hasBetaAfter && valGamma == 30 && mapLen == 2 && orderOk && setOk {\n        print(\"@std/collections verification success:\", valGamma + 12);\n        return 0;\n    }\n    return 0;\n}\n";

#[test]
fn build_std_collections_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-stdcol-{tag}", tag = "bin"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_COLLECTIONS_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("col_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        "@std/collections verification success: 42\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_std_collections_interpreter() {
    let dir = std::env::temp_dir().join(format!("rnx-stdcol-{tag}", tag = "run"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_COLLECTIONS_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(dir.join("main.rnx")).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("@std/collections verification success: 42"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}

const STD_SYNC_SRC: &str = "import { AtomicInt, Channel } from \"@std/sync\";\n\nfn worker(): Int {\n    let counter = AtomicInt.byId(1);\n    let chan = Channel.byId(1);\n    counter.fetchAdd(10);\n    chan.send(32);\n    return 0;\n}\n\nfn Main(): Int {\n    let counter = AtomicInt.byId(1);\n    counter.set(0);\n    let chan = Channel.byId(1);\n    let emptyCheck = chan.tryRecv();\n    let handle = Thread.spawn(worker);\n    let received = chan.recv();\n    handle.join();\n    let countVal = counter.get();\n    let total = received + countVal;\n    if emptyCheck == -1 && total == 42 {\n        print(\"@std/sync verification success:\", total);\n        return 0;\n    }\n    return 0;\n}\n";

#[test]
fn build_std_sync_native_binary() {
    let dir = std::env::temp_dir().join(format!("rnx-stdsync-{tag}", tag = "bin"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_SYNC_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let out_path = dir.join("sync_test");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&out_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&out_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        "@std/sync verification success: 42\n"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn run_std_sync_interpreter() {
    let dir = std::env::temp_dir().join(format!("rnx-stdsync-{tag}", tag = "run"));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), STD_SYNC_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx).arg("run").arg(dir.join("main.rnx")).output().unwrap();
    assert!(run.status.success(), "{}", String::from_utf8_lossy(&run.stderr));
    let stdout = String::from_utf8(run.stdout).unwrap();
    assert!(stdout.contains("@std/sync verification success: 42"), "{stdout}");
    let _ = std::fs::remove_dir_all(&dir);
}
