use lir::instr::{CallTarget, Instr, Terminator};

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

fn self_calls(func: &lir::instr::Function, self_id: usize) -> usize {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| match i {
            Instr::Call { target: CallTarget::Fn(id), .. } => *id == self_id,
            _ => false,
        })
        .count()
}

fn has_back_branch(func: &lir::instr::Function) -> bool {
    let targets = |term: &lir::instr::Terminator| -> Vec<usize> {
        match term {
            Terminator::Br(t) => vec![*t],
            Terminator::BrIf { then_bb, else_bb, .. } => vec![*then_bb, *else_bb],
            _ => Vec::new(),
        }
    };
    for (bi, block) in func.blocks.iter().enumerate() {
        if let Terminator::Br(t) = &block.term {
            let mut seen = std::collections::BTreeSet::new();
            let mut stack = vec![*t];
            while let Some(b) = stack.pop() {
                if b == bi {
                    return true;
                }
                if b < func.blocks.len() && seen.insert(b) {
                    stack.extend(targets(&func.blocks[b].term));
                }
            }
        }
    }
    false
}

const SUM_SRC: &str = "fn sumDown(n: Int, acc: Int): Int {\n    if n <= 0 {\n        return acc;\n    }\n    return sumDown(n - 1, acc + n);\n}\nfn Main(): Int {\n    return sumDown(10, 0);\n}\n";

#[test]
fn test_tco_eliminates_self_call() {
    let mut module = lower_src(SUM_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let id = module.fn_id("sumDown").unwrap();
    let func = fn_by_name(&module, "sumDown");
    assert_eq!(self_calls(func, id), 0);
    assert!(has_back_branch(func));
    let out = cli::run_source(SUM_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(55)) => {}
        other => panic!("{other:?}"),
    }
}

const DEEP_SRC: &str = "fn sumDown(n: Int, acc: Int): Int {\n    if n <= 0 {\n        return acc;\n    }\n    return sumDown(n - 1, acc + n);\n}\nfn Main(): Int {\n    let total = sumDown(50000, 0);\n    print(\"tco deep:\", total);\n    if total == 1250025000 {\n        return 0;\n    }\n    return 1;\n}\n";

#[test]
fn test_deep_recursion_no_stack_overflow() {
    let out = cli::run_source(DEEP_SRC, "Main", vec![]);
    assert_eq!(out.output, vec!["tco deep: 1250025000".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(0)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(DEEP_SRC);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 0);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 0);
    let dir = std::env::temp_dir().join(format!("rnx-tco-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("deep.rnx");
    std::fs::write(&src_path, DEEP_SRC).unwrap();
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
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "tco deep: 1250025000\n");
    let _ = std::fs::remove_dir_all(&dir);
}

const NONTAIL_SRC: &str = "fn fibLike(n: Int): Int {\n    if n <= 0 {\n        return 0;\n    }\n    return 1 + fibLike(n - 1);\n}\nfn Main(): Int {\n    return fibLike(10);\n}\n";

const MUTUAL_SRC: &str = "fn isEven(n: Int): Int {\n    if n == 0 {\n        return 1;\n    }\n    return isOdd(n - 1);\n}\nfn isOdd(n: Int): Int {\n    if n == 0 {\n        return 0;\n    }\n    return isEven(n - 1);\n}\nfn Main(): Int {\n    return isEven(10) + isOdd(9);\n}\n";

#[test]
fn test_mutual_or_non_tail_call_preserved() {
    let mut module = lower_src(NONTAIL_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let id = module.fn_id("fibLike").unwrap();
    assert!(self_calls(fn_by_name(&module, "fibLike"), id) > 0);
    let out = cli::run_source(NONTAIL_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(10)) => {}
        other => panic!("{other:?}"),
    }
    let mut module = lower_src(MUTUAL_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let even = module.fn_id("isEven").unwrap();
    let odd = module.fn_id("isOdd").unwrap();
    assert!(self_calls(fn_by_name(&module, "isEven"), even) == 0);
    assert!(self_calls(fn_by_name(&module, "isOdd"), odd) == 0);
    assert!(fn_by_name(&module, "isEven")
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .any(|i| matches!(i, Instr::Call { .. })));
    let out = cli::run_source(MUTUAL_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(2)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_opt_0_preserves_tail_calls() {
    let mut module = lower_src(SUM_SRC);
    lir::opt::optimize_lir(&mut module, 0, "Main");
    let id = module.fn_id("sumDown").unwrap();
    assert!(self_calls(fn_by_name(&module, "sumDown"), id) > 0);
}
