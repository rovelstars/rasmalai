use inkwell::context::Context;

use llvm::codegen::Jit;

fn lir_of(src: &str, tag: &str) -> lir::instr::Module {
    let dir = std::env::temp_dir().join(format!("rnx-factmeta-{tag}-{}", std::process::id()));
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

fn fn_ir(ir: &str, name: &str) -> String {
    let mut in_target = false;
    let mut body = String::new();
    let mut tails = String::new();
    let marker = format!("@{name}(");
    for line in ir.lines() {
        if line.starts_with("define") {
            in_target = line.contains(&marker);
        }
        if line.starts_with('!') {
            tails.push_str(line);
            tails.push('\n');
        } else if in_target {
            body.push_str(line);
            body.push('\n');
        }
    }
    assert!(body.contains("define"), "no @{name} in IR");
    body.push_str(&tails);
    body
}

const FRESH_PAIR: &str = "fn Main(): Int {\n    let a = [1, 2, 3, 4];\n    let b = [5, 6, 7, 8];\n    let s = 0;\n    let i = 0;\n    while i < 4 {\n        s = s + a[i] + b[i];\n        i = i + 1;\n    }\n    return s;\n}\n";

#[test]
fn exact_count_loop_emits_all_facts() {
    let context = Context::create();
    let lir = opt_lir(FRESH_PAIR, "facts", "Main");
    let jit = Jit::compile(&context, &lir, "facts").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 36);
    let body = fn_ir(&jit.ir(), "Main");
    let tagged: Vec<&str> = body.lines().filter(|l| l.contains("llvm.access.group")).collect();
    assert!(!tagged.is_empty(), "loads need access groups; got:\n{body}");
    assert!(
        tagged.iter().all(|l| l.contains(", align 8")),
        "tagged element loads need align 8; got:\n{body}"
    );
    assert!(body.contains(" nuw ") && body.contains(" nsw "), "induction increment needs nuw+nsw; got:\n{body}");
    assert!(body.contains("!range"), "induction loads need !range; got:\n{body}");
    assert!(
        body.contains("llvm.loop.parallel_accesses"),
        "fresh-array loop needs parallel_accesses; got:\n{body}"
    );
    assert!(body.contains("llvm.access.group"), "loads need access groups; got:\n{body}");
    assert!(
        body.contains("llvm.loop.estimated_trip_count"),
        "exact-count loop needs trip count; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.unroll.count"),
        "call-free loop keeps its unroll hint; got:\n{body}"
    );
}

const ALIASED: &str = "fn f(a: Array<Int>, b: Array<Int>, n: Int): Int {\n    let s = 0;\n    let i = 0;\n    while i < n {\n        s = s + a[i] + b[i];\n        i = i + 1;\n    }\n    return s;\n}\nfn Main(): Int {\n    return 0;\n}\n";

#[test]
fn aliased_params_get_no_parallel_accesses() {
    let context = Context::create();
    let lir = opt_lir(ALIASED, "aliased", "f");
    let jit = Jit::compile(&context, &lir, "aliased").unwrap_or_else(|e| panic!("{e}"));
    let body = fn_ir(&jit.ir(), "f");
    assert!(
        !body.contains("llvm.loop.parallel_accesses"),
        "may-alias params must not get parallel_accesses; got:\n{body}"
    );
    assert!(!body.contains("llvm.access.group"), "may-alias loads must stay untagged; got:\n{body}");
}

#[test]
fn unknown_bound_gets_no_nuw_or_trip_count() {
    let context = Context::create();
    let lir = opt_lir(ALIASED, "unknown", "f");
    let jit = Jit::compile(&context, &lir, "unknown").unwrap_or_else(|e| panic!("{e}"));
    let body = fn_ir(&jit.ir(), "f");
    assert!(!body.contains(" nuw "), "unknown bound must not get nuw; got:\n{body}");
    assert!(!body.contains(" nsw "), "unknown bound must not get nsw; got:\n{body}");
    assert!(
        !body.contains("llvm.loop.estimated_trip_count"),
        "unknown bound must not get trip count; got:\n{body}"
    );
    assert!(!body.contains("!range"), "unknown bound must not get !range; got:\n{body}");
}

const ZERO_TRIP: &str = "fn Main(): Int {\n    let s = 7;\n    let i = 10;\n    while i < 5 {\n        s = s + i;\n        i = i + 1;\n    }\n    return s;\n}\n";

#[test]
fn zero_trip_loop_reports_zero_count() {
    let context = Context::create();
    let lir = opt_lir(ZERO_TRIP, "zero", "Main");
    let jit = Jit::compile(&context, &lir, "zero").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 7);
    let body = fn_ir(&jit.ir(), "Main");
    assert!(
        body.contains("llvm.loop.estimated_trip_count"),
        "zero-trip loop needs trip count; got:\n{body}"
    );
}

const PROVEN_INNER: &str = "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16];\n    let b = [16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];\n    let s = 0;\n    let i = 0;\n    while i < 16 {\n        s = s + a[i] + b[i];\n        i = i + 1;\n    }\n    return s;\n}\n";

