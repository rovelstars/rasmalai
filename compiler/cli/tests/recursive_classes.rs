use std::path::PathBuf;

fn resolve_src(src: &str, tag: &str) -> (lir::instr::Module, PathBuf) {
    let dir = std::env::temp_dir().join(format!("rnx-recclass-{tag}-{}", std::process::id()));
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

fn check_diagnostic(src: &str, want_code: &str, want_msg: &str, tag: &str) {
    let dir = std::env::temp_dir().join(format!("rnx-recclass-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    let main = dir.join("main.rnx");
    std::fs::write(&main, src).unwrap();
    let g = frontend::modules::ModuleGraph::build(&main).unwrap_or_else(|e| panic!("{e}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let c = frontend::semantic::check(&m);
    assert!(
        c.iter().any(|x| x.code.as_str() == want_code && x.message.contains(want_msg)),
        "{c:?}"
    );
    let _ = std::fs::remove_dir_all(&dir);
}

const TREE: &str = "class TreeNode {\n\
    let value: Int;\n\
    let left: TreeNode?;\n\
    let right: TreeNode?;\n\
    fn count(): Int {\n\
        let total = 1;\n\
        if (this.left != null) {\n\
            total = total + this.left.count();\n\
        }\n\
        if (this.right != null) {\n\
            total = total + this.right.count();\n\
        }\n\
        return total;\n\
    }\n\
    }\n";

#[test]
fn recursive_tree_count_and_access() {
    let src = format!(
        "{TREE}\nfn Main(): Int {{\n\
        let root = new TreeNode();\n\
        root.value = 10;\n\
        let l = new TreeNode();\n\
        l.value = 5;\n\
        let r = new TreeNode();\n\
        r.value = 15;\n\
        root.left = l;\n\
        root.right = r;\n\
        assert(root.count() == 3, \"count\");\n\
        assert(root.left.value == 5, \"left\");\n\
        assert(root.right.value == 15, \"right\");\n\
        assert(root.left.left == null, \"leaf null\");\n\
        print(\"tree-ok\");\n\
        return 0;\n\
        }}\n"
    );
    check_all_backends(&src, 0, &["tree-ok".to_string()], "tree");
}

#[test]
fn mutually_recursive_link_and_break() {
    let src = "class A {\n\
        let b: B?;\n\
        }\n\
        class B {\n\
        let a: A?;\n\
        }\n\
        fn Main(): Int {\n\
        let a = new A();\n\
        let b = new B();\n\
        a.b = b;\n\
        b.a = a;\n\
        assert(a.b != null, \"linked\");\n\
        assert(b.a != null, \"back\");\n\
        b.a = null;\n\
        assert(b.a == null, \"broken\");\n\
        print(\"mutual-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["mutual-ok".to_string()], "mutual");
}

#[test]
fn field_overwrite_releases_old_child() {
    let src = format!(
        "{TREE}\nfn Main(): Int {{\n\
        let root = new TreeNode();\n\
        root.value = 1;\n\
        let old = new TreeNode();\n\
        old.value = 2;\n\
        root.left = old;\n\
        assert(root.count() == 2, \"before\");\n\
        let fresh = new TreeNode();\n\
        fresh.value = 3;\n\
        root.left = fresh;\n\
        assert(root.count() == 2, \"after\");\n\
        assert(root.left.value == 3, \"replaced\");\n\
        assert(old.value == 2, \"old alive\");\n\
        print(\"overwrite-ok\");\n\
        return 0;\n\
        }}\n"
    );
    check_all_backends(&src, 0, &["overwrite-ok".to_string()], "overwrite");
}

#[test]
fn deep_chain_build_and_drop() {
    let src = "class Node {\n\
        let value: Int;\n\
        let next: Node?;\n\
        }\n\
        fn Main(): Int {\n\
        let head: Node? = null;\n\
        let i = 0;\n\
        while (i < 5000) {\n\
        let nd = new Node();\n\
        nd.value = i;\n\
        nd.next = head;\n\
        head = nd;\n\
        i = i + 1;\n\
        }\n\
        let count = 0;\n\
        let cur = head;\n\
        while (cur != null) {\n\
        count = count + 1;\n\
        cur = cur.next;\n\
        }\n\
        assert(count == 5000, \"count\");\n\
        head = null;\n\
        cur = null;\n\
        print(\"drop-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["drop-ok".to_string()], "deep");
}

#[test]
fn cyclic_default_direct_rejected() {
    check_diagnostic(
        "class Node {\n    let next: Node = new Node();\n}\nfn Main(): Int {\n    return 0;\n}\n",
        "E108",
        "cyclic default construction `Node -> Node`",
        "cycdirect",
    );
}

#[test]
fn cyclic_default_mutual_rejected() {
    check_diagnostic(
        "class A {\n    let b: B = new B();\n}\nclass B {\n    let a: A = new A();\n}\nfn Main(): Int {\n    return 0;\n}\n",
        "E108",
        "cyclic default construction",
        "cycmutual",
    );
}

#[test]
fn conditional_construction_in_constructor_allowed() {
    let src = "class Node {\n\
        let value: Int;\n\
        let next: Node?;\n\
        init(v: Int, depth: Int) {\n\
        this.value = v;\n\
        if (depth > 0) {\n\
        this.next = new Node(v - 1, depth - 1);\n\
        }\n\
        }\n\
        fn sum(): Int {\n\
        if (this.next == null) { return this.value; }\n\
        return this.value + this.next.sum();\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let n = new Node(3, 3);\n\
        assert(n.sum() == 6, \"sum\");\n\
        print(\"ctor-ok\");\n\
        return 0;\n\
        }\n";
    check_all_backends(src, 0, &["ctor-ok".to_string()], "ctor");
}
