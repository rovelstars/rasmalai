use frontend::ast::{Decl, Expr, Stmt};
use frontend::desugar::desugar;
use frontend::parser::Parser;
use frontend::semantic::check;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn codes(src: &str) -> Vec<String> {
    let mut m = parse(src);
    let mut out: Vec<String> = desugar(&mut m)
        .iter()
        .map(|d| d.code.as_str().to_string())
        .collect();
    out.extend(check(&m).iter().map(|d| d.code.as_str().to_string()));
    out
}

#[test]
fn new_parses_with_target_and_args() {
    let m = parse("class M { let v: Int; } fn F(): Int { let m = new M(1, 2); return 0; }");
    let body = match &m.decls[1].node {
        Decl::Fn(f) => match &f.body {
            frontend::ast::FnBody::Block(b) => b,
            _ => panic!("block"),
        },
        _ => panic!("fn"),
    };
    match &body.stmts[0].node {
        Stmt::Var { value, .. } => match &value.node {
            Expr::New { target, args, .. } => {
                assert_eq!(target, "M");
                assert_eq!(args.len(), 2);
            }
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

#[test]
fn new_parses_type_args_and_chains() {
    let m = parse("class M { let v: Int; } fn F(): Int { let m = new M<Int>(1).v; return 0; }");
    let body = match &m.decls[1].node {
        Decl::Fn(f) => match &f.body {
            frontend::ast::FnBody::Block(b) => b,
            _ => panic!("block"),
        },
        _ => panic!("fn"),
    };
    match &body.stmts[0].node {
        Stmt::Var { value, .. } => match &value.node {
            Expr::Member { base, field } => {
                assert_eq!(field, "v");
                assert!(matches!(&base.node, Expr::New { .. }));
            }
            other => panic!("{other:?}"),
        },
        other => panic!("{other:?}"),
    }
}

#[test]
fn constructor_is_rejected() {
    let err = Parser::parse_module("class M { let v: Int; constructor(v: Int) { this.v = v; } }")
        .expect_err("constructor must not parse");
    assert!(err.message.contains("`init`"), "{err:?}");
}

#[test]
fn direct_class_call_is_e204() {
    let got = codes("class M { let v: Int; } fn F(): Int { let m = M(1); return 0; }");
    assert!(got.iter().any(|c| c == "E204"), "{got:?}");
}

#[test]
fn init_is_clean() {
    let got = codes("class M { let v: Int; init(v: Int) { this.v = v; } }");
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn duplicate_init_is_error() {
    let got = codes(
        "class M { let v: Int; init(v: Int) { this.v = v; } init(v: Int) { this.v = v; } }",
    );
    assert!(got.iter().any(|c| c == "E108"), "{got:?}");
}

#[test]
fn new_without_parens_is_parse_error() {
    assert!(Parser::parse_module("class M { let v: Int; } fn F(): Int { let m = new M; return 0; }").is_err());
}
