use cranelift::jit::Jit;


static HARNESS_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn module_of(src: &str, tag: &str) -> lir::instr::Module {
    let n = HARNESS_CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rnx-harness-{tag}-{}-{n}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn jit_of(src: &str) -> lir::instr::Module {
    module_of(src, "cl")
}

const FIB: &str = "fn Fib(n: Int): Int { let a = 0 let b = 1 let i = 0 while i < n { let t = a + b a = b b = t i += 1 } return a }";

#[test]
fn jit_fib_matches_interpreter() {
    let lir = jit_of(FIB);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Fib", &[10]).unwrap(), 55);
    assert_eq!(jit.call("Fib", &[0]).unwrap(), 0);
    assert_eq!(jit.call("Fib", &[1]).unwrap(), 1);
    assert_eq!(jit.call("Fib", &[20]).unwrap(), 6765);
}

#[test]
fn jit_arith_branches_calls() {
    let lir = jit_of(
        "fn Double(x: Int): Int { return x * 2 }
         fn Pick(a: Int, b: Int): Int { return (a > b ? a : b) + Double(1) }
         fn Main(): Int { return Pick(3, 9) + Double(4) }",
    );
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 9 + 2 + 8);
    assert_eq!(jit.call("Double", &[21]).unwrap(), 42);
}