#[test]
fn proven_unit_stride_inner_gets_vectorize_hints() {
    let context = Context::create();
    let lir = opt_lir(PROVEN_INNER, "proven", "Main");
    let jit = Jit::compile(&context, &lir, "proven").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 272);
    let body = fn_ir(&jit.ir(), "Main");
    assert!(
        body.contains("llvm.loop.vectorize.enable"),
        "proven inner needs vectorize.enable; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.interleave.count"),
        "proven inner needs interleave.count; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.unroll.count"),
        "load-only inner keeps its unroll hint; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.parallel_accesses"),
        "proven inner keeps parallel_accesses; got:\n{body}"
    );
    assert!(
        body.contains("getelementptr inbounds"),
        "hinted loop keeps inbounds GEP shape; got:\n{body}"
    );
    assert!(body.contains("!tbaa"), "hinted loop keeps tbaa; got:\n{body}");
    assert!(body.contains("!range"), "hinted loop keeps range; got:\n{body}");
}

const VECTOR_PAIR: &str = "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 14, 15, 16, 17, 18, 19, 20];\n    let b = [20, 19, 18, 17, 16, 15, 14, 13, 12, 11, 10, 9, 8, 7, 6, 5, 4, 3, 2, 1];\n    let i = 0;\n    while i < 20 {\n        a[i] = b[i] + 1;\n        i = i + 1;\n    }\n    return a[0] + a[19];\n}\n";

#[test]
fn vectorized_inner_loses_unroll_count() {
    let context = Context::create();
    let lir = opt_lir(VECTOR_PAIR, "vecpair", "Main");
    let jit = Jit::compile(&context, &lir, "vecpair").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 23);
    let body = fn_ir(&jit.ir(), "Main");
    assert!(
        !body.contains("llvm.loop.unroll.count"),
        "vectorized inner must lose unroll.count; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.vectorize.enable"),
        "vectorized inner needs vectorize.enable; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.interleave.count"),
        "vectorized inner needs interleave.count; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.parallel_accesses"),
        "vectorized inner keeps parallel_accesses; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.estimated_trip_count"),
        "vectorized inner keeps trip count; got:\n{body}"
    );
    assert!(
        body.contains("getelementptr inbounds"),
        "hinted loop keeps inbounds GEP shape; got:\n{body}"
    );
    assert!(body.contains("!tbaa"), "hinted loop keeps tbaa; got:\n{body}");
    assert!(
        body.contains("llvm.access.group"),
        "hinted loop keeps access groups; got:\n{body}"
    );
    assert!(body.contains("!range"), "hinted loop keeps range; got:\n{body}");
}

const DIVISIBLE_PAIR: &str = "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5, 6, 7, 8];\n    let b = [8, 7, 6, 5, 4, 3, 2, 1];\n    let i = 0;\n    while i < 8 {\n        a[i] = b[i] + 1;\n        i = i + 1;\n    }\n    return a[0] + a[7];\n}\n";

#[test]
fn small_trip_pair_keeps_existing_hints() {
    let context = Context::create();
    let lir = opt_lir(DIVISIBLE_PAIR, "divpair", "Main");
    let jit = Jit::compile(&context, &lir, "divpair").unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 11);
    let body = fn_ir(&jit.ir(), "Main");
    assert!(
        body.contains("llvm.loop.unroll.count"),
        "small-trip pair keeps its unroll hint; got:\n{body}"
    );
    assert!(
        !body.contains("llvm.loop.vectorize.enable"),
        "small-trip pair gets no vectorize.enable; got:\n{body}"
    );
    assert!(
        !body.contains("llvm.loop.interleave.count"),
        "small-trip pair gets no interleave.count; got:\n{body}"
    );
}

const ALIASED_PAIR: &str = "fn f(a: Array<Int>, b: Array<Int>): Int {\n    let i = 0;\n    while i < 8 {\n        a[i] = b[i] + 1;\n        i = i + 1;\n    }\n    return a[0];\n}\nfn Main(): Int {\n    return 0;\n}\n";

#[test]
fn unproven_pair_gets_no_vectorize_hints() {
    let context = Context::create();
    let lir = opt_lir(ALIASED_PAIR, "unproven", "f");
    let jit = Jit::compile(&context, &lir, "unproven").unwrap_or_else(|e| panic!("{e}"));
    let body = fn_ir(&jit.ir(), "f");
    assert!(
        !body.contains("llvm.loop.vectorize.enable"),
        "unproven loop gets no vectorize.enable; got:\n{body}"
    );
    assert!(
        !body.contains("llvm.loop.interleave.count"),
        "unproven loop gets no interleave.count; got:\n{body}"
    );
    assert!(
        !body.contains("llvm.loop.parallel_accesses"),
        "unproven loop gets no parallel_accesses; got:\n{body}"
    );
    assert!(
        body.contains("llvm.loop.unroll.count"),
        "may-alias pair keeps its unroll hint; got:\n{body}"
    );
}
