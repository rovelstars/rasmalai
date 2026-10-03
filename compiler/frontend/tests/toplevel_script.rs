use frontend::ast::Decl;
use frontend::desugar::desugar;
use frontend::modules::{ModuleFile, ModuleGraph, ModuleKind};
use frontend::parser::Parser;
use std::path::PathBuf;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn has_async_main(m: &frontend::ast::Module) -> bool {
    m.decls.iter().any(|d| match &d.node {
        Decl::Fn(f) => f.name == "Main" || f.name == "main" || f.name == "__Main_async",
        _ => false,
    })
}

#[test]
fn top_level_statements_parse_as_decls() {
    let m = parse("let a = 10;\nprint(a);\n");
    assert_eq!(m.decls.len(), 2);
    assert!(m.decls.iter().all(|d| matches!(&d.node, Decl::Stmt(_))));
}

#[test]
fn top_level_await_parses() {
    let m = parse("let data = await Promise.resolve(42);\nprint(data);\n");
    assert_eq!(m.decls.len(), 2);
}

#[test]
fn desugar_synthesizes_sync_main_without_await() {
    let mut m = parse("let a = 10;\nlet b = 20;\nprint(a + b);\n");
    let diags = desugar(&mut m);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(has_async_main(&m));
    assert!(!m.decls.iter().any(|d| matches!(&d.node, Decl::Stmt(_))));
    assert!(m.decls.iter().any(|d| matches!(&d.node, Decl::Fn(f) if f.name == "Main" && !f.is_async)));
    assert!(!m.decls.iter().any(|d| matches!(&d.node, Decl::Fn(f) if f.name == "__Main_async")));
}

#[test]
fn desugar_top_await_has_no_e109() {
    let mut m = parse("let data = await Promise.resolve(42);\nprint(data);\n");
    let diags = desugar(&mut m);
    assert!(!diags.iter().any(|d| d.code.as_str() == "E109"), "{diags:?}");
    assert!(has_async_main(&m));
}

#[test]
fn desugar_keeps_top_return_and_appends_default() {
    let mut m = parse("print(1);\nreturn 7;\n");
    let diags = desugar(&mut m);
    assert!(diags.is_empty(), "{diags:?}");
    let f = m.decls.iter().find_map(|d| match &d.node {
        Decl::Fn(f) if f.name == "Main" => Some(f),
        _ => None,
    }).expect("synthesized sync Main");
    assert!(!f.is_async);
    let body = match &f.body {
        frontend::ast::FnBody::Block(b) => b,
        _ => panic!("block"),
    };
    assert!(matches!(body.stmts.last().map(|s| &s.node), Some(frontend::ast::Stmt::Return(_))));
}

#[test]
fn desugar_top_await_synthesizes_async_main() {
    let mut m = parse("let data = await Promise.resolve(42);\nprint(data);\n");
    let diags = desugar(&mut m);
    assert!(!diags.iter().any(|d| d.code.as_str() == "E109"), "{diags:?}");
    let f = m.decls.iter().find_map(|d| match &d.node {
        Decl::Fn(f) if f.name == "__Main_async" => Some(f),
        _ => None,
    }).expect("expanded __Main_async for await script");
    let body = match &f.body {
        frontend::ast::FnBody::Block(b) => b,
        _ => panic!("block"),
    };
    assert!(matches!(body.stmts.last().map(|s| &s.node), Some(frontend::ast::Stmt::Return(_))));
}

#[test]
fn desugar_sync_closure_await_stays_sync_main() {
    // Await quarantined inside a sync closure is that closure's own error;
    // it must not drag the top-level script into the async path.
    let mut m = parse("let f = () => 1;\nprint(f());\n");
    let diags = desugar(&mut m);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(m.decls.iter().any(|d| matches!(&d.node, Decl::Fn(f) if f.name == "Main" && !f.is_async)));
    assert!(!m.decls.iter().any(|d| matches!(&d.node, Decl::Fn(f) if f.name == "__Main_async")));
}

#[test]
fn desugar_no_main_no_top_leaves_module_alone() {
    let mut m = parse("fn helper(): Int {\n    return 1;\n}\n");
    let diags = desugar(&mut m);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(!has_async_main(&m));
}

#[test]
fn explicit_main_plus_top_level_is_e108() {
    let mut m = parse("print(1);\nfn Main(): Int {\n    return 0;\n}\n");
    let diags = desugar(&mut m);
    assert!(diags.iter().any(|d| d.code.as_str() == "E108"), "{diags:?}");
}

#[test]
fn imported_imperative_statement_is_e112() {
    let root = PathBuf::from("/tmp/rnx-tl-e112-main.rnx");
    let helper = PathBuf::from("/tmp/rnx-tl-e112-helpers.rnx");
    let g = ModuleGraph {
        root: root.clone(),
        files: vec![
            ModuleFile {
                path: root,
                key: String::new(),
                kind: ModuleKind::Entry,
                module: parse("import { helper } from \"./helpers\";\nfn Main(): Int {\n    return helper();\n}\n"),
            },
            ModuleFile {
                path: helper,
                key: "helpers".to_string(),
                kind: ModuleKind::Imported,
                module: parse("fn helper(): Int {\n    return 1;\n}\nprint(\"leak\");\n"),
            },
        ],
    };
    let err = g.resolve().expect_err("expected E112");
    assert_eq!(err.code.as_str(), "E112");
}

