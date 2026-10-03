use lir::instr::{Instr, Lit, Terminator};

fn lower_src(src: &str) -> lir::instr::Module {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"))
}

fn count_arith(module: &lir::instr::Module) -> usize {
    module
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Arith { .. }))
        .count()
}

fn int_consts(module: &lir::instr::Module) -> Vec<i64> {
    module
        .functions
        .iter()
        .flat_map(|f| f.blocks.iter())
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::Const { lit: Lit::Int(n), .. } => Some(*n),
            _ => None,
        })
        .collect()
}

fn has_brif(module: &lir::instr::Module) -> bool {
    module.functions.iter().flat_map(|f| f.blocks.iter()).any(|b| {
        matches!(b.term, Terminator::BrIf { .. })
    })
}

#[test]
fn test_constant_folding() {
    let src = "fn compute(): Int { return 10 * 4 + (5 - 3); }";
    let mut module = lower_src(src);
    assert!(count_arith(&module) > 0);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert_eq!(count_arith(&module), 0);
    assert!(int_consts(&module).contains(&42));
    let out = cli::run_source(src, "compute", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_dead_branch_elimination() {
    let src = "fn pick(): Int { if false { return 999; } else { return 42; } }";
    let mut module = lower_src(src);
    assert!(has_brif(&module));
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert!(!has_brif(&module));
    let blocks: usize = module.functions.iter().map(|f| f.blocks.len()).sum();
    assert_eq!(blocks, 2);
    let ints = int_consts(&module);
    assert!(!ints.contains(&999), "{ints:?}");
    assert!(ints.contains(&42), "{ints:?}");
    let out = cli::run_source(src, "pick", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

const WORKLOAD: &str = "fn Main(): Int {\n    let x = 10 * 4 + (5 - 3);\n    if x == 42 {\n        print(\"opt success:\", x);\n        return x;\n    }\n    return 0;\n}\n";

#[test]
fn test_opt_all_backends() {
    let out = cli::run_source(WORKLOAD, "Main", vec![]);
    assert_eq!(out.output, vec!["opt success: 42".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(WORKLOAD);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let dir = std::env::temp_dir().join(format!("rnx-opt-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("work.rnx");
    std::fs::write(&src_path, WORKLOAD).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    for level in ["0", "1"] {
        let run = std::process::Command::new(rnx)
            .arg("run")
            .arg("-O")
            .arg(level)
            .arg(&src_path)
            .output()
            .unwrap();
        assert_eq!(run.status.code().unwrap(), 42, "-O {level}");
        assert_eq!(
            String::from_utf8(run.stdout).unwrap(),
            "opt success: 42\n",
            "-O {level}"
        );
    }
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_opt_level_zero_preserves_lir() {
    let src = "fn pick(): Int { if false { return 999; } else { return 42; } }";
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 0, "Main");
    assert!(has_brif(&module));
    assert!(count_arith(&lower_src("fn compute(): Int { return 10 * 4 + (5 - 3); }")) > 0);
    let ints = int_consts(&module);
    assert!(ints.contains(&999), "{ints:?}");
}

fn fn_by_name<'a>(module: &'a lir::instr::Module, name: &str) -> &'a lir::instr::Function {
    module
        .functions
        .iter()
        .find(|f| f.name == name)
        .unwrap_or_else(|| panic!("no function `{name}`"))
}

fn count_calls(func: &lir::instr::Function) -> usize {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Call { .. }))
        .count()
}

fn count_fn_calls(func: &lir::instr::Function) -> usize {
    use lir::instr::CallTarget;
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| {
            matches!(
                i,
                Instr::Call {
                    target: CallTarget::Fn(_) | CallTarget::Method { .. },
                    ..
                }
            )
        })
        .count()
}

#[test]
fn test_leaf_inlining_eliminates_call() {
    let src = "fn add(a: Int, b: Int): Int { return a + b; }\nfn Main(): Int { return add(20, 22); }\n";
    let mut module = lower_src(src);
    assert!(count_calls(fn_by_name(&module, "Main")) > 0);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert_eq!(count_calls(fn_by_name(&module, "Main")), 0);
    let out = cli::run_source(src, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_inline_with_constant_folding() {
    let src = "fn square(x: Int): Int { return x * x; }\nfn Main(): Int { return square(6); }\n";
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    assert_eq!(count_calls(main), 0);
    let arith = main
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Arith { .. }))
        .count();
    assert_eq!(arith, 0);
    let ints: Vec<i64> = main
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::Const { lit: Lit::Int(n), .. } => Some(*n),
            _ => None,
        })
        .collect();
    assert!(ints.contains(&36), "{ints:?}");
}

#[test]
fn test_non_leaf_not_inlined() {
    let src = "fn makeC(): Int { return 1; }\nfn makeB(): Int { return makeC(); }\nfn makeA(): Int { return makeB(); }\nfn Main(): Int { return makeA(); }\n";
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert!(count_calls(fn_by_name(&module, "makeA")) > 0);
    assert_eq!(count_calls(fn_by_name(&module, "makeB")), 0);
}

#[test]
fn test_opt_0_preserves_calls() {
    let src = "fn add(a: Int, b: Int): Int { return a + b; }\nfn Main(): Int { return add(20, 22); }\n";
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 0, "Main");
    assert!(count_calls(fn_by_name(&module, "Main")) > 0);
}

