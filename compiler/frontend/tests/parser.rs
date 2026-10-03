use diagnostics;
use frontend::ast;
use frontend::parser::Parser;

#[test]
fn rejects_c_style_for() {
    let e = Parser::parse_module("fn F() { for (let i = 0; i < 3; i += 1) { } }")
        .expect_err("must reject");
    assert_eq!(e.code.as_str(), "E110", "{e}");
}

#[test]
fn parses_for_in_forms() {
    for src in [
        "fn F() { for i in 0..10 { } }",
        "fn F() { for (k, v) in m.items() { } }",
        "fn F() { for i in (0..10).stride(2) { } }",
    ] {
        Parser::parse_module(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    }
}

#[test]
fn parses_switch_try_defer_guard() {
    let src = r#"
fn F(x: Int): String {
  defer { IO.print("out") }
  guard let y = Opt(x) else { return "none" }
  try {
    switch x {
      case 0..=10: return "low"
      case is Foo if x > 1: return "foo"
      default: return "other"
    }
  } catch (err) {
    throw err
  } finally {
    IO.print("done")
  }
}
"#;
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn parses_decay_closure_and_interp() {
    let src = r#"class B extends Component {
  let onClick: fn()
  init() { this.onClick = fn decay(this) { this.flush() } }
  fn flush() { IO.print("hi {this.count}") }
}"#;
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn parses_record_decl() {
    let m = Parser::parse_module("record Point(x: Float, y: Float)").unwrap();
    assert_eq!(m.decls.len(), 1);
}

#[test]
fn parses_generic_enum() {
    let m = Parser::parse_module("enum Option<T> { None, Some(T) }").unwrap();
    assert_eq!(m.decls.len(), 1);
}

#[test]
fn parses_unsafe_fn_and_block() {
    let src = "unsafe fn Raw(): Int { return 1 } fn F(): Int { unsafe { print(\"u\") } return Raw2() } unsafe fn Raw2(): Int { return 2 }";
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn parses_unsafe_method() {
    Parser::parse_module("class C { unsafe fn m(): Int { return 1 } }").unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn parses_unsafe_expr() {
    Parser::parse_module("fn F() { let x = unsafe { 1 } }").unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn parses_enum_pattern() {
    Parser::parse_module(
        "fn F(x: Int): Int { switch x { case .Some(v): return v case .None: return 0 } }",
    )
    .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn parses_tuple_for() {
    Parser::parse_module("fn F(p: Array): Int { for (a, b) in p { print(\"x\") } return 0 }")
        .unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn parses_access_modifiers() {
    let src = "public class A { public let x: Int; private let y: Int; let z: Int; public fn f(): Int { return 1 } }";
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
}

#[test]
fn default_access_needs_no_keyword() {
    use frontend::ast::{Access, ClassMember, Decl};
    let m = Parser::parse_module("class A { let x: Int; fn f(): Int { return 1 } }").unwrap();
    let members = match &m.decls[0].node {
        Decl::Class { members, .. } => members,
        _ => panic!("class"),
    };
    assert!(matches!(m.decls[0].node, Decl::Class { access: Access::Internal, .. }));
    for mem in members {
        match &mem.node {
            ClassMember::Field(f) => assert_eq!(f.access, Access::Internal),
            ClassMember::Method(f) => assert_eq!(f.access, Access::Internal),
            _ => {}
        }
    }
}

#[test]
fn async_fn_and_await_parse() {
    let m = Parser::parse_module("async fn f(x: Int): Int { return await g(x); }").unwrap();
    assert_eq!(m.decls.len(), 1);
}

#[test]
fn await_outside_async_is_e109() {
    let mut m = Parser::parse_module("fn f(): Int { return await g(); }").unwrap();
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.iter().any(|x| x.code == diagnostics::Code::E109));
}

fn closure_of(src: &str) -> ast::Expr {
    let m = Parser::parse_module(src).unwrap_or_else(|e| panic!("{src}: {e}"));
    let f = m
        .decls
        .iter()
        .find_map(|d| match &d.node {
            ast::Decl::Fn(f) => Some(f),
            _ => None,
        })
        .expect("fn decl");
    let b = match &f.body {
        ast::FnBody::Block(b) => b,
        _ => panic!("block body"),
    };
    match &b.stmts[0].node {
        ast::Stmt::Var { value, .. } => match &value.node {
            e @ ast::Expr::Closure { .. } => e.clone(),
            other => panic!("expected closure, got {other:?}"),
        },
        other => panic!("expected let, got {other:?}"),
    }
}

#[test]
fn arrow_paren_untyped_desugars_to_closure() {
    match closure_of("fn F() { let f = (x) => x * 2; }") {
        ast::Expr::Closure {
            decay,
            params,
            ret,
            body,
            ..
        } => {
            assert!(!decay);
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "x");
            assert!(params[0].ty.is_none());
            assert!(ret.is_none());
            assert!(matches!(body, ast::FnBody::Expr(_)));
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn arrow_single_ident_desugars_to_closure() {
    match closure_of("fn F() { let f = x => x + 1; }") {
        ast::Expr::Closure { params, .. } => {
            assert_eq!(params.len(), 1);
            assert_eq!(params[0].name, "x");
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn arrow_typed_with_return_type() {
    match closure_of("fn F() { let f = (a: Integer, b: Integer): Integer => a + b; }") {
        ast::Expr::Closure { params, ret, .. } => {
            assert_eq!(params.len(), 2);
            assert!(params.iter().all(|p| p.ty.is_some()));
            assert!(ret.is_some());
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn arrow_empty_params_block_body() {
    match closure_of("fn F() { let f = () => { return 42; }; }") {
        ast::Expr::Closure { params, body, .. } => {
            assert!(params.is_empty());
            assert!(matches!(body, ast::FnBody::Block(_)));
        }
        other => panic!("got {other:?}"),
    }
}

#[test]
fn rejects_fn_paren_arrow_with_e105() {
    let e =
        Parser::parse_module("fn F() { let f = fn(x) => x * 2; }").expect_err("must reject");
    assert_eq!(e.code, diagnostics::Code::E105, "{e}");
}

#[test]
fn rejects_named_compact_body_with_e105() {
    let e =
        Parser::parse_module("fn foo(x: Integer): Integer => x * 2;").expect_err("must reject");
    assert_eq!(e.code, diagnostics::Code::E105, "{e}");
}

#[test]
fn rejects_decay_expr_body_with_e105() {
    let e =
        Parser::parse_module("class B { let cb: fn() init() { this.cb = fn decay(this) => 1 } }")
            .expect_err("must reject");
    assert_eq!(e.code, diagnostics::Code::E105, "{e}");
}

fn parse_all_errs(src: &str) -> Vec<diagnostics::Code> {
    match Parser::parse_module_all(src) {
        Ok(_) => panic!("expected errors in {src}"),
        Err(errs) => errs.iter().map(|e| e.code).collect(),
    }
}

#[test]
fn recovers_across_bad_decls() {
    let errs = parse_all_errs("fn a(): Int => 1;\nfn b(): Int => 2;\n");
    assert_eq!(errs, vec![diagnostics::Code::E105, diagnostics::Code::E105]);
}

#[test]
fn recovery_points_at_each_site() {
    let src = "fn a(): Int => 1;\nfn Good(): Int { return 1; }\nfn c(): Int => 3;\n";
    match Parser::parse_module_all(src) {
        Ok(_) => panic!("expected errors"),
        Err(errs) => {
            assert_eq!(errs.len(), 2);
            let first = src[errs[0].span.unwrap().start as usize..].starts_with("=>");
            let second = src[errs[1].span.unwrap().start as usize..].starts_with("=>");
            assert!(first && second, "{errs:?}");
            assert!(errs[1].span.unwrap().start > errs[0].span.unwrap().start);
        }
    }
}

#[test]
fn recovers_inside_blocks() {
    let errs = parse_all_errs("fn F() { let x = ; let y = ; return 1; }\n");
    assert_eq!(errs.len(), 2, "{errs:?}");
}

#[test]
fn recovers_inside_class() {
    let errs = parse_all_errs("class A { fn a(): Int => 1; fn b(): Int => 2; fn ok(): Int { return 0; } }\n");
    assert_eq!(errs.len(), 2, "{errs:?}");
}

#[test]
fn unterminated_block_still_errors() {
    let errs = parse_all_errs("fn F() { let x = 1;\n");
    assert!(!errs.is_empty());
}

fn parse_all_diags(src: &str) -> Vec<diagnostics::Diagnostic> {
    match Parser::parse_module_all(src) {
        Ok(_) => panic!("expected errors in {src}"),
        Err(errs) => errs,
    }
}

fn assert_e206_without_decl_mask(diags: &[diagnostics::Diagnostic]) {
    assert!(
        diags.iter().any(|e| e.code == diagnostics::Code::E206),
        "{diags:?}"
    );
    assert!(
        !diags.iter().any(|e| e.message.contains("expected declaration")),
        "{diags:?}"
    );
}

#[test]
fn uninit_let_parses_clean() {
    assert!(Parser::parse_module_all("let a;\n").is_ok());
}

#[test]
fn uninit_typed_let_parses_clean() {
    assert!(Parser::parse_module_all("let a: Int;\n").is_ok());
}

#[test]
fn uninit_const_reports_e206() {
    let diags = parse_all_diags("const b;\n");
    assert_e206_without_decl_mask(&diags);
}

#[test]
fn uninit_typed_const_reports_e206() {
    let diags = parse_all_diags("const b: Int;\n");
    assert_e206_without_decl_mask(&diags);
}

#[test]
fn uninit_let_in_block_parses_clean() {
    assert!(Parser::parse_module_all("fn Main(): Int { let a: Int; return 0; }\n").is_ok());
}

#[test]
fn uninit_let_parses_following_line() {
    let src = "let a;\nprint(a);\n";
    assert!(Parser::parse_module_all(src).is_ok());
}

#[test]
fn uninit_let_sync_reaches_second_broken_decl() {
    let diags = parse_all_diags("let a;\nlet b =;\n");
    assert_eq!(diags.len(), 1, "{diags:?}");
    assert_eq!(diags[0].code, diagnostics::Code::E108);
}