#[test]
fn imported_top_await_is_e112_not_e109() {
    let root = PathBuf::from("/tmp/rnx-tl-e112a-main.rnx");
    let helper = PathBuf::from("/tmp/rnx-tl-e112a-helpers.rnx");
    let g = ModuleGraph {
        root: root.clone(),
        files: vec![
            ModuleFile {
                path: root,
                key: String::new(),
                kind: ModuleKind::Entry,
                module: parse("import { get } from \"./helpers\";\nfn Main(): Int {\n    return get();\n}\n"),
            },
            ModuleFile {
                path: helper,
                key: "helpers".to_string(),
                kind: ModuleKind::Imported,
                module: parse("fn get(): Int {\n    return 1;\n}\nlet d = await Promise.resolve(1);\n"),
            },
        ],
    };
    let err = g.resolve().expect_err("expected E112");
    assert_eq!(err.code.as_str(), "E112");
}

#[test]
fn imported_let_binding_is_e112() {
    let root = PathBuf::from("/tmp/rnx-tl-e112b-main.rnx");
    let helper = PathBuf::from("/tmp/rnx-tl-e112b-helpers.rnx");
    let g = ModuleGraph {
        root: root.clone(),
        files: vec![
            ModuleFile {
                path: root,
                key: String::new(),
                kind: ModuleKind::Entry,
                module: parse("fn Main(): Int {\n    return 0;\n}\n"),
            },
            ModuleFile {
                path: helper,
                key: "helpers".to_string(),
                kind: ModuleKind::Imported,
                module: parse("let x = 1 + 2;\n"),
            },
        ],
    };
    let err = g.resolve().expect_err("expected E112");
    assert_eq!(err.code.as_str(), "E112");
}

fn write_tree(tag: &str, files: &[(&str, &str)]) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-tl-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    for (name, src) in files {
        std::fs::write(dir.join(name), src).unwrap();
    }
    dir
}

#[test]
fn valid_declarative_import_resolves() {
    let dir = write_tree(
        "decl",
        &[
            ("helpers.rnx", "class Foo {\n    let x: Int;\n    init(x: Int) {\n        this.x = x;\n    }\n}\n\nfn bar(): Int {\n    return 41;\n}\n\nconst MAX = 100;\n"),
            ("main.rnx", "import { bar } from \"./helpers\";\n\nfn Main(): Int {\n    print(bar());\n    return 0;\n}\n"),
        ],
    );
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e:?}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let diags = desugar(&mut m);
    assert!(!diags.iter().any(|d| d.code.as_str() == "E112"), "{diags:?}");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn entry_top_level_survives_build() {
    let dir = write_tree("entry", &[("main.rnx", "let a = 10;\nprint(a);\n")]);
    let g = ModuleGraph::build(&dir.join("main.rnx")).unwrap_or_else(|e| panic!("{e:?}"));
    let mut m = g.resolve().unwrap_or_else(|e| panic!("{e}"));
    let diags = desugar(&mut m);
    assert!(diags.is_empty(), "{diags:?}");
    assert!(has_async_main(&m));
    let _ = std::fs::remove_dir_all(&dir);
}

fn merged(src: &str) -> frontend::ast::Module {
    ModuleGraph::from_source(src).unwrap_or_else(|e| panic!("{e}"))
}

fn contains_rnx_host_new(m: &frontend::ast::Module) -> bool {
    fn expr(e: &frontend::ast::Expr) -> bool {
        match e {
            frontend::ast::Expr::New { target, .. } => target == "std.prelude.RnxHost",
            _ => false,
        }
    }
    fn ex(e: &frontend::ast::Spanned<frontend::ast::Expr>) -> bool {
        expr(&e.node)
            || match &e.node {
                frontend::ast::Expr::Member { base, .. } => ex(base),
                frontend::ast::Expr::Call { callee, args, .. } => {
                    ex(callee) || args.iter().any(|a| ex(&a.value))
                }
                _ => false,
            }
    }
    m.decls.iter().any(|d| match &d.node {
        Decl::Fn(f) => match &f.body {
            frontend::ast::FnBody::Block(b) => b.stmts.iter().any(|s| match &s.node {
                frontend::ast::Stmt::Expr(e) => ex(e),
                _ => false,
            }),
            _ => false,
        },
        Decl::Stmt(s) => matches!(&s.node, frontend::ast::Stmt::Expr(e) if ex(e)),
        _ => false,
    })
}

#[test]
fn ambient_rnx_rewrites_to_host_new() {
    let m = merged("fn Main(): Int {\n    print(rnx.version);\n    return 0;\n}\n");
    assert!(contains_rnx_host_new(&m));
}

#[test]
fn shadowed_rnx_keeps_user_binding() {
    let m = merged("fn Main(): Int {\n    let rnx = 5;\n    print(rnx);\n    return 0;\n}\n");
    assert!(!contains_rnx_host_new(&m));
}
