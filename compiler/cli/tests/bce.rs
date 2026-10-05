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

fn array_gets(func: &lir::instr::Function) -> Vec<bool> {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter_map(|i| match i {
            Instr::ArrayGet { unchecked, .. } => Some(*unchecked),
            _ => None,
        })
        .collect()
}

const SUM_SRC: &str = "fn Main(): Int {\n    let arr = [10, 20, 30, 40];\n    let sum = 0;\n    let i = 0;\n    while i < arr.length {\n        sum = sum + arr[i];\n        i = i + 1;\n    }\n    return sum;\n}\n";

#[test]
fn test_bce_marks_loop_array_access_unchecked() {
    let mut module = lower_src(SUM_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    let gets = array_gets(main);
    assert!(!gets.is_empty());
    assert!(gets.iter().all(|u| *u), "{gets:?}");
    let out = cli::run_source(SUM_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(100)) => {}
        other => panic!("{other:?}"),
    }
}

const PUSH_SRC: &str = "fn Main(): Int {\n    let arr = [1, 2, 3];\n    let sum = 0;\n    let i = 0;\n    let pushed = 0;\n    while i < arr.length {\n        sum = sum + arr[i];\n        if pushed == 0 {\n            arr.push(4);\n            pushed = 1;\n        }\n        i = i + 1;\n    }\n    return sum;\n}\n";

#[test]
fn test_bce_disqualified_on_mutation() {
    let mut module = lower_src(PUSH_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    let gets = array_gets(main);
    assert!(!gets.is_empty());
    assert!(gets.iter().all(|u| !*u), "{gets:?}");
    let out = cli::run_source(PUSH_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(10)) => {}
        other => panic!("{other:?}"),
    }
}

fn big_array_src() -> String {
    let elems: Vec<String> = (0..100).map(|n| n.to_string()).collect();
    format!(
        "fn Main(): Int {{\n    let arr = [{}];\n    let sum = 0;\n    let i = 0;\n    while i < arr.length {{\n        sum = sum + arr[i];\n        i = i + 1;\n    }}\n    print(\"bce success:\", sum);\n    if sum == 4950 {{\n        return 42;\n    }}\n    return 0;\n}}\n",
        elems.join(", ")
    )
}

#[test]
fn test_bce_cross_backend_execution() {
    let src = big_array_src();
    let out = cli::run_source(&src, "Main", vec![]);
    assert_eq!(out.output, vec!["bce success: 4950".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(&src);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let main = fn_by_name(&for_jit, "Main");
    assert!(array_gets(main).iter().all(|u| *u));
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let dir = std::env::temp_dir().join(format!("rnx-bce-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("acc.rnx");
    std::fs::write(&src_path, &src).unwrap();
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
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "bce success: 4950\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_opt_0_preserves_bounds_checks() {
    let mut module = lower_src(SUM_SRC);
    lir::opt::optimize_lir(&mut module, 0, "Main");
    let main = fn_by_name(&module, "Main");
    let gets = array_gets(main);
    assert!(!gets.is_empty());
    assert!(gets.iter().all(|u| !*u), "{gets:?}");
}

const CONST_SRC: &str = "fn Main(): Int {\n    let a = [1, 2, 3, 4, 5];\n    let s = 0;\n    let i = 0;\n    while i < 5 {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\n";

#[test]
fn test_bce_const_bound_cross_backend() {
    let mut module = lower_src(CONST_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    let gets = array_gets(main);
    assert!(!gets.is_empty());
    assert!(gets.iter().filter(|u| **u).count() >= 1, "{gets:?}");
    let out = cli::run_source(CONST_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(15)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(CONST_SRC);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 15);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 15);
}

const SHORT_SRC: &str = "fn Main(): Int {\n    let a = [1, 2];\n    let s = 0;\n    let i = 0;\n    while i < 5 {\n        s = s + a[i];\n        i = i + 1;\n    }\n    return s;\n}\n";

#[test]
fn test_bce_short_array_still_traps() {
    let out = cli::run_source(SHORT_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Thrown(_) | cli::RunOutcome::Fatal { .. } => {}
        other => panic!("interpreter: {other:?}"),
    }
    let dir = std::env::temp_dir().join(format!("rnx-bce-trap-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("short.rnx");
    std::fs::write(&src_path, SHORT_SRC).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    for backend in ["cranelift", "llvm"] {
        let run = std::process::Command::new(rnx)
            .arg("run")
            .arg("--backend")
            .arg(backend)
            .arg(&src_path)
            .output()
            .unwrap();
        assert!(!run.status.success(), "{backend} read past the end without trapping");
        let stderr = String::from_utf8_lossy(&run.stderr).into_owned();
        assert!(stderr.contains("index out of bounds"), "{backend}: {stderr}");
    }
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg("--release")
        .arg(&src_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert!(!run.status.success(), "aot read past the end without trapping");
    let _ = std::fs::remove_dir_all(&dir);
}
