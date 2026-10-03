use cranelift::jit::{FunctionSlotId, Jit};

static HARNESS_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

fn module_of(src: &str, tag: &str) -> lir::instr::Module {
    let n = HARNESS_CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let dir = std::env::temp_dir().join(format!("rnx-hotreload-{tag}-{n}-{}", std::process::id()));
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

const V1: &str = "fn getMultiplier(): Int { return 2; }
    fn calculate(x: Int): Int { return x * getMultiplier(); }";
const V2: &str = "fn getMultiplier(): Int { return 5; }
    fn calculate(x: Int): Int { return x * getMultiplier(); }";

#[test]
fn test_function_hot_swap() {
    let lir = module_of(V1, "swap");
    let mut jit = Jit::compile_hot(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("calculate", &[10]).unwrap(), 20);
    let lir2 = module_of(V2, "swap2");
    let slot: FunctionSlotId = jit.slot_of("getMultiplier").expect("slot");
    jit.hot_swap_function(slot, &lir2).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("calculate", &[10]).unwrap(), 50);
    assert_eq!(jit.call("getMultiplier", &[]).unwrap(), 5);
}

#[test]
fn test_hot_swap_across_loop_and_recursion() {
    let lir = module_of(V1, "loop");
    let mut jit = Jit::compile_hot(&lir).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..100 {
        assert_eq!(jit.call("calculate", &[i]).unwrap(), i * 2);
    }
    let lir2 = module_of(V2, "loop2");
    let slot: FunctionSlotId = jit.slot_of("getMultiplier").expect("slot");
    jit.hot_swap_function(slot, &lir2).unwrap_or_else(|e| panic!("{e}"));
    for i in 0..100 {
        assert_eq!(jit.call("calculate", &[i]).unwrap(), i * 5);
    }
    let rec = module_of(
        "fn step(n: Int): Int { return n * getMultiplier(); }
         fn down(n: Int): Int { return n <= 0 ? 0 : step(n) + down(n - 1); }
         fn getMultiplier(): Int { return 2; }",
        "rec",
    );
    let mut jit2 = Jit::compile_hot(&rec).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit2.call("down", &[4]).unwrap(), 2 * (4 + 3 + 2 + 1));
    let rec2 = module_of(
        "fn step(n: Int): Int { return n * getMultiplier(); }
         fn down(n: Int): Int { return n <= 0 ? 0 : step(n) + down(n - 1); }
         fn getMultiplier(): Int { return 3; }",
        "rec2",
    );
    let slot2: FunctionSlotId = jit2.slot_of("getMultiplier").expect("slot");
    jit2.hot_swap_function(slot2, &rec2).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit2.call("down", &[4]).unwrap(), 3 * (4 + 3 + 2 + 1));
}

#[test]
fn test_hot_swap_refusals() {
    let lir = module_of(V1, "ref");
    let mut cold = Jit::compile(&lir).unwrap_or_else(|e| panic!("{e}"));
    let lir2 = module_of(V2, "ref2");
    assert!(cold.hot_swap_function(0, &lir2).is_err());
    let mut jit = Jit::compile_hot(&lir).unwrap_or_else(|e| panic!("{e}"));
    assert!(jit.hot_swap_function(9999, &lir2).is_err());
    let bad_sig = module_of(
        "fn getMultiplier(x: Int): Int { return x; }
         fn calculate(x: Int): Int { return x * getMultiplier(x); }",
        "ref3",
    );
    let slot: FunctionSlotId = jit.slot_of("getMultiplier").expect("slot");
    assert!(jit.hot_swap_function(slot, &bad_sig).is_err());
    assert_eq!(jit.call("calculate", &[10]).unwrap(), 20);
}
