use lir::instr::{ArithOp, Instr, NumKind};

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

fn loop_blocks(func: &lir::instr::Function) -> std::collections::BTreeSet<usize> {
    let dom = lir::licm::compute_dominators(func);
    let loops = lir::licm::find_natural_loops(func, &dom);
    assert_eq!(loops.len(), 1);
    loops.into_iter().next().unwrap().blocks
}

fn adds_in(func: &lir::instr::Function, blocks: &std::collections::BTreeSet<usize>) -> usize {
    blocks
        .iter()
        .flat_map(|b| func.blocks[*b].instrs.iter())
        .filter(|i| matches!(i, Instr::Arith { op: ArithOp::Add, .. }))
        .count()
}

const INV_SRC: &str = "fn Main(): Int {\n    let sum = 0;\n    let i = 0;\n    let a = 10;\n    let b = 20;\n    while i < 100 {\n        let inv = a + b;\n        sum = sum + inv;\n        i = i + 1;\n    }\n    return sum;\n}\n";

#[test]
fn test_licm_hoists_invariant_arithmetic() {
    let mut module = lower_src(INV_SRC);
    lir::opt::inline_call_sites(&mut module);
    {
        let main = fn_by_name(&module, "Main");
        let body = loop_blocks(main);
        assert!(adds_in(main, &body) >= 3);
    }
    for f in &mut module.functions {
        lir::licm::loop_invariant_code_motion(f);
    }
    let main = fn_by_name(&module, "Main");
    let body = loop_blocks(main);
    assert_eq!(adds_in(main, &body), 2);
    let outside: usize = (0..main.blocks.len())
        .filter(|b| !body.contains(b))
        .flat_map(|b| main.blocks[b].instrs.iter())
        .filter(|i| matches!(i, Instr::Arith { op: ArithOp::Add, kind: NumKind::Int, .. }))
        .count();
    assert!(outside >= 1);
    let out = cli::run_source(INV_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(3000)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_licm_preserves_variant_expressions() {
    use lir::instr::Local;
    let mut module = lower_src(INV_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    let body = loop_blocks(main);
    let mut loop_defs = std::collections::BTreeSet::<Local>::new();
    for b in &body {
        for ins in &main.blocks[*b].instrs {
            match ins {
                Instr::Const { dst, .. }
                | Instr::Copy { dst, .. }
                | Instr::Arith { dst, .. } => {
                    loop_defs.insert(*dst);
                }
                _ => {}
            }
        }
    }
    for b in &body {
        for ins in &main.blocks[*b].instrs {
            if let Instr::Arith { lhs, rhs, .. } = ins {
                assert!(
                    loop_defs.contains(lhs) || loop_defs.contains(rhs),
                    "{ins:?}"
                );
            }
        }
    }
}

const NESTED_SRC: &str = "fn Main(): Int {\n    let total = 2;\n    let i = 0;\n    let k = 8;\n    while i < 2 {\n        let j = 0;\n        while j < 2 {\n            let inv = k + 1;\n            total = total + inv + i + j;\n            j = j + 1;\n        }\n        i = i + 1;\n    }\n    print(\"licm success:\", total);\n    return total;\n}\n";

#[test]
fn test_cross_backend_licm_execution() {
    let out = cli::run_source(NESTED_SRC, "Main", vec![]);
    assert_eq!(out.output, vec!["licm success: 42".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(NESTED_SRC);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let dir = std::env::temp_dir().join(format!("rnx-licm-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("nested.rnx");
    std::fs::write(&src_path, NESTED_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&src_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "licm success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_opt_0_skips_licm() {
    let mut module = lower_src(INV_SRC);
    lir::opt::optimize_lir(&mut module, 0, "Main");
    let main = fn_by_name(&module, "Main");
    let body = loop_blocks(main);
    assert!(adds_in(main, &body) >= 3);
}

const ZERO_TRIP_SRC: &str = "fn fInt(n: Int): Int {\n    let a = 5;\n    let i = 0;\n    while i < n {\n        if i == 99 { a = 7; }\n        i = i + 1;\n    }\n    return a;\n}\nfn fBool(n: Int): Bool {\n    let a = true;\n    let i = 0;\n    while i < n {\n        if i == 99 { a = false; }\n        i = i + 1;\n    }\n    return a;\n}\nfn fStr(n: Int): String {\n    let a = \"keep\";\n    let i = 0;\n    while i < n {\n        if i == 99 { a = \"leak\"; }\n        i = i + 1;\n    }\n    return a;\n}\nfn Main(): Int {\n    print(fInt(0));\n    print(fBool(0));\n    print(fStr(0));\n    print(fInt(200));\n    return 0;\n}\n";

const ZERO_TRIP_WANT: [&str; 4] = ["5", "true", "keep", "7"];

#[test]
fn test_licm_does_not_hoist_outer_defined_stores() {
    let mut module = lower_src(
        "fn Main(): Int {\n    let a = 5;\n    let i = 0;\n    while i < 0 {\n        if i == 99 { a = 7; }\n        i = i + 1;\n    }\n    return a;\n}\n",
    );
    let snapshot = |main: &lir::instr::Function| -> Vec<String> {
        let dom = lir::licm::compute_dominators(main);
        let loops = lir::licm::find_natural_loops(main, &dom);
        assert_eq!(loops.len(), 1);
        let body = &loops.into_iter().next().unwrap().blocks;
        let ret_local = main
            .blocks
            .iter()
            .find_map(|b| match &b.term {
                lir::instr::Terminator::Ret(v) => v.first().copied(),
                _ => None,
            })
            .expect("Main returns a local");
        let mut out = Vec::new();
        for (bi, block) in main.blocks.iter().enumerate() {
            if body.contains(&bi) {
                continue;
            }
            for ins in &block.instrs {
                let writes_ret = match ins {
                    Instr::Copy { dst, .. } | Instr::Const { dst, .. } => *dst == ret_local,
                    _ => false,
                };
                if writes_ret {
                    out.push(format!("{ins:?}"));
                }
            }
        }
        out
    };
    let before = snapshot(fn_by_name(&module, "Main"));
    assert!(!before.is_empty());
    for f in &mut module.functions {
        lir::licm::loop_invariant_code_motion(f);
    }
    assert_eq!(snapshot(fn_by_name(&module, "Main")), before);
}

#[test]
fn test_cross_backend_zero_trip_loop_keeps_initial() {
    let out = cli::run_source(ZERO_TRIP_SRC, "Main", vec![]);
    let want: Vec<String> = ZERO_TRIP_WANT.iter().map(|s| s.to_string()).collect();
    assert_eq!(out.output, want);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(0)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(ZERO_TRIP_SRC);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 0);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 0);
    let dir = std::env::temp_dir().join(format!("rnx-licm-zerotrip-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("zerotrip.rnx");
    std::fs::write(&src_path, ZERO_TRIP_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&src_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 0);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "5\ntrue\nkeep\n7\n");
    let _ = std::fs::remove_dir_all(&dir);
}
