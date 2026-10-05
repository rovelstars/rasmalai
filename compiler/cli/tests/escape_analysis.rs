use lir::instr::Instr;

fn lower_src(src: &str) -> lir::instr::Module {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"))
}

fn lower_file(path: &std::path::Path) -> lir::instr::Module {
    let graph = frontend::modules::ModuleGraph::build(path).unwrap_or_else(|e| panic!("{e}"));
    let mut m = graph.resolve().unwrap_or_else(|e| panic!("{e}"));
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

fn count_kind(func: &lir::instr::Function, stack: bool) -> usize {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| match i {
            Instr::StackAlloc { .. } => stack,
            Instr::ObjNew { .. } => !stack,
            _ => false,
        })
        .count()
}

fn has_arc(func: &lir::instr::Function) -> bool {
    func.blocks.iter().flat_map(|b| b.instrs.iter()).any(|i| {
        matches!(
            i,
            Instr::Retain { .. } | Instr::Release { .. }
        )
    })
}

const VEC2_SRC: &str = "import { Vec2 } from \"@std/math\";\nfn Main(): Int {\n    let v1 = new Vec2(3.0, 4.0);\n    let len = v1.length();\n    return Int(len);\n}\n";

fn fixture(tag: &str, rel: &str, src: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-escape-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let path = dir.join(rel);
    std::fs::write(&path, src).unwrap();
    path
}

#[test]
fn test_vec2_stack_promotion() {
    let path = fixture("vec2", "main.rnx", VEC2_SRC);
    let mut module = lower_file(&path);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    assert!(count_kind(main, true) > 0);
    assert!(!has_arc(main));
    let out = cli::run_files(path.to_str().unwrap(), "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(5)) => {}
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}

#[test]
fn test_escaping_object_remains_heap() {
    let src = "class Box {\n    let v: Int;\n    init(v: Int) { this.v = v; }\n}\nfn make(): Box { return new Box(7); }\nfn Main(): Int {\n    let b = make();\n    return b.v;\n}\n";
    let mut module = lower_src(src);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let make = fn_by_name(&module, "make");
    assert!(count_kind(make, false) > 0);
    assert_eq!(count_kind(make, true), 0);
    let out = cli::run_source(src, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(7)) => {}
        other => panic!("{other:?}"),
    }
}

const LOOP_SRC: &str = "import { Vec2 } from \"@std/math\";\nfn Main(): Int {\n    let total = 10.0;\n    let i = 0;\n    while i < 2 {\n        let a = new Vec2(3.0, 4.0);\n        let b = new Vec2(1.0, 2.0);\n        total = total + a.dot(b) + a.length();\n        i = i + 1;\n    }\n    let res = Int(total);\n    print(\"escape success:\", res);\n    return res;\n}\n";

#[test]
fn test_cross_backend_stack_alloc() {
    let path = fixture("loop", "loop.rnx", LOOP_SRC);
    let mut module = lower_file(&path);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    assert!(count_kind(main, true) > 0);
    let out = cli::run_files(path.to_str().unwrap(), "Main", vec![]);
    assert_eq!(out.output, vec!["escape success: 42".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_file(&path);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(&path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), 42);
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "escape success: 42\n");
    let _ = std::fs::remove_dir_all(path.parent().unwrap());
}
