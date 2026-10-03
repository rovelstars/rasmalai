use runtime::machine::{ExecError, Machine};

use runtime::value::Value;


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

fn run(src: &str, func: &str, args: Vec<Value>) -> (Result<Value, ExecError>, Vec<String>) {
    run_with(src, func, args, &mut |_| Vec::new())
}

fn run_with(
    src: &str,
    func: &str,
    args: Vec<Value>,
    build: &mut dyn FnMut(&mut Machine) -> Vec<Value>,
) -> (Result<Value, ExecError>, Vec<String>) {
    let out = module_of(src, "rt");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    let extra = build(&mut machine);
    let mut full_args = extra;
    full_args.extend(args);
    let r = machine.call(func, full_args);
    (r, machine.output.clone())
}

fn ok(src: &str, func: &str) -> Value {
    match run(src, func, vec![]) {
        (Ok(v), _) => v,
        (Err(e), _) => panic!("{func}: {e:?}"),
    }
}

#[test]
fn arith() {
    let v = ok("fn F(): Int { return 2 + 3 * 4 - 10 / 2 }", "F");
    assert_eq!(v, Value::Int(9));
}

#[test]
fn fib_called_with_arg() {
    let src = "fn Fib(n: Int): Int { let a = 0 let b = 1 let i = 0 while i < n { let t = a + b a = b b = t i += 1 } return a }";
    let (r, _) = run(src, "Fib", vec![Value::Int(10)]);
    assert_eq!(r, Ok(Value::Int(55)));
}

#[test]
fn for_range_stride() {
    let src = "fn F(): Int { let t = 0 for i in 0..10 { t += i } for j in (0..10).stride(3) { t += j } return t }";
    assert_eq!(ok(src, "F"), Value::Int(45 + 0 + 3 + 6 + 9));
}

#[test]
fn do_while_runs_once() {
    let src = "fn F(): Int { let t = 0 do { t += 5 } while t > 100 return t }";
    assert_eq!(ok(src, "F"), Value::Int(5));
}

#[test]
fn break_continue_nested() {
    let src = "fn F(): Int { let t = 0 for i in 0..10 { if i % 2 == 0 { continue } if i > 7 { break } t += i } return t }";
    assert_eq!(ok(src, "F"), Value::Int(1 + 3 + 5 + 7));
}

#[test]
fn switch_dispatch() {
    let src = r#"fn F(x: Int): Int {
  switch x {
    case 0: return 10
    case 1..=3: return 20
    case 4..10 if x % 2 == 0: return 30
    default: return 40
  }
}"#;
    for (arg, want) in [(0, 10), (2, 20), (4, 30), (5, 40), (99, 40)] {
        let (r, _) = run(src, "F", vec![Value::Int(arg)]);
        assert_eq!(r, Ok(Value::Int(want)), "arg {arg}");
    }
}

#[test]
fn try_catch_finally_order() {
    let src = r#"fn F(fail: Int): Int {
  try {
    if fail == 1 { throw 99 }
    return 1
  } catch (err) {
    return 2
  } finally {
    print("fin")
  }
}"#;
    let (r, out) = run(src, "F", vec![Value::Int(0)]);
    assert_eq!(r, Ok(Value::Int(1)));
    assert_eq!(out, vec!["fin".to_string()]);
    let (r, out) = run(src, "F", vec![Value::Int(1)]);
    assert_eq!(r, Ok(Value::Int(2)));
    assert_eq!(out, vec!["fin".to_string()]);
}

