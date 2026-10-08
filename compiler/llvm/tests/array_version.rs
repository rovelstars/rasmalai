use inkwell::context::Context;

use llvm::codegen::{optimized_ir, Jit};

fn lir_of(src: &str, tag: &str) -> lir::instr::Module {
    let dir = std::env::temp_dir().join(format!("rnx-arrver-{tag}-{}", std::process::id()));
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

fn opt_lir(src: &str, tag: &str, entry: &str) -> lir::instr::Module {
    let mut lir = lir_of(src, tag);
    lir::opt::optimize_lir(&mut lir, 1, entry);
    lir
}

const SUM_TO: &str = "fn sum_to(a: Array<Int>, n: Int): Int {\n    let s = 0;\n    let i = 0;\n    while i < n {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\n";

#[test]
fn versioned_loop_matches_checked_results() {
    let src = format!(
        "{SUM_TO}fn Main(): Int {{\n    let a = [10, 20, 30];\n    let r1 = sum_to(a, 3);\n    let r2 = sum_to(a, 0);\n    let r3 = sum_to(a, 2);\n    return r1 * 10000 + r2 * 100 + r3;\n}}\n"
    );
    let context = Context::create();
    let lir = opt_lir(&src, "jit", "Main");
    let jit = Jit::compile(&context, &lir, "verdiff").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 60 * 10000 + 0 * 100 + 30);
}

#[test]
fn guard_fail_zero_trip_returns_without_trap() {
    let src = format!(
        "{SUM_TO}fn sum_off(a: Array<Int>, n: Int): Int {{\n    let s = 7;\n    let i = 10;\n    while i < n {{\n        s = s + a[i];\n        i = i + 1;\n    }}\n    return s;\n}}\nfn Main(): Int {{\n    let a = [10, 20, 30];\n    return sum_off(a, 5);\n}}\n"
    );
    let context = Context::create();
    let lir = opt_lir(&src, "guardfail", "Main");
    let jit = Jit::compile(&context, &lir, "guardfail").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 7);
}

#[test]
fn versioned_param_loop_vectorizes_in_opt_ir() {
    let src = format!(
        "{SUM_TO}fn Main(): Int {{\n    return 0;\n}}\n"
    );
    let lir = opt_lir(&src, "vec", "sum_to");
    let ir = optimized_ir(&lir, "vervec").unwrap_or_else(|e| panic!("{e}"));
    let mut in_target = false;
    let mut body = String::new();
    for line in ir.lines() {
        if line.starts_with("define") && line.contains("@sum_to(") {
            in_target = true;
        } else if line.starts_with("define") {
            in_target = false;
        }
        if in_target {
            body.push_str(line);
            body.push('\n');
        }
    }
    assert!(body.contains("define"), "no @sum_to in post-O3 IR");
    assert!(
        body.contains("vector.body"),
        "versioned param loop must vectorize; got:\n{body}"
    );
}

#[test]
fn nested_spectral_shape_vectorizes_inner() {
    let src = "fn inner(x: Array<Int>, n: Int): Int {\n    let s = 0;\n    let j = 0;\n    while j < n {\n        s = s + x[j] * 2;\n        j = j + 1;\n    }\n    return s;\n}\nfn outer(x: Array<Int>, n: Int): Int {\n    let t = 0;\n    let i = 0;\n    while i < n {\n        t = t + inner(x, n);\n        i = i + 1;\n    }\n    return t;\n}\nfn Main(): Int {\n    return 0;\n}\n";
    let lir = opt_lir(src, "spectral", "outer");
    let ir = optimized_ir(&lir, "specvec").unwrap_or_else(|e| panic!("{e}"));
    assert!(
        ir.contains("vector.body"),
        "nested spectral shape must vectorize its inner loop; got:\n{ir}"
    );
}
