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

fn arc_count(func: &lir::instr::Function) -> usize {
    func.blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .filter(|i| matches!(i, Instr::Retain { .. } | Instr::Release { .. }))
        .count()
}

const BOX_SRC: &str = "class Box {\n    let val: Int;\n    init(v: Int) { this.val = v; }\n    get(): Int { return this.val; }\n}\nfn readBox(b: Box): Int {\n    return b.get();\n}\nfn Main(): Int {\n    let b = new Box(42);\n    let res = readBox(b);\n    return res;\n}\n";

#[test]
fn test_inlined_argument_retain_release_stripped() {
    let mut module = lower_src(BOX_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let main = fn_by_name(&module, "Main");
    assert_eq!(arc_count(main), 0);
    let out = cli::run_source(BOX_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

const MAKE_SRC: &str = "class Box {\n    let v: Int;\n    init(v: Int) { this.v = v; }\n}\nfn make(): Box {\n    return new Box(7);\n}\nfn Main(): Int {\n    let b = make();\n    let r = b.v * 6;\n    return r;\n}\n";

#[test]
fn test_escaping_reference_balanced_arc() {
    let mut module = lower_src(MAKE_SRC);
    lir::opt::optimize_lir(&mut module, 1, "Main");
    let make = fn_by_name(&module, "make");
    assert!(make
        .blocks
        .iter()
        .flat_map(|b| b.instrs.iter())
        .any(|i| matches!(i, Instr::ObjNew { .. })));
    assert_eq!(arc_count(make), 0);
    let out = cli::run_source(MAKE_SRC, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
}

const LOOP_SRC: &str = "class Box {\n    let v: Int;\n    init(v: Int) { this.v = v; }\n}\nfn get2(b: Box): Int {\n    return b.v;\n}\nfn Main(): Int {\n    let b = new Box(6);\n    let s = 0;\n    let i = 0;\n    while i < 7 {\n        s = s + get2(b);\n        i = i + 1;\n    }\n    print(\"arc success:\", s);\n    return s;\n}\n";

#[test]
fn test_cross_backend_arc_opt_execution() {
    let out = cli::run_source(LOOP_SRC, "Main", vec![]);
    assert_eq!(out.output, vec!["arc success: 42".to_string()]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(42)) => {}
        other => panic!("{other:?}"),
    }
    let mut for_jit = lower_src(LOOP_SRC);
    lir::opt::optimize_lir(&mut for_jit, 1, "Main");
    let mut jit = cranelift::jit::Jit::compile(&for_jit).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), 42);
    assert_eq!(llvm::codegen::execute(&for_jit, "Main").unwrap(), 42);
    let dir = std::env::temp_dir().join(format!("rnx-arc-bin-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("arc.rnx");
    std::fs::write(&src_path, LOOP_SRC).unwrap();
    let bin_path = dir.join("arc_bin");
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
    assert_eq!(String::from_utf8(run.stdout).unwrap(), "arc success: 42\n");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_opt_0_preserves_arc_instructions() {
    let src = "class B {\n    let v: Int;\n    init(v: Int) { this.v = v; }\n}\nfn Main(): Int {\n    let a = [new B(1)];\n    a[0] = new B(2);\n    return a[0].v;\n}\n";
    let fresh = lower_src(src);
    assert!(arc_count(fn_by_name(&fresh, "Main")) > 0);
    let mut level_zero = lower_src(src);
    lir::opt::optimize_lir(&mut level_zero, 0, "Main");
    assert_eq!(
        format!("{:?}", level_zero.functions),
        format!("{:?}", fresh.functions)
    );
    let dir = std::env::temp_dir().join(format!("rnx-arc-zero-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let src_path = dir.join("main.rnx");
    std::fs::write(&src_path, src).unwrap();
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let run = std::process::Command::new(rnx)
        .arg("run")
        .arg("-O")
        .arg("0")
        .arg(&src_path)
        .output()
        .unwrap();
    assert_eq!(run.status.code().unwrap(), 2);
    let out = cli::run_source(src, "Main", vec![]);
    match out.outcome {
        cli::RunOutcome::Value(runtime::value::Value::Int(2)) => {}
        other => panic!("{other:?}"),
    }
    let _ = std::fs::remove_dir_all(&dir);
}
