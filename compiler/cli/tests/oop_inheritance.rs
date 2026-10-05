use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-oop-{tag}-{}", std::process::id()));
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

fn check_fails_with(src: &str, code: &str, hint: &str) {
    let rep = cli::check_source(src);
    assert!(!rep.errors.is_empty(), "expected {code} for:\n{src}");
    assert!(
        rep.errors.iter().any(|e| e.code.as_str() == code),
        "expected {code}, got {:?}",
        rep.errors.iter().map(|e| e.code.as_str()).collect::<Vec<_>>()
    );
    if !hint.is_empty() {
        assert!(
            rep.errors.iter().any(|e| e.message.contains(hint)),
            "expected hint {hint:?}, got {:?}",
            rep.errors.iter().map(|e| e.message.clone()).collect::<Vec<_>>()
        );
    }
}

const ANIMAL_SRC: &str = "class Animal {\n\
    let name: String;\n\
    init(name: String) { this.name = name; }\n\
    fn speak(): String { return \"grunt\"; }\n\
    }\n\
    class Dog extends Animal {\n\
    let breed: String;\n\
    init(name: String, breed: String) { super(name); this.breed = breed; }\n\
    fn speak(): String { return \"woof\"; }\n\
    }\n\
    fn Main(): Int {\n\
    let d = new Dog(\"rex\", \"collie\");\n\
    print(d.name);\n\
    print(d.breed);\n\
    print(d.speak());\n\
    print(d is Dog);\n\
    print(d is Animal);\n\
    print(d is String);\n\
    return 0;\n\
    }\n";

#[test]
fn field_and_method_inheritance() {
    check_all_backends(
        ANIMAL_SRC,
        0,
        &[
            "rex".to_string(),
            "collie".to_string(),
            "woof".to_string(),
            "true".to_string(),
            "true".to_string(),
            "false".to_string(),
        ],
        "animal",
    );
}

#[test]
fn base_method_runs_on_child_layout() {
    check_all_backends(
        "class Base {\n\
        let x: Int;\n\
        init(x: Int) { this.x = x; }\n\
        fn get(): Int { return this.x * 2; }\n\
        }\n\
        class Child extends Base {\n\
        let y: Int = 100;\n\
        init(x: Int) { super(x); }\n\
        }\n\
        class GrandChild extends Child {\n\
        init(x: Int) { super(x); }\n\
        }\n\
        fn Main(): Int {\n\
        let c = new Child(21);\n\
        print(c.get());\n\
        print(c.y);\n\
        print(c.x);\n\
        let g = new GrandChild(5);\n\
        print(g.get());\n\
        print(g is Base);\n\
        print(g is Child);\n\
        print(g is GrandChild);\n\
        return 0;\n\
        }\n",
        0,
        &[
            "42".to_string(),
            "100".to_string(),
            "21".to_string(),
            "10".to_string(),
            "true".to_string(),
            "true".to_string(),
            "true".to_string(),
        ],
        "grandchild",
    );
}

#[test]
fn interface_adoption_inherited() {
    check_all_backends(
        "interface HasX {\n\
        fn get(): Int;\n\
        }\n\
        class Base with HasX {\n\
        let x: Int;\n\
        init(x: Int) { this.x = x; }\n\
        fn get(): Int { return this.x * 2; }\n\
        }\n\
        class Child extends Base {\n\
        init(x: Int) { super(x); }\n\
        }\n\
        fn useIt(h: HasX): Int {\n\
        return h.get();\n\
        }\n\
        fn Main(): Int {\n\
        let c = new Child(21);\n\
        print(useIt(c));\n\
        print(c is HasX);\n\
        return 0;\n\
        }\n",
        0,
        &["42".to_string(), "true".to_string()],
        "iface-inherit",
    );
}

#[test]
fn local_trait_composition() {
    check_all_backends(
        "trait Clickable {\n\
        fn onClick() { print(\"clicked\"); }\n\
        }\n\
        class Button with Clickable {\n\
        fn onClick() { print(\"pressed\"); }\n\
        }\n\
        trait Named {\n\
        let label: String = \"anon\";\n\
        fn describe(): String { return this.label; }\n\
        }\n\
        class Item with Named {\n\
        }\n\
        fn Main(): Int {\n\
        let b = new Button();\n\
        b.onClick();\n\
        let i = new Item();\n\
        print(i.describe());\n\
        print(i.label);\n\
        return 0;\n\
        }\n",
        0,
        &[
            "pressed".to_string(),
            "anon".to_string(),
            "anon".to_string(),
        ],
        "traits",
    );
}

#[test]
fn cyclic_inheritance_is_error() {
    check_fails_with(
        "class A extends B { }\nclass B extends A { }\nfn Main(): Int { return 0; }\n",
        "E108",
        "cyclic",
    );
}

#[test]
fn unknown_base_is_error() {
    check_fails_with(
        "class C extends Nope { }\nfn Main(): Int { return 0; }\n",
        "E108",
        "unknown base class",
    );
}

#[test]
fn field_redefinition_is_error() {
    check_fails_with(
        "class B {\n\
        let x: Int;\n\
        init(x: Int) { this.x = x; }\n\
        }\n\
        class C extends B {\n\
        let x: Int;\n\
        init(x: Int) { super(x); this.x = x; }\n\
        }\n\
        fn Main(): Int { return 0; }\n",
        "E108",
        "redefines inherited field",
    );
}

#[test]
fn override_signature_mismatch_is_error() {
    check_fails_with(
        "class B {\n\
        fn g(): Int { return 1; }\n\
        }\n\
        class C extends B {\n\
        fn g(): String { return \"s\"; }\n\
        }\n\
        fn Main(): Int { return 0; }\n",
        "E108",
        "does not match inherited",
    );
}

#[test]
fn private_base_field_denied_in_child() {
    check_fails_with(
        "class B {\n\
        private let s: Int;\n\
        init(v: Int) { this.s = v; }\n\
        }\n\
        class C extends B {\n\
        init(v: Int) { super(v); this.s = v; }\n\
        }\n\
        fn Main(): Int {\n\
        let c = new C(1);\n\
        return 0;\n\
        }\n",
        "E203",
        "private field",
    );
}
