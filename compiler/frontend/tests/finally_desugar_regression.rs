use frontend::ast::*;
use frontend::desugar::desugar;
use frontend::parser::Parser;

fn parse(src: &str) -> Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn fn_body<'a>(module: &'a Module, name: &str) -> &'a Block {
    module
        .decls
        .iter()
        .find_map(|d| match &d.node {
            Decl::Fn(f) if f.name == name => Some(f),
            _ => None,
        })
        .and_then(|f| match &f.body {
            FnBody::Block(b) => Some(b),
            _ => None,
        })
        .expect("function with a block body")
}

fn has_defer(block: &Block) -> bool {
    block.stmts.iter().any(|s| matches!(s.node, Stmt::Defer(_)))
}

#[test]
fn finally_is_not_left_on_the_try_or_turned_into_a_function_defer() {
    let mut module = parse(
        "fn F(): Int { try { print(\"body\") } catch (e) { print(\"caught\") } finally { print(\"cleanup\") } print(\"after\"); return 0; }",
    );
    assert!(desugar(&mut module).is_empty());
    let body = fn_body(&module, "F");
    assert!(!has_defer(body), "finally must not park a defer at function scope");
    let try_index = body
        .stmts
        .iter()
        .position(|s| matches!(s.node, Stmt::Try { .. }))
        .expect("try survives desugar");
    match &body.stmts[try_index].node {
        Stmt::Try { finally, .. } => assert!(finally.is_none(), "finally must be consumed"),
        _ => unreachable!(),
    }
    let after = &body.stmts[try_index + 1].node;
    assert!(
        matches!(after, Stmt::If { .. }),
        "the fallthrough cleanup must follow the try, got {after:?}"
    );
    let Stmt::If { cond: IfCond::Expr(c), then, .. } = after else {
        panic!("expected the cleanup if");
    };
    assert!(matches!(c.node, Expr::Bool(true)));
    assert!(
        then.stmts.iter().any(|s| matches!(s.node, Stmt::Expr(_))),
        "cleanup body must carry the finally statements"
    );
}

#[test]
fn finally_copies_precede_return_and_escaping_throw() {
    let mut module = parse(
        "fn F(): Int throws { try { return 1; } catch (e) { throw \"again\"; } finally { print(\"cleanup\"); } }",
    );
    assert!(desugar(&mut module).is_empty());
    let body = fn_body(&module, "F");
    let Stmt::Try { body: try_body, catch, .. } = &body.stmts[0].node else {
        panic!("first statement must be the try");
    };
    let ret_pos = try_body
        .stmts
        .iter()
        .position(|s| matches!(s.node, Stmt::Return(_)))
        .expect("return survives");
    assert!(
        try_body.stmts[..ret_pos]
            .iter()
            .any(|s| matches!(s.node, Stmt::Expr(_))),
        "cleanup must run before the return"
    );
    let (_, catch_body) = catch.as_ref().expect("catch survives");
    let throw_pos = catch_body
        .stmts
        .iter()
        .position(|s| matches!(s.node, Stmt::Throw(_)))
        .expect("throw survives");
    assert!(
        catch_body.stmts[..throw_pos]
            .iter()
            .any(|s| matches!(s.node, Stmt::Expr(_))),
        "cleanup must run before a throw escaping the catch"
    );
    assert!(
        catch_body.stmts[..throw_pos]
            .iter()
            .any(|s| matches!(s.node, Stmt::Var { .. })),
        "the thrown value must be pinned before the cleanup runs"
    );
}

#[test]
fn finally_guards_question_mark_with_the_cleanup_arm() {
    let mut module = parse(
        "fn F(): Result<Int, String> { try { let x = risky()?; return Ok(x); } finally { print(\"cleanup\"); } }",
    );
    assert!(desugar(&mut module).is_empty());
    let body = fn_body(&module, "F");
    let Stmt::Try { body: try_body, .. } = &body.stmts[0].node else {
        panic!("first statement must be the try");
    };
    let Stmt::Var { value, .. } = &try_body.stmts[0].node else {
        panic!("the `?` binding must survive first");
    };
    let Expr::Propagate(guarded) = &value.node else {
        panic!("the `?` must be reapplied to the guarded switch");
    };
    let Expr::Switch { cases, default, .. } = &guarded.node else {
        panic!("the `?` operand must be a switch, got {:?}", guarded.node);
    };
    assert!(default.is_none(), "Ok and Err cover the Result");
    assert_eq!(cases.len(), 2);
    let SwitchExprBody::Block(err_arm) = &cases[1].body else {
        panic!("the Err arm must carry the cleanup block");
    };
    assert!(
        err_arm
            .stmts
            .iter()
            .any(|s| matches!(s.node, Stmt::Expr(_))),
        "the Err arm must run the finally body"
    );
    assert!(
        matches!(
            err_arm.stmts.last().map(|s| &s.node),
            Some(Stmt::Expr(e)) if matches!(e.node, Expr::Call { .. })
        ),
        "the Err arm must end by rebuilding the error Result"
    );
}