#[test]
fn rethrow_propagates() {
    let src = r#"fn Inner(): Int throws { throw 7 }
fn Outer(): Int { try { return Inner() } catch (err) { throw } }"#;
    let (r, _) = run(src, "Outer", vec![]);
    match r {
        Err(ExecError::Throw(Value::Int(7))) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn defer_lifo() {
    let src = r#"fn F(): Int {
  defer { print("first") }
  defer { print("second") }
  return 3
}"#;
    let (r, out) = run(src, "F", vec![]);
    assert_eq!(r, Ok(Value::Int(3)));
    assert_eq!(out, vec!["second".to_string(), "first".to_string()]);
}

#[test]
fn guard_let() {
    let src = r#"fn F(x: Int): Int {
  guard let y = Maybe(x) else { return -1 }
  return y
}
fn Maybe(x: Int): Int? { if x > 0 { return x } return null }"#;
    let (r, _) = run(src, "F", vec![Value::Int(5)]);
    assert_eq!(r, Ok(Value::Int(5)));
    let (r, _) = run(src, "F", vec![Value::Int(-2)]);
    assert_eq!(r, Ok(Value::Int(-1)));
}

#[test]
fn class_struct_semantics() {
    let src = r#"
struct Vec { let x: Int let y: Int }
class Counter { let n: Int = 0 init() { } fn bump() { this.n += 1 } fn get(): Int { return this.n } }
fn F(): Int {
  let a = Vec(1, 2)
  let b = a
  b.x = 9
  let c = new Counter()
  let d = c
  d.bump()
  return a.x + b.x + c.get()
}"#;
    assert_eq!(ok(src, "F"), Value::Int(1 + 9 + 1));
}

#[test]
fn genref_alive_and_empty() {
    let src = r#"
class W { let t: Int init(t: Int) { this.t = t } }
fn F(): Int {
  let w = new W(42)
  let g = GenRef.of(w)
  let e = GenRef.empty()
  let v = g.get()
  if v != null { return v.t } else { return -1 }
}"#;
    assert_eq!(ok(src, "F"), Value::Int(42));
}

#[test]
fn closures_capture_and_decay() {
    let src = r#"
fn Adder(x: Int): fn(Int): Int { return (y: Int): Int => x + y }
class B { let v: Int = 3 fn get(): Int { return this.v } }
fn F(): Int {
  let add = Adder(10)
  let b = new B()
  let f = fn decay(this) { return 0 }
  return add(5) + b.get()
}"#;
    assert_eq!(ok(src, "F"), Value::Int(15 + 3));
}

#[test]
fn strings_arrays() {
    let src = r#"fn F(): String {
  let xs = [1, 2, 3]
  xs.push(4)
  let s = "n={xs.len()}"
  return s + "!"
}"#;
    assert_eq!(ok(src, "F"), Value::Str("n=4!".to_string()));
}

#[test]
fn ternary_and_logic() {
    let src = "fn F(a: Int, b: Int): Int { return (a > b ? a : b) + (a == 1 && b == 2 ? 100 : 0) }";
    let (r, _) = run(src, "F", vec![Value::Int(3), Value::Int(2)]);
    assert_eq!(r, Ok(Value::Int(3)));
    let (r, _) = run(src, "F", vec![Value::Int(1), Value::Int(2)]);
    assert_eq!(r, Ok(Value::Int(102)));
}

#[test]
fn recursion() {
    let src = "fn Fact(n: Int): Int { if n <= 1 { return 1 } return n * Fact(n - 1) }";
    let (r, _) = run(src, "Fact", vec![Value::Int(6)]);
    assert_eq!(r, Ok(Value::Int(720)));
}

#[test]
fn assert_traps() {
    let src = "fn F(): Int { Assert!(1 == 2, \"boom\") return 1 }";
    let (r, _) = run(src, "F", vec![]);
    assert!(matches!(r, Err(ExecError::Throw(_))), "{r:?}");
}

#[test]
fn enum_match_payload() {
    let src = r#"enum Maybe { None, Some(Int) }
fn F(o: Maybe): Int {
  switch o {
    case .None: return -1
    case .Some(v): return v * 2
  }
}"#;
    let (none, _) = run(src, "F", vec![Value::Enum { enu: 0, variant: 0, name: "None".to_string(), payload: vec![] }]);
    assert_eq!(none, Ok(Value::Int(-1)));
    let (some, _) = run(src, "F", vec![Value::Enum { enu: 0, variant: 1, name: "Some".to_string(), payload: vec![Value::Int(21)] }]);
    assert_eq!(some, Ok(Value::Int(42)));
}