#[test]
fn jit_accepts_heap_capture() {
    let lir = jit_of("fn Main(): Int { let a = [1]; let f = () => a; return 0; }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 0);
}

#[test]
fn jit_closure_pure_and_capturing() {
    let lir = jit_of("fn Main(): Int { let f = (): Int => 42; let factor = 10; let mult = (x: Int): Int => x * factor; return f() + mult(5); }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 92);
}

#[test]
fn jit_array_len_call_form() {
    let lir = jit_of("fn Main(): Int { let xs = [1, 2]; return xs.len(); }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 2);
}

#[test]
fn jit_print_i64_via_runtime_symbol() {
    let lir = jit_of("fn Main(): Int { print(42) return 7 }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 7);
}

#[test]
fn jit_class_point() {
    let lir = jit_of(
        "class Point { let x: Int; let y: Int; init(x: Int, y: Int) { this.x = x; this.y = y; } } fn Main(): Int { let pt = new Point(10, 32); return pt.x + pt.y; }",
    );
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

const POINT_PROG: &str = "class Point { let x: Int; let y: Int; init(x: Int, y: Int) { this.x = x; this.y = y; } } fn Main(): Int { let pt = new Point(10, 32); print(pt.x + pt.y); return pt.x + pt.y; }";

#[test]
fn jit_point_print_and_return() {
    let lir = jit_of(POINT_PROG);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_overwrite_releases_old() {
    let lir = jit_of(
        "class Point { let x: Int; let y: Int; init(x: Int, y: Int) { this.x = x; this.y = y; } } fn Main(): Int { let a = new Point(1, 2); let b = new Point(3, 4); b = a; return b.x + b.y; }",
    );
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 3);
}

#[test]
fn jit_recursive_release() {
    let lir = jit_of("class B { let v: Int; init(v: Int) { this.v = v; } } class A { let b: B; init(b: B) { this.b = b; } fn sum(): Int { return this.b.v + 1; } } fn Main(): Int { let a = new A(new B(41)); return a.sum(); }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

const GENREF_PROG: &str = "class Node { let value: Int; init(v: Int) { this.value = v; } } fn testGenRef(): Int { let weakRef: GenRef<Node>? = null; do { let temp = new Node(99); weakRef = GenRef(temp); guard let active = weakRef.get() else { return 0; } } while false guard let stale = weakRef.get() else { return 42; } return stale.value; }";

#[test]
fn jit_genref_expires() {
    let lir = jit_of(GENREF_PROG);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("testGenRef", &[]).unwrap(), 42);
}

#[test]
fn jit_strings_concat_eq_print() {
    let lir = jit_of(r#"fn Main(): Int { let greeting = "Hello"; let target = "World"; let msg = greeting + ", " + target + "!"; print(msg, 42); if msg == "Hello, World!" { return 42; } return 0; }"#);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_string_interp_int() {
    let lir = jit_of(r#"fn Main(): Int { let s = "n={40 + 2}!"; print(s); if s == "n=42!" { return 42; } return 0; }"#);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_string_neq() {
    let lir = jit_of(r#"fn Main(): Int { if "a" != "b" { return 7; } return 0; }"#);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 7);
}

#[test]
fn jit_string_inner_scope() {
    let lir = jit_of(r#"fn Main(): Int { let t = 0; if t == 0 { let s = "a" + "b"; print(s); } return 7; }"#);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 7);
}

#[test]
fn jit_string_field() {
    let lir = jit_of(r#"class C { let s: String; init(s: String) { this.s = s; } fn get(): String { return this.s; } } fn Main(): Int { let c = new C("xy"); print(c.get()); if c.get() == "xy" { return 9; } return 0; }"#);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 9);
}

#[test]
fn jit_bool_interp() {
    let lir = jit_of(r#"fn Main(): Int { let s = "v={1 == 1}!"; print(s); if s == "v=true!" { return 5; } return 0; }"#);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 5);
}

#[test]
fn jit_arrays_basic_flow() {
    let lir = jit_of("fn Main(): Int { let list = [10, 20, 30]; list.push(40); let sum = 0; for x in list { sum = sum + x; } if sum == 100 && list.length == 4 { return 42; } return 0; }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_nested_arrays() {
    let lir = jit_of("fn Main(): Int { let outer = [[1, 2], [3, 4]]; let t = 0; for row in outer { for v in row { t = t + v; } } if t == 10 { return 42; } return 0; }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_array_set_get() {
    let lir = jit_of("fn Main(): Int { let a = [1, 2, 3]; a[1] = 20; return a[0] + a[1] + a[2]; }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 24);
}

const FLOAT_TASK: &str = "fn Main(): Int { let a: Float = 10.5; let b: Float = 20.25; let c = a * 2.0 + b; let fast_c = c.asFast(); let scaled = fast_c * 2.0.asFast(); let result = Int(scaled.asStrict()) / 2; if result == 41 { return 42; } return 0; }";

#[test]
fn jit_float_task_program() {
    let lir = jit_of(FLOAT_TASK);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_float_bits_roundtrip() {
    let lir = jit_of("fn Id(x: Float): Float { return x }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    let bits = 41.25f64.to_bits() as i64;
    assert_eq!(jit.call("Id", &[bits]).unwrap(), bits);
}

#[test]
fn jit_float_cmp_and_neg() {
    let lir = jit_of("fn Cmp(a: Float, b: Float): Int { if a < b { return 1; } if a == b { return 2; } return 3; } fn Neg(x: Float): Float { return 0.0 - x }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    let (a, b) = (1.5f64.to_bits() as i64, 2.5f64.to_bits() as i64);
    assert_eq!(jit.call("Cmp", &[a, b]).unwrap(), 1);
    assert_eq!(jit.call("Cmp", &[b, b]).unwrap(), 2);
    assert_eq!(jit.call("Cmp", &[b, a]).unwrap(), 3);
    assert_eq!(f64::from_bits(jit.call("Neg", &[a]).unwrap() as u64), -1.5);
}

#[test]
fn jit_int_switch_dispatch() {
    let lir = jit_of("fn F(x: Int): Int { switch x { case 1: return 10; case 2: return 20; default: return 0; } }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("F", &[1]).unwrap(), 10);
    assert_eq!(jit.call("F", &[2]).unwrap(), 20);
    assert_eq!(jit.call("F", &[9]).unwrap(), 0);
}

#[test]
fn jit_enum_construct_switch_payload() {
    let lir = jit_of("enum Shape { None, Circle(Int) } fn Eval(s: Shape): Int { switch s { case .None: return 0; case .Circle(v): return v * 2; } } fn Main(): Int { return Eval(Circle(21)); }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_enum_str_payload_drops() {
    let lir = jit_of("enum Msg { Blank, Text(String) } fn Len(m: Msg): Int { switch m { case .Blank: return 0; case .Text(t): return 7; } } fn Main(): Int { let i = 0 let t = 0 while i < 100 { let m = Text(\"hi\"); t = t + Len(m); i = i + 1; } return t; }");
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 700);
}

#[test]
fn jit_thread_spawn_join() {
    let dir = std::env::temp_dir().join(format!("rnx-jit-thr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), "fn worker(): Int { let sum = 0 let i = 0 while i < 1000 { sum = sum + 1 i = i + 1 } return sum } fn Main(): Int { let handle = Thread.spawn(worker) let res = handle.join().unwrap() if res == 1000 { return 42 } return 0 }").unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let lir = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_async_await_promise() {
    let src = "async fn sub(x: Int): Int { return x + 1; } async fn compute(): Int { let b = await sub(20); return b + 21; } fn Main(): Int { let r: Int = compute().wait().unwrap(); return r; }";
    let lir = jit_of(src);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_async_main_driver() {
    let src = "async fn Main(): Int { let x = 40; return x + 2; }";
    let lir = jit_of(src);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_async_recursion_with_await() {
    let src = "async fn worker(d: Int): Int { if d <= 0 { return 1; } let n = await worker(d - 1); return n + 10; } async fn Main(): Int { let r = await worker(3); return r + 11; }";
    let lir = jit_of(src);
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_std_time_virtual_and_mono() {
    let dir = std::env::temp_dir().join("rnx-jit-stdtime");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Clock } from \"@std/time\";\nfn Main(): Int { let v = Clock.virtual(1000000); v.tick(500000); let d = v.now(); let t0 = Clock.mono(); let t1 = Clock.mono(); if d.nanos == 1500000 && d.toMillis() == 1 && t1.nanos >= t0.nanos { return 42; } return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_std_random_determinism() {
    let dir = std::env::temp_dir().join("rnx-jit-stdrandom");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Rng, CryptoRng } from \"@std/random\";\nfn Main(): Int { let r1 = Rng.seeded(1337); let r2 = Rng.seeded(1337); if r1.next() != r2.next() { return 0; } if r1.next() != r2.next() { return 0; } let b = r1.int(10, 20); if b < 10 || b > 20 { return 0; } let c = CryptoRng.int(); let d = c == c; if d { return 42; } return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_std_fs_roundtrip() {
    let dir = std::env::temp_dir().join("rnx-jit-stdfs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    let target = dir.join("t.txt");
    let src = format!("import fs, {{ File, OpenMode, Path }} from \"@std/fs\";\nfn Main(): Int {{ let p = \"{}\"; let w = File.open(p, OpenMode.Write); w.writeText(\"abc\"); w.close(); if !Path.exists(p) {{ return 0; }} let r = File.open(p, OpenMode.Read); let c = r.readText().unwrapOr(\"\"); r.close(); let j = Path.join(\"a\", \"b\"); if c == \"abc\" && j == \"a/b\" && fs.remove(p).unwrapOr(false) && !Path.exists(p) {{ return 42; }} return 0; }}\n", target.to_string_lossy());
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_std_env_roundtrip() {
    let dir = std::env::temp_dir().join("rnx-jit-stdenv");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Env } from \"@std/env\";\nfn Main(): Int { Env.set(\"RNX_JIT_ENV_KEY\", \"v42\"); let v = Env.get(\"RNX_JIT_ENV_KEY\"); let c = Env.cwd(); let n = Env.args().length; if v == \"v42\" && c != \"\" && n >= 1 { return 42; } return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_std_math_scalar_and_vec() {
    let dir = std::env::temp_dir().join("rnx-jit-stdmath");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Math, Vec2 } from \"@std/math\";\nfn Main(): Int { if Math.sqrt(16.0) != 4.0 { return 0; } if Math.floor(2.7) != 2.0 { return 0; } if Math.sin(0.0) != 0.0 { return 0; } let v = new Vec2(3.0, 4.0); if v.length() != 5.0 { return 0; } if Int(Math.PI() * 1000000.0) != 3141592 { return 0; } return 42; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_std_collections_map_set() {
    let dir = std::env::temp_dir().join("rnx-jit-stdcol");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Map, Set } from \"@std/collections\";\nfn Main(): Int { let m = new Map(); m.set(\"a\", 1); m.set(\"b\", 2); if !m.has(\"a\") { return 0; } if (m.get(\"b\") ?? -1) != 2 { return 0; } m.delete(\"a\"); if m.len() != 1 { return 0; } let k = m.keys(); if k.length != 1 || k[0] != \"b\" { return 0; } let v = m.values(); let vv: Int = v[0]; if vv != 2 { return 0; } m.clear(); if m.len() != 0 { return 0; } let s = new Set(); s.add(\"x\"); if !s.has(\"x\") || s.len() != 1 { return 0; } if s.delete(\"x\") != true { return 0; } s.clear(); return 42; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_std_sync_atomic_channel() {
    let dir = std::env::temp_dir().join("rnx-jit-stdsync");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { AtomicInt, Channel } from \"@std/sync\";\nfn worker(): Int { let c = AtomicInt.byId(41); c.fetchAdd(10); let ch = Channel.byId(41); ch.send(32); return 0; }\nfn Main(): Int { let c = AtomicInt.byId(41); c.set(0); let ch = Channel.byId(41); if ch.tryRecv() != -1 { return 0; } if !c.cas(0, 0) { return 0; } let h = Thread.spawn(worker); let r = ch.recv(); h.join(); if ch.len() != 0 { return 0; } if r + c.get() == 42 { return 42; } return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn jit_stdlib_grand_tour() {
    std::fs::create_dir_all("target").unwrap();
    let main = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cli/tests/fixtures/grand_tour/src/main.rnx");
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap();
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert!(!std::path::Path::new("target/grand_tour_tmp.txt").exists());
    let _ = std::fs::remove_dir("target");
}

#[test]
fn jit_multi_package_deps() {
    let main = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../cli/tests/fixtures/multi_package/app/src/main.rnx");
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let lir = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
}

#[test]
fn jit_is_const_false_arm_no_host_abort() {
    let lir = jit_of(
        "fn Kind(x: Int): Int { switch x { case is String: return 1; default: return 0; } }
         fn Main(): Int { return Kind(7); }",
    );
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Kind", &[7]).unwrap(), 0);
    assert_eq!(jit.call("Main", &[]).unwrap(), 0);
}

#[test]
fn jit_is_class_check_no_host_abort() {
    let lir = jit_of(
        "class Box { let v: Int; init(v: Int) { this.v = v; } }
         fn IsBox(v: Any): Int { switch v { case is Box: return 1; default: return 0; } }
         fn Main(): Int { let b = new Box(41); return IsBox(b) + IsBox(7); }",
    );
    let mut jit = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 1);
}
