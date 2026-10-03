use lir::instr::Instr;

fn lower_file(path: &std::path::Path) -> lir::instr::Module {
    let graph = frontend::modules::ModuleGraph::build(path).unwrap_or_else(|e| panic!("{e}"));
    let mut m = graph.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"))
}

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

fn has_alloc(func: &lir::instr::Function) -> bool {
    func.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| {
        matches!(
            i,
            Instr::ObjNew { .. } | Instr::StackAlloc { .. }
        )
    })
}

fn has_field_access(func: &lir::instr::Function) -> bool {
    func.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| {
        matches!(
            i,
            Instr::GetField { .. }
                | Instr::GetFieldByName { .. }
                | Instr::SetField { .. }
                | Instr::SetFieldByName { .. }
        )
    })
}

fn fixture(tag: &str, rel: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-sroa-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(rel);
    std::fs::write(&path, src).unwrap();
    path
}

const VEC_XY: &str = "import { Vec2 } from \"@std/math\";\nfn Main(): Int {\n    let v = new Vec2(10.0, 20.0);\n    let x = v.x;\n    let y = v.y;\n    return Int(x + y);\n}\n";

#[test]
fn test_sroa_eliminates_stack_alloc() {
    let path = fixture("xy", "main.rnx", VEC_XY);
    let mut module = lower_file(&path);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    assert!(!has_alloc(main));
    assert!(!has_field_access(main));
    let out = cli::run_files(path.to_str().unwrap(), "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(30)) => {}
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

const COUNTER_SRC: &str = "class Counter {\n    let n: Int;\n    init(v: Int) { this.n = v; }\n}\nfn Main(flag: Int): Int {\n    let c = new Counter(10);\n    if flag == 1 {\n        c.n = c.n + 5;\n    } else {\n        c.n = c.n * 2;\n    }\n    return c.n;\n}\n";

#[test]
fn test_sroa_with_mutation() {
    let mut module = lower_src(COUNTER_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    assert!(!has_alloc(main));
    assert!(!has_field_access(main));
    for (flag, want) in [(1, 15), (0, 20)] {
        let out = cli::run_source(COUNTER_SRC, "Main", vec![runtime::value::Value::Int(flag)]);
        match out.outcome {
            cli::RunOutcome::Value(runtime::value::Value::Int(v)) => assert_eq!(v, want),
            other => panic!("{other:?}"),
        }
    }
}

const LOOP_SRC: &str = "import { Vec2 } from \"@std/math\";\nfn Main(): Int {\n    let total = 0.0;\n    let i = 0;\n    while i < 3 {\n        let a = new Vec2(1.0, 2.0);\n        let b = new Vec2(3.0, 4.0);\n        let d = a.dot(b);\n        let len = a.length();\n        if d > 10.0 && len > 2.0 {\n            total = total + d;\n        }\n        i = i + 1;\n    }\n    total = total + 9.0;\n    print(\"sroa success:\", Int(total));\n    return Int(total);\n}\n";

#[test]
fn test_cross_backend_sroa_execution() {
    let path = fixture("loop", "main.rnx", LOOP_SRC);
    let out = cli::run_files(path.to_str().unwrap(), "Main", vec![]);
    assert_eq!(out.output, vec!["sroa success: 42".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_file(&path);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let bin_path = path.parent().unwrap().join("sroa_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&path)
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "sroa success: 42\n");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

const OPAQUE_SRC: &str = "class Box {\n    let v: Int;\n    init(v: Int) { this.v = v; }\n}\nfn useBox(b: Box): Int {\n    print(\"use\");\n    return b.v;\n}\nfn Main(): Int {\n    let b = new Box(21);\n    let r = useBox(b);\n    return r * 2;\n}\n";

#[test]
fn test_disqualified_aggregate_retains_stack_alloc() {
    let mut module = lower_src(OPAQUE_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    assert!(main
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .any(|i| matches!(i, Instr::StackAlloc { .. })));
    let out = cli::run_source(OPAQUE_SRC, "Main", vec![]);
    assert_eq!(out.output, vec!["use".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

