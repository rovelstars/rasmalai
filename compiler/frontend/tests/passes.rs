use frontend::desugar::desugar;
use frontend::parser::Parser;
use frontend::semantic::check;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

#[test]
fn finally_becomes_defer() {
    let mut m = parse("fn F() { try { IO.print(\"a\") } finally { IO.print(\"b\") } }");
    let diags = desugar(&mut m);
    assert!(diags.is_empty());
    let f = m.decls.iter().find_map(|d| match &d.node {
        frontend::ast::Decl::Fn(f) => Some(f),
        _ => None,
    }).expect("fn");
    let body = match &f.body {
        frontend::ast::FnBody::Block(b) => b,
        _ => panic!("block"),
    };
    assert_eq!(body.stmts.len(), 2);
    assert!(matches!(
        body.stmts[0].node,
        frontend::ast::Stmt::Defer(_)
    ));
    assert!(matches!(
        body.stmts[1].node,
        frontend::ast::Stmt::Try { .. }
    ));
    if let frontend::ast::Stmt::Try { finally, .. } = &body.stmts[1].node {
        assert!(finally.is_none());
    }
}

#[test]
fn unterminated_case_errors() {
    let mut m = parse("fn F(x: Int) { switch x { case 1: IO.print(\"a\") case 2: return } }");
    let diags = desugar(&mut m);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code.as_str(), "E108");
}

#[test]
fn terminated_cases_pass() {
    let mut m = parse(
        "fn F(x: Int) { switch x { case 1: IO.print(\"a\") fallthrough; case 2: return default: return } }",
    );
    assert!(desugar(&mut m).is_empty());
}

#[test]
fn w104_fires_without_decay() {
    let m = parse(
        "class B { let cb: fn() init() { this.cb = () => this.flush() } fn flush() { } }",
    );
    let diags = check(&m);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code.as_str(), "W104");
}

#[test]
fn w104_silent_with_decay() {
    let m = parse(
        "class B { let cb: fn() init() { this.cb = fn decay(this) { this.flush() } } fn flush() { } }",
    );
    let diags = check(&m);
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn w109_fires_when_never_cleared() {
    let m = parse(
        "class N { let next: N? #[Allow(CyclicReference)] let back: N? deinit { } }",
    );
    let diags = check(&m);
    assert!(diags.iter().any(|d| d.code.as_str() == "W109"), "{diags:?}");
}

#[test]
fn w109_silent_when_cleared() {
    let m = parse(
        "class N { let next: N? #[Allow(CyclicReference)] let back: N? fn clearLinks() { this.back = null } deinit { } }",
    );
    assert!(
        !check(&m).iter().any(|d| d.code.as_str() == "W109"),
        "{:?}",
        check(&m)
    );
}

#[test]
fn w108_fires_on_mutual_strong() {
    let m = parse("class A { let b: B } class B { let a: A }");
    let diags = check(&m);
    assert_eq!(diags.len(), 1);
    assert_eq!(diags[0].code.as_str(), "W108");
}

#[test]
fn w108_silent_with_genref() {
    let m = parse("class A { let b: B } class B { let a: GenRef<A> }");
    assert!(check(&m).is_empty());
}

#[test]
fn async_expands_to_promise_fn() {
    let mut m = Parser::parse_module("async fn compute(x: Int): Int { let b = await sub(x); return b; }").unwrap();
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let mut saw_plain = false;
    for decl in &m.decls {
        match &decl.node {
            frontend::ast::Decl::Fn(f) if f.name == "compute" && !f.is_async => saw_plain = true,
            frontend::ast::Decl::Fn(f) if f.is_async => panic!("async fn should be rewritten"),
            frontend::ast::Decl::Enum { name, .. } if name == "Poll" => panic!("Poll must not be synthesized"),
            _ => {}
        }
    }
    assert!(saw_plain);
    let f = m.decls.iter().find_map(|d| match &d.node {
        frontend::ast::Decl::Fn(f) if f.name == "compute" => Some(f),
        _ => None,
    }).unwrap();
    let ret = f.ret.as_ref().unwrap();
    assert_eq!(ret.path, vec!["std.prelude.Promise".to_string()]);
    assert_eq!(ret.args.len(), 1);
}

#[test]
fn async_plain_fn_returns_promise() {
    let mut m = Parser::parse_module("async fn compute(): Int { return 1; }").unwrap();
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let f = m.decls.iter().find_map(|d| match &d.node {
        frontend::ast::Decl::Fn(f) if f.name == "compute" => Some(f),
        _ => None,
    }).unwrap();
    assert_eq!(f.ret.as_ref().unwrap().path, vec!["std.prelude.Promise".to_string()]);
}

#[test]
fn no_yield_synthesis_with_async() {
    let mut m = parse("async fn f(): Int { return 1; }");
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    assert!(!m.decls.iter().any(|d| matches!(&d.node, frontend::ast::Decl::Class { name, .. } if name == "YieldTask")));
    assert!(!m.decls.iter().any(|d| matches!(&d.node, frontend::ast::Decl::Fn(f) if f.name == "yieldNow")));
}

#[test]
fn no_yield_synthesis_without_async() {
    let mut m = parse("fn f(): Int { return 1; }");
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    assert!(!m.decls.iter().any(|d| matches!(&d.node, frontend::ast::Decl::Class { name, .. } if name == "YieldTask")));
}

#[test]
fn async_main_gets_wait_driver() {
    let mut m = parse("async fn Main(): Int { return 1; }");
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let main = m.decls.iter().find_map(|d| match &d.node {
        frontend::ast::Decl::Fn(f) if f.name == "Main" => Some(f),
        _ => None,
    }).unwrap();
    assert!(!main.is_async);
    assert_eq!(main.ret.as_ref().unwrap().path, vec!["std.prelude.Int".to_string()]);
    let inner = m.decls.iter().find_map(|d| match &d.node {
        frontend::ast::Decl::Fn(f) if f.name == "__Main_async" => Some(f),
        _ => None,
    }).unwrap();
    assert!(!inner.is_async);
    assert_eq!(inner.ret.as_ref().unwrap().path, vec!["std.prelude.Promise".to_string()]);
}

#[test]
fn sync_main_untouched() {
    let mut m = parse("fn Main(): Int { return 1; }");
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    assert!(!m.decls.iter().any(|d| matches!(&d.node, frontend::ast::Decl::Fn(f) if f.name == "__Main_async")));
}

#[test]
fn await_inside_if_allowed() {
    let mut m = parse("async fn f(c: Bool): Int { if c { await g(); } return 1; }");
    let d = frontend::desugar::desugar(&mut m);
    assert!(!d.iter().any(|x| x.code == diagnostics::Code::E108), "{d:?}");
}

#[test]
fn async_main_without_return_fulfills_zero() {
    let mut m = parse("async fn main(){ print(\"hello\") }");
    assert!(frontend::desugar::desugar(&mut m).is_empty());
    let inner = m.decls.iter().find_map(|d| match &d.node {
        frontend::ast::Decl::Fn(f) if f.name == "__main_async" => Some(f),
        _ => None,
    }).expect("inner async fn");
    assert!(!inner.is_async);
    assert_eq!(inner.ret.as_ref().unwrap().path, vec!["std.prelude.Promise".to_string()]);
    let body = match &inner.body {
        frontend::ast::FnBody::Block(b) => b,
        _ => panic!("block"),
    };
    assert_eq!(body.stmts.len(), 3);
}
