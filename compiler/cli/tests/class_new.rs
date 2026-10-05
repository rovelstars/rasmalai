use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-classnew-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    assert!(c.iter().all(|x| x.code.is_warning()), "{c:?}");
    let mut out = lir::lower::lower(&m).unwrap_or_else(|e| panic!("{e}"));
    lir::opt::optimize_lir(&mut out, 1, "Main");
    let v = lir::verify::verify(&out);
    assert!(v.is_empty(), "{v:?}");
    (out, dir)
}

fn check_all_backends(src: &str, want: i64, want_out: &[String], tag: &str) {
    let (module, dir) = resolve_src(src, tag);
    let leaked: &'static lir::instr::Module = Box::leak(Box::new(module));
    let mut machine = runtime::machine::Machine::new(leaked);
    let r = machine.call("Main", vec![]).unwrap_or_else(|e| panic!("interpreter {tag}: {e:?}"));
    match r {
        runtime::value::Value::Int(v) if v == want => {}
        other => panic!("interpreter {tag}: {other:?}"),
    }
    let got = machine.output.clone();
    let want_out: Vec<String> = want_out.to_vec();
    assert_eq!(got, want_out, "interpreter {tag} stdout");

    let mut jit = cranelift::jit::Jit::compile(leaked).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(jit.call("Main", &[]).unwrap(), want, "cranelift {tag}");
    assert_eq!(llvm::codegen::execute(leaked, "Main").unwrap(), want, "llvm {tag}");

    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let bin = String::from_utf8_lossy(&build.stdout).lines().rev().find_map(|l| l.strip_prefix("artifact: ")).map(|s| s.trim().trim_end_matches(" (fresh)").to_string()).expect("build prints artifact path");
    let run = std::process::Command::new(&bin).output().unwrap();
    assert_eq!(run.status.code().unwrap(), want as i32, "aot {tag} exit");
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

fn check_fails_with(src: &str, code: &str) -> String {
    let rep = cli::check_source(src);
    assert!(!rep.errors.is_empty(), "expected {code} for:\n{src}");
    assert!(
        rep.errors.iter().any(|e| e.code.as_str() == code),
        "expected {code}, got {:?}",
        rep.errors.iter().map(|e| e.code.as_str()).collect::<Vec<_>>()
    );
    rep.errors
        .iter()
        .map(|e| {
            format!(
                "[{}] {}{}",
                e.code.as_str(),
                e.message,
                e.hint.as_ref().map(|h| format!(" hint: {h}")).unwrap_or_default()
            )
        })
        .collect::<Vec<_>>()
        .join("\n")
}

#[test]
fn new_basic_instantiation() {
    check_all_backends(
        "class Meter {\n\
        let v: Int;\n\
        init(v: Int) { this.v = v; }\n\
        get(): Int { return this.v * 2; }\n\
        }\n\
        fn Main(): Int {\n\
        let m = new Meter(21);\n\
        print(m.get());\n\
        return 0;\n\
        }\n",
        0,
        &["42".to_string()],
        "basic",
    );
}

#[test]
fn new_inheritance_runs_child_constructor() {
    check_all_backends(
        "class Base {\n\
        let x: Int;\n\
        init(x: Int) { this.x = x; }\n\
        }\n\
        class Child extends Base {\n\
        let y: Int;\n\
        init(x: Int, y: Int) { super(x); this.y = x + y; }\n\
        }\n\
        fn Main(): Int {\n\
        let c = new Child(20, 22);\n\
        print(c.x);\n\
        print(c.y);\n\
        return 0;\n\
        }\n",
        0,
        &["20".to_string(), "42".to_string()],
        "inherit",
    );
}

#[test]
fn child_init_without_super_is_e108() {
    let out = check_fails_with(
        "class Base {\n\
        let x: Int;\n\
        init(x: Int) { this.x = x; }\n\
        }\n\
        class Child extends Base {\n\
        let y: Int;\n\
        init(x: Int, y: Int) { this.y = x + y; }\n\
        }\n\
        fn Main(): Int {\n\
        let c = new Child(20, 22);\n\
        return 0;\n\
        }\n",
        "E108",
    );
    assert!(out.contains("super"), "{out}");
}

#[test]
fn new_default_field_construction() {
    check_all_backends(
        "class Pair {\n\
        let a: Int;\n\
        let b: Int;\n\
        }\n\
        fn Main(): Int {\n\
        let p = new Pair(20, 22);\n\
        print(p.a + p.b);\n\
        return 0;\n\
        }\n",
        0,
        &["42".to_string()],
        "fields",
    );
}

#[test]
fn direct_class_call_is_e204() {
    let out = check_fails_with(
        "class Meter {\n\
        let v: Int;\n\
        init(v: Int) { this.v = v; }\n\
        }\n\
        fn Main(): Int {\n\
        let m = Meter(20);\n\
        return 0;\n\
        }\n",
        "E204",
    );
    assert!(out.contains("without `new`"), "{out}");
    assert!(out.contains("new Meter"), "{out}");
}

#[test]
fn new_unknown_class_is_e108() {
    check_fails_with(
        "fn Main(): Int {\n\
        let m = new Nope(1);\n\
        return 0;\n\
        }\n",
        "E108",
    );
}

#[test]
fn new_struct_target_is_e108() {
    check_fails_with(
        "struct Box {\n\
        let v: Int;\n\
        }\n\
        fn Main(): Int {\n\
        let b = new Box(1);\n\
        return 0;\n\
        }\n",
        "E108",
    );
}

#[test]
fn new_interface_target_is_e108() {
    check_fails_with(
        "interface P {\n\
        fn ping(): Int;\n\
        }\n\
        fn Main(): Int {\n\
        let p = new P();\n\
        return 0;\n\
        }\n",
        "E108",
    );
}

#[test]
fn init_is_clean() {
    let rep = cli::check_source(
        "class Meter {\n\
        let v: Int;\n\
        init(v: Int) { this.v = v; }\n\
        }\n\
        fn Main(): Int {\n\
        let m = new Meter(20);\n\
        print(m.v);\n\
        return 0;\n\
        }\n",
    );
    assert!(rep.errors.is_empty(), "{:?}", rep.errors);
    assert!(rep.warnings.is_empty(), "{:?}", rep.warnings);
}

#[test]
fn constructor_is_rejected_with_init_hint() {
    let out = check_fails_with(
        "class Meter {\n\
        let v: Int;\n\
        constructor(v: Int) { this.v = v; }\n\
        }\n\
        fn Main(): Int {\n\
        return 0;\n\
        }\n",
        "E108",
    );
    assert!(out.contains("`init`"), "{out}");
}

#[test]
fn duplicate_init_is_error() {
    check_fails_with(
        "class Meter {\n\
        let v: Int;\n\
        init(v: Int) { this.v = v; }\n\
        init(v: Int) { this.v = v; }\n\
        }\n\
        fn Main(): Int {\n\
        return 0;\n\
        }\n",
        "E108",
    );
}