#[test]
fn enum_construct_and_generic() {
    let src = r#"enum Box<T> { Empty, Full(T) }
fn F(): Int {
  let a = Full(10)
  let b = Empty
  switch a {
    case .Full(x): return x + 1
    case .Empty: return 0
  }
}"#;
    assert_eq!(ok(src, "F"), Value::Int(11));
}

#[test]
fn enum_qualified_construct() {
    let src = r#"enum Color { Red, Green }
fn F(): Int {
  let c = Color.Green
  switch c {
    case .Red: return 1
    case .Green: return 2
  }
}"#;
    assert_eq!(ok(src, "F"), Value::Int(2));
}

#[test]
fn record_copy_semantics() {
    let src = r#"record Point(x: Float, y: Float)
fn F(): Float {
  let a = Point(1.0, 2.0)
  let b = a
  b.x = 9.0
  return a.x + b.x
}"#;
    assert_eq!(ok(src, "F"), Value::Float(10.0, lir::instr::FloatKind::Strict));
}

#[test]
fn tuple_for_arrays() {
    let src = "fn F(p: Array): Int { let t = 0 for (a, b) in p { t += a + b } return t }";
    let (r, _) = run_with(src, "F", vec![], &mut |machine| {
        let inner1 = machine.new_array(vec![Value::Int(1), Value::Int(2)]);
        let inner2 = machine.new_array(vec![Value::Int(3), Value::Int(4)]);
        vec![machine.new_array(vec![inner1, inner2])]
    });
    assert_eq!(r, Ok(Value::Int(10)));
}

#[test]
fn unsafe_block_runs() {
    let src = r#"unsafe fn Raw(x: Int): Int { return x * 2 }
fn F(): Int {
  let y = 0
  unsafe { y = Raw(21) }
  return y
}"#;
    assert_eq!(ok(src, "F"), Value::Int(42));
}

#[test]
fn unsafe_pointer_ops() {
    let src = "fn F(): Int { unsafe { let x = 5 let p = &x let q = p + 2 return q - p } }";
    assert_eq!(ok(src, "F"), Value::Int(2));
}

#[test]
fn print_captures_output() {
    let (r, out) = run("fn Main(): Int { print(42) return 7 }", "Main", vec![]);
    assert_eq!(r, Ok(Value::Int(7)));
    assert_eq!(out, vec!["42".to_string()]);
}

#[test]
fn point_print_and_return() {
    let src = "class Point { let x: Int; let y: Int; init(x: Int, y: Int) { this.x = x; this.y = y; } } fn Main(): Int { let pt = new Point(10, 32); print(pt.x + pt.y); return pt.x + pt.y; }";
    let (r, out) = run(src, "Main", vec![]);
    assert_eq!(r, Ok(Value::Int(42)));
    assert_eq!(out, vec!["42".to_string()]);
}

fn live_slots(src: &str, func: &str) -> (Result<Value, ExecError>, usize) {
    let out = module_of(src, "rt");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    let r = machine.call(func, vec![]);
    let live = machine.arena.live_count();
    (r, live)
}

#[test]
fn recursive_release_no_leak() {
    let src = "class B { let v: Int; init(v: Int) { this.v = v; } } class A { let b: B; init(b: B) { this.b = b; } fn sum(): Int { return this.b.v + 1; } } fn Main(): Int { let a = new A(new B(41)); return a.sum(); }";
    let (r, live) = live_slots(src, "Main");
    assert_eq!(r, Ok(Value::Int(42)));
    assert_eq!(live, 0);
}

