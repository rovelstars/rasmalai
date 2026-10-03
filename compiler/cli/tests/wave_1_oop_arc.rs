use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-wave1-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
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

    let bin_path = dir.join("wave1_bin");
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let build = std::process::Command::new(rnx)
        .arg("build")
        .arg(dir.join("main.rnx"))
        .arg("-o")
        .arg(&bin_path)
        .output()
        .unwrap();
    assert!(build.status.success(), "{}", String::from_utf8_lossy(&build.stderr));
    let run = std::process::Command::new(&bin_path).output().unwrap();
    match run.status.code() {
        Some(code) => assert_eq!(code, want as i32, "aot {tag} exit"),
        None => panic!("aot {tag} killed by signal: {run:?}"),
    }
    let suffix = if want_out.is_empty() { "" } else { "\n" };
    assert_eq!(
        String::from_utf8(run.stdout).unwrap(),
        want_out.join("\n") + suffix,
        "aot {tag} stdout"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const SELF_ASSIGN_SRC: &str = "fn id(x: Array<Int>): Array<Int> { return x; }\nfn add(notes: Array<Int>, body: Int): Array<Int> {\n    notes.push(body);\n    return notes;\n}\nfn Main(): Int {\n    let a: Array<Int> = [1];\n    a = id(a);\n    if a.len() != 1 {\n        return 1;\n    }\n    a = add(a, 5);\n    if a.len() != 2 {\n        return 2;\n    }\n    print(a.len());\n    return 0;\n}\n";

#[test]
fn test_self_reassignment_survives() {
    check_all_backends(SELF_ASSIGN_SRC, 0, &["2".to_string()], "selfassign");
}

const CHURN_SRC: &str = "fn bump(xs: Array<Int>): Array<Int> {\n    xs.push(1);\n    return xs;\n}\nfn Main(): Int {\n    let a: Array<Int> = [];\n    let i = 0;\n    while i < 5000 {\n        a = bump(a);\n        i = i + 1;\n    }\n    if a.len() != 5000 {\n        return 1;\n    }\n    print(a.len());\n    return 0;\n}\n";

#[test]
fn test_reassignment_churn_5000() {
    check_all_backends(CHURN_SRC, 0, &["5000".to_string()], "churn");
}

const SUPER_SRC: &str = "class Entity {\n    let id: Int;\n    init(id: Int) {\n        if id <= 0 { throw 0; }\n        this.id = id;\n    }\n    fn tag(): String { return \"E\" + this.id; }\n}\nclass Order extends Entity {\n    let total: Int;\n    init(id: Int, total: Int) {\n        super(id);\n        this.total = total;\n    }\n    fn tag(): String { return \"O\" + this.id; }\n}\nclass Note extends Entity {\n    let body: String;\n    init(id: Int, body: String) {\n        super(id);\n        this.body = body;\n    }\n}\nfn Main(): Int {\n    try {\n        let bad = new Order(0, 10);\n        print(\"BAD\");\n    } catch (e) {\n        print(\"order threw\");\n    }\n    try {\n        let n = new Note(-5, \"hi\");\n        print(\"BAD\");\n    } catch (e) {\n        print(\"note threw\");\n    }\n    let ok = new Order(3, 40);\n    print(ok.id);\n    print(ok.total);\n    print(ok.tag());\n    return 0;\n}\n";

#[test]
fn test_super_chains_parent_validation() {
    check_all_backends(
        SUPER_SRC,
        0,
        &[
            "order threw".to_string(),
            "note threw".to_string(),
            "3".to_string(),
            "40".to_string(),
            "O3".to_string(),
        ],
        "super",
    );
}

const VIRTUAL_SRC: &str = "class Base {\n    let id: Int;\n    init(id: Int) { this.id = id; }\n    fn kind(): String { return \"base\"; }\n    fn viaBase(): String { return this.kind(); }\n}\nclass Child extends Base {\n    init(id: Int) { super(id); }\n    fn kind(): String { return \"child\"; }\n}\nfn Main(): Int {\n    let c = new Child(1);\n    print(c.kind());\n    print(c.viaBase());\n    return 0;\n}\n";

#[test]
fn test_inherited_self_call_reaches_override() {
    check_all_backends(VIRTUAL_SRC, 0, &["child".to_string(), "child".to_string()], "virtual");
}

const IFACE_SRC: &str = "interface Kinded {\n    fn kind(): String;\n}\nclass Base {\n    let id: Int;\n    init(id: Int) { this.id = id; }\n    fn kind(): String { return \"base\"; }\n}\nclass Child extends Base : Kinded {\n    init(id: Int) { super(id); }\n}\nfn Main(): Int {\n    let c: Kinded = new Child(1);\n    print(c.kind());\n    return 0;\n}\n";

#[test]
fn test_inherited_method_satisfies_interface() {
    check_all_backends(IFACE_SRC, 0, &["base".to_string()], "iface");
}

const SUPER_METHOD_SRC: &str = "class Base {\n    let id: Int;\n    init(id: Int) { this.id = id; }\n    fn kind(): String { return \"base\"; }\n}\nclass Child extends Base {\n    init(id: Int) { super(id); }\n    fn kind(): String { return \"child-\" + super.kind(); }\n}\nfn Main(): Int {\n    let c = new Child(1);\n    print(c.kind());\n    return 0;\n}\n";

#[test]
fn test_super_method_reaches_parent() {
    check_all_backends(SUPER_METHOD_SRC, 0, &["child-base".to_string()], "supermethod");
}
