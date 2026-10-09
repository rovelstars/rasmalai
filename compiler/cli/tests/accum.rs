use lir::instr::Instr;

fn lower_src(src: &str) -> lir::instr::Module {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"))
}

fn fn_by_name<'a>(module: &'a lir::instr::Module, name: &str) -> &'a lir::instr::Function {
    module
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

fn has_accumulation_store(func: &lir::instr::Function) -> bool {
    for block in &func.blocks {
        for ins in &block.instrs {
            if let Instr::ArraySet { value, .. } = ins {
                let mut found = false;
                for b in &func.blocks {
                    for nested in &b.instrs {
                        match nested {
                            Instr::Arith {
                                op: lir::instr::ArithOp::Add,
                                kind: lir::instr::NumKind::Int,
                                dst,
                                ..
                            } if *dst == *value => found = true,
                            _ => {}
                        }
                    }
                }
                if found {
                    return true;
                }
            }
        }
    }
    false
}

const SMALL_ZERO: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % 8;\n            b[r * n + k] = (r * k) % 8;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

const SMALL_GARBAGE: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(7);\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % 8;\n            b[r * n + k] = (r * k) % 8;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            c[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

const SMALL_ALIASED: &str = "fn Main(): Int {\n    let n = 16;\n    let a: Array<Int> = [];\n    let b: Array<Int> = [];\n    let c: Array<Int> = [];\n    let i = 0;\n    while i < n * n {\n        a.push(0);\n        b.push(0);\n        c.push(0);\n        i = i + 1;\n    }\n    let r = 0;\n    while r < n {\n        let k = 0;\n        while k < n {\n            a[r * n + k] = (r + k) % 8;\n            b[r * n + k] = (r * k) % 8;\n            k = k + 1;\n        }\n        r = r + 1;\n    }\n    let x = 0;\n    while x < n {\n        let y = 0;\n        while y < n {\n            let s = 0;\n            let z = 0;\n            while z < n {\n                s = s + a[x * n + z] * b[z * n + y];\n                z = z + 1;\n            }\n            a[x * n + y] = s;\n            y = y + 1;\n        }\n        x = x + 1;\n    }\n    let total = 0;\n    let t = 0;\n    while t < n * n {\n        total = total + c[t];\n        t = t + 1;\n    }\n    return total;\n}\n";

fn cross_backend_value(src: &str, expect: i64) {
    let out = cli::run_source(src, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(n)) => assert_eq!(n, expect),
        other => panic!("interpreter: {other:?}"),
    }
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&module).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), expect);
    assert_eq!(llvm::codegen::execute(&module, "Main").unwrap(), expect);
}

#[test]
fn test_accum_zero_filled_cross_backend() {
    cross_backend_value(SMALL_ZERO, 39424);
    let mut module = lower_src(SMALL_ZERO);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert!(has_accumulation_store(fn_by_name(&module, "Main")));
}

#[test]
fn test_accum_garbage_prefilled_cross_backend() {
    cross_backend_value(SMALL_GARBAGE, 39424);
    let mut module = lower_src(SMALL_GARBAGE);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert!(has_accumulation_store(fn_by_name(&module, "Main")));
}

#[test]
fn test_accum_aliased_cross_backend() {
    cross_backend_value(SMALL_ALIASED, 0);
    let module = lower_src(SMALL_ALIASED);
    for func in &module.functions {
        let before = format!("{func:?}");
        let mut work = func.clone();
        let (action, _) = lir::accum::accumulate_interchange(&mut work);
        assert!(matches!(action, lir::accum::AccumAction::Untouched));
        assert_eq!(format!("{work:?}"), before);
    }
}