const INLINE_WORKLOAD: &str = "fn pick(x: Int): Int {\n    if x == 0 {\n        return 7;\n    }\n    return x * 6;\n}\nfn Main(): Int {\n    let a = pick(0);\n    let b = pick(7);\n    print(\"inline success:\", a, b);\n    return b;\n}\n";

#[test]
fn test_cross_backend_inlining_run() {
    let out = cli::run_source(INLINE_WORKLOAD, "Main", vec![]);
    assert_eq!(out.output, vec!["inline success: 7 42".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(INLINE_WORKLOAD);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let main = fn_by_name(&for_jit, "Main");
    assert_eq!(count_fn_calls(main), 0);
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let dir = std::env::temp_dir().join(format!("rnx-inline-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("inline.rnx");
    std::fs::write(&src_path, INLINE_WORKLOAD).unwrap();
    let bin_path = dir.join("inline_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "inline success: 7 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

fn fn_names(module: &lir::instr::Module) -> Vec<String> {
    module.functions.iter().map(|f| f.name.clone()).collect()
}

#[test]
fn test_inlined_function_stripped() {
    let src = "fn helper(x: Int): Int { return x + 1; }\nfn Main(): Int { return helper(41); }\n";
    let mut module = lower_src(src);
    assert!(fn_names(&module).contains(&"helper".to_string()));
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert!(!fn_names(&module).contains(&"helper".to_string()));
    assert!(fn_names(&module).contains(&"Main".to_string()));
    let out = cli::run_source(src, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

#[test]
fn test_uncalled_dead_function_stripped() {
    let src = "fn deadHelper(): Int { return 999; }\nfn Main(): Int { return 42; }\n";
    let mut module = lower_src(src);
    assert!(fn_names(&module).contains(&"deadHelper".to_string()));
    lir::opt::optimize_lir(&mut module, 1, "Main");
    assert!(!fn_names(&module).contains(&"deadHelper".to_string()));
}

#[test]
fn test_opt_0_preserves_dead_functions() {
    let src = "fn helper(x: Int): Int { return x + 1; }\nfn deadHelper(): Int { return 999; }\nfn Main(): Int { return helper(41); }\n";
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 0, "Main");
    let names = fn_names(&module);
    assert!(names.contains(&"helper".to_string()), "{names:?}");
    assert!(names.contains(&"deadHelper".to_string()), "{names:?}");
}

#[test]
fn test_indirect_call_reachability() {
    let src = "fn makeC(): Int { print(\"c\"); return 3; }\nfn makeB(): Int { return makeC() * 2; }\nfn makeA(): Int { return makeB() * 7; }\nfn Main(): Int { return makeA(); }\n";
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let names = fn_names(&module);
    assert!(names.contains(&"makeA".to_string()), "{names:?}");
    assert!(names.contains(&"makeB".to_string()), "{names:?}");
    assert!(names.contains(&"makeC".to_string()), "{names:?}");
    let out = cli::run_source(src, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

const DEAD_ELIM_WORKLOAD: &str = "fn unused(): Int { return 999; }\nfn double(x: Int): Int { return x * 2; }\nfn Main(): Int {\n    let v = double(21);\n    print(\"dead elim success:\", v);\n    return v;\n}\n";

#[test]
fn test_cross_backend_compilation_and_execution() {
    let out = cli::run_source(DEAD_ELIM_WORKLOAD, "Main", vec![]);
    assert_eq!(out.output, vec!["dead elim success: 42".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(DEAD_ELIM_WORKLOAD);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    assert!(!fn_names(&for_jit).contains(&"unused".to_string()));
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let dir = std::env::temp_dir().join(format!("rnx-deadelim-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("dead.rnx");
    std::fs::write(&src_path, DEAD_ELIM_WORKLOAD).unwrap();
    let bin_path = dir.join("dead_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&src_path)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "dead elim success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}