const GENREF_PROG: &str = "class Node { let value: Int; init(v: Int) { this.value = v; } } fn testGenRef(): Int { let weakRef: GenRef<Node>? = null; do { let temp = new Node(99); weakRef = GenRef(temp); guard let active = weakRef.get() else { return 0; } } while false guard let stale = weakRef.get() else { return 42; } return stale.value; }";

#[test]
fn genref_expires_after_scope() {
    assert_eq!(ok(GENREF_PROG, "testGenRef"), Value::Int(42));
}

#[test]
fn strings_concat_eq_print_backend() {
    let src = r#"fn Main(): Int { let greeting = "Hello"; let target = "World"; let msg = greeting + ", " + target + "!"; print(msg, 42); if msg == "Hello, World!" { return 42; } return 0; }"#;
    let (r, out) = run(src, "Main", vec![]);
    assert_eq!(r, Ok(Value::Int(42)));
    assert_eq!(out, vec!["Hello, World! 42".to_string()]);
}

const ARRAY_PROG: &str = "fn Main(): Int { let list = [10, 20, 30]; list.push(40); let sum = 0; for x in list { sum = sum + x; } if sum == 100 && list.length == 4 { return 42; } return 0; }";

#[test]
fn arrays_basic_flow() {
    assert_eq!(ok(ARRAY_PROG, "Main"), Value::Int(42));
}

#[test]
fn arrays_no_leak() {
    let mut m = frontend::parser::Parser::parse_module(ARRAY_PROG).unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]), Ok(Value::Int(42)));
    assert_eq!(machine.arrays.live_count(), 0);
}

#[test]
fn array_oob_traps_catchable() {
    let src = "fn Main(): Int { let a = [1, 2]; let x = a[5]; return x; }";
    let (r, _) = run(src, "Main", vec![]);
    assert!(matches!(r, Err(ExecError::Throw(_))), "{r:?}");
}

#[test]
fn array_set_oob_traps() {
    let src = "fn Main(): Int { let a = [1]; a[3] = 9; return 0; }";
    let (r, _) = run(src, "Main", vec![]);
    assert!(matches!(r, Err(ExecError::Throw(_))), "{r:?}");
}

#[test]
fn nested_arrays_drop_clean() {
    let src = "fn Main(): Int { let outer = [[1, 2], [3, 4]]; let t = 0; for row in outer { for v in row { t = t + v; } } if t == 10 { return 42; } return 0; }";
    let mut m = frontend::parser::Parser::parse_module(src).unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]), Ok(Value::Int(42)));
    assert_eq!(machine.arrays.live_count(), 0);
}

#[test]
fn array_set_replaces_element() {
    let src = "fn Main(): Int { let a = [1, 2, 3]; a[1] = 20; return a[0] + a[1] + a[2]; }";
    assert_eq!(ok(src, "Main"), Value::Int(24));
}

#[test]
fn e2e_float_task_program() {
    let src = "fn Main(): Int { let a: Float = 10.5; let b: Float = 20.25; let c = a * 2.0 + b; let fast_c = c.asFast(); let scaled = fast_c * 2.0.asFast(); let result = Int(scaled.asStrict()) / 2; if result == 41 { print(\"Float math verified:\", c); return 42; } return 0; }";
    let (r, out) = run(src, "Main", vec![]);
    assert_eq!(r.unwrap(), Value::Int(42));
    assert!(out.iter().any(|l| l.contains("Float math verified: 41.25")));
}

#[test]
fn e2e_float_neg_cmp() {
    let v = ok("fn Main(): Int { let x: Float = 1.5; let y = 0.0 - x; if y < 0.0 && y == 0.0 - 1.5 { return 1; } return 0; }", "Main");
    assert_eq!(v, Value::Int(1));
}

#[test]
fn enum_owned_payload_no_leak() {
    let src = "class B { let v: Int; init(v: Int) { this.v = v; } } enum Wrap { Empty, Hold(B) } fn Main(): Int { let w = Hold(new B(41)); switch w { case .Empty: return 0; case .Hold(b): return b.v + 1; } }";
    let (r, live) = live_slots(src, "Main");
    assert_eq!(r, Ok(Value::Int(42)));
    assert_eq!(live, 0);
}

