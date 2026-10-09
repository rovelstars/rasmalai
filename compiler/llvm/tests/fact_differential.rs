use inkwell::context::Context;

fn opt_module(src: &str, tag: &str, entry: &str) -> lir::instr::Module {
    let dir = std::env::temp_dir().join(format!("rnx-factdiff-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(dir.join("main.rnx"), src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    let diags = runtime::lir_pipeline::optimize_for_release(&mut out, entry);
    assert!(diags.is_empty(), "{diags:?}");
    let _ = std::fs::remove_dir_all(&dir);
    out
}

fn run_llvm(module: &lir::instr::Module, func: &str) -> Result<i64, ()> {
    let context = Context::create();
    let jit = llvm::codegen::Jit::compile(&context, module, "diff").map_err(|_| ())?;
    jit.call(func, &[]).map_err(|_| ())
}

fn run_cranelift(module: &lir::instr::Module, func: &str) -> Result<i64, ()> {
    let mut jit = cranelift::jit::Jit::compile(module).map_err(|_| ())?;
    jit.call(func, &[]).map_err(|_| ())
}

fn run_interp(module: lir::instr::Module, func: &str) -> Result<i64, ()> {
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    match machine.call(func, vec![]) {
        Ok(runtime::value::Value::Int(n)) => Ok(n),
        _ => Err(()),
    }
}

fn check_all(src: &str, tag: &str, entry: &str, expect: Result<i64, ()>) {
    let module = opt_module(src, tag, entry);
    let llvm = run_llvm(&module, entry);
    let cl = run_cranelift(&module, entry);
    let interp = run_interp(module, entry);
    assert_eq!(llvm, expect, "llvm diverged on {tag}");
    assert_eq!(cl, expect, "cranelift diverged on {tag}");
    assert_eq!(interp, expect, "interpreter diverged on {tag}");
}

const SAXPY_STORE: &str = "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5, 6, 7, 8];\n    let b = [8, 7, 6, 5, 4, 3, 2, 1];\n    let i = 0;\n    while i < 8 {\n        a[i] = a[i] + b[i];\n        i = i + 1;\n    }\n    let s = 0;\n    let j = 0;\n    while j < 8 {\n        s = s + a[j];\n        j = j + 1;\n    }\n    return s;\n}\n";

#[test]
fn diff_exact_count_store_loop() {
    check_all(SAXPY_STORE, "store", "Main", Ok(72));
}

const ALIASED_WRITE: &str = "fn f(a: Array<Int>, b: Array<Int>, n: Int): Int {\n    let i = 0;\n    while i < n {\n        a[i] = b[i] + 1;\n        i = i + 1;\n    }\n    return a[0] + a[1] + a[2];\n}\nfn Main(): Int {\n    let x = [10, 20, 30];\n    return f(x, x, 3);\n}\n";

#[test]
fn diff_aliased_params_write() {
    check_all(ALIASED_WRITE, "alias", "Main", Ok(63));
}

const BOUNDARY_READ: &str = "fn Main(): Int {\n    let a = [1, 2, 3];\n    let idx = [2];\n    let e: Array<Int> = [];\n    let s = 0;\n    let i = 0;\n    while i < e.length {\n        s = s + e[i];\n        i = i + 1;\n    }\n    return a[idx[0]] * 100 + s;\n}\n";

#[test]
fn diff_boundary_indices() {
    check_all(BOUNDARY_READ, "boundary", "Main", Ok(300));
}

const MINI_MATMUL: &str = "fn Main(): Int {\n    let n = 4;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % n;\n            b[r * n + k] = (r * k) % n;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < 16 {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

#[test]
fn diff_mini_matmul() {
    check_all(MINI_MATMUL, "matmul", "Main", Ok(96));
}

const PARAM_SUM: &str = "fn f(a: Array<Int>, n: Int): Int {\n    let s = 0;\n    let i = 0;\n    while i < n {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\nfn Main(): Int {\n    let a = [3, 1, 4, 1, 5];\n    return f(a, 5) + f(a, 0);\n}\n";

#[test]
fn diff_param_bound_sum() {
    check_all(PARAM_SUM, "paramsum", "Main", Ok(14));
}