#[test]
fn thread_spawn_join_e2e() {
    let dir = std::env::temp_dir().join(format!("rnx-rt-thr-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), "fn worker(): Int { let sum = 0 let i = 0 while i < 1000 { sum = sum + 1 i = i + 1 } return sum } fn Main(): Int { let handle = Thread.spawn(worker) let res = handle.join().unwrap() if res == 1000 { print(\"Thread execution success:\", res) return 42 } return 0 }").unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = runtime::machine::Machine::new(leaked);
    match machine.call("Main", vec![]) {
        Ok(v) => assert_eq!(v, Value::Int(42)),
        Err(e) => panic!("Main: {e:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn async_await_promise_e2e() {
    let v = ok("async fn sub(x: Int): Int { return x + 1; } async fn compute(): Int { let b = await sub(20); return b + 21; } fn Main(): Int { let r: Int = compute().wait().unwrap(); return r; }", "Main");
    assert_eq!(v, Value::Int(42));
}

#[test]
fn async_main_driver_e2e() {
    let v = ok("async fn Main(): Int { let x = 40; return x + 2; }", "Main");
    assert_eq!(v, Value::Int(42));
}

#[test]
fn async_recursion_with_await_e2e() {
    let v = ok("async fn worker(d: Int): Int { if d <= 0 { return 1; } let n = await worker(d - 1); return n + 10; } async fn Main(): Int { let r = await worker(3); return r + 11; }", "Main");
    assert_eq!(v, Value::Int(42));
}

#[test]
fn std_time_virtual_clock_e2e() {
    let dir = std::env::temp_dir().join("rnx-rt-stdtime");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Clock } from \"@std/time\";\nfn Main(): Int { let v = Clock.virtual(1000000); v.tick(500000); let d = v.now(); if d.nanos == 1500000 && d.toMillis() == 1 { return 42; } return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]).unwrap(), Value::Int(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_random_determinism_e2e() {
    let dir = std::env::temp_dir().join("rnx-rt-stdrandom");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Rng } from \"@std/random\";\nfn Main(): Int { let r1 = Rng.seeded(99); let r2 = Rng.seeded(99); if r1.next() != r2.next() { return 0; } let b = r1.bool(); if b == r1.bool() && b == r1.bool() { return 0; } return 42; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]).unwrap(), Value::Int(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_fs_roundtrip_e2e() {
    let dir = std::env::temp_dir().join("rnx-rt-stdfs");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    let target = dir.join("t.txt");
    let src = format!("import fs, {{ File, OpenMode }} from \"@std/fs\";\nfn Main(): Int {{ let p = \"{}\"; let w = File.open(p, OpenMode.Write); w.writeText(\"abc\"); w.flush(); w.close(); let r = File.open(p, OpenMode.Read); let c = r.readText().unwrapOr(\"\"); r.close(); if c == \"abc\" && fs.remove(p).unwrapOr(false) {{ return 42; }} return 0; }}\n", target.to_string_lossy());
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]).unwrap(), Value::Int(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_env_roundtrip_e2e() {
    let dir = std::env::temp_dir().join("rnx-rt-stdenv");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Env } from \"@std/env\";\nfn Main(): Int { Env.set(\"RNX_RT_ENV_KEY\", \"v42\"); let v = Env.get(\"RNX_RT_ENV_KEY\"); if v == \"v42\" && Env.get(\"RNX_RT_ENV_MISSING_XYZ\") == \"\" { return 42; } return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]).unwrap(), Value::Int(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_math_scalar_e2e() {
    let dir = std::env::temp_dir().join("rnx-rt-stdmath");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Math } from \"@std/math\";\nfn Main(): Int { if Math.max(3.0, 4.0) != 4.0 { return 0; } if Math.min(3.0, 4.0) != 3.0 { return 0; } if Math.abs(0.0 - 5.0) != 5.0 { return 0; } if Math.round(2.5) != 3.0 { return 0; } if Math.atan2(1.0, 1.0) <= 0.78 { return 0; } if Math.tan(0.0) != 0.0 { return 0; } return 42; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]).unwrap(), Value::Int(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_collections_map_e2e() {
    let dir = std::env::temp_dir().join("rnx-rt-stdcol");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { Map } from \"@std/collections\";\nfn Main(): Int { let m = new Map(); m.set(\"a\", 5); m.set(\"a\", 6); if (m.get(\"a\") ?? -1) != 6 { return 0; } if (m.get(\"missing\") ?? 0) != 0 { return 0; } if m.len() != 1 { return 0; } m.clear(); if m.has(\"a\") { return 0; } return 42; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]).unwrap(), Value::Int(42));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn std_sync_atomic_channel_e2e() {
    let dir = std::env::temp_dir().join("rnx-rt-stdsync");
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, "import { AtomicInt, Channel } from \"@std/sync\";\nfn worker(): Int { let c = AtomicInt.byId(43); c.fetchAdd(5); let ch = Channel.byId(43); ch.send(37); return 0; }\nfn Main(): Int { let c = AtomicInt.byId(43); c.set(0); let ch = Channel.byId(43); let h = Thread.spawn(worker); let r = ch.recv(); h.join(); if !c.cas(5, 6) { return 0; } if r + c.get() == 43 { return 42; } return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let out = lir::lower::lower(&m).unwrap();
    assert!(lir::verify::verify(&out).is_empty());
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    assert_eq!(machine.call("Main", vec![]).unwrap(), Value::Int(42));
    let _ = std::fs::remove_dir_all(&dir);
}

fn ns_tree(tag: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-ns-{tag}-{}-{}", std::process::id(), HARNESS_CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst)));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("lib.rnx"), "const K: Int = 42;\nclass Widget { let v: Int init(v: Int) { this.v = v; } }\nenum Shape { Circle(Int), Square }\nfn helper(x: Int): Int { return x * 2; }\n").unwrap();
    dir
}

fn ns_run(main_src: &str, tag: &str) -> (Result<Value, ExecError>, Vec<String>) {
    let dir = ns_tree(tag);
    std::fs::write(dir.join("main.rnx"), main_src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(out));
    let mut machine = Machine::new(leaked);
    let r = machine.call("Main", vec![]);
    let out = machine.output.clone();
    let _ = std::fs::remove_dir_all(&dir);
    (r, out)
}

#[test]
fn namespace_print_shows_exports() {
    let (r, out) = ns_run("import ns from \"./lib\";\nfn Main(): Int { print(ns); return 42; }\n", "print");
    assert_eq!(r.unwrap(), Value::Int(42));
    let line = out.join("\n");
    assert!(line.contains("[Module ns]"), "{line}");
    assert!(line.contains("helper: [Function: helper]"), "{line}");
    assert!(line.contains("Widget: [Class: Widget]"), "{line}");
    assert!(line.contains("Shape: [Enum: Shape]"), "{line}");
    assert!(line.contains("K: 42"), "{line}");
}

#[test]
fn namespace_assigned_alias_calls() {
    let (r, out) = ns_run("import ns from \"./lib\";\nfn Main(): Int { let alias = ns; print(alias); return alias.helper(21); }\n", "alias");
    assert_eq!(r.unwrap(), Value::Int(42));
    assert!(out.join("\n").contains("[Module ns]"));
}

#[test]
fn namespace_field_assign_fails() {
    let dir = ns_tree("assign");
    std::fs::write(dir.join("main.rnx"), "import ns from \"./lib\";\nfn Main(): Int { ns.helper = 5; return 0; }\n").unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap();
    let mut m = g.resolve().unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    assert!(lir::lower::lower(&m).is_err());
    let _ = std::fs::remove_dir_all(&dir);
}
