use crate::ast as A;
use crate::ast::sp;
use diagnostics::{Diagnostic, Span};

const Z: Span = Span { start: 0, end: 0 };

fn ident(name: &str) -> A::Spanned<A::Expr> {
    sp(A::Expr::Ident(name.to_string()), Z)
}

fn intlit(n: i64) -> A::Spanned<A::Expr> {
    sp(A::Expr::Int(n), Z)
}

fn floatlit(n: f64) -> A::Spanned<A::Expr> {
    sp(A::Expr::Float(n), Z)
}

fn boollit(b: bool) -> A::Spanned<A::Expr> {
    sp(A::Expr::Bool(b), Z)
}

fn strlit(text: &str) -> A::Spanned<A::Expr> {
    sp(A::Expr::Interp(vec![A::InterpPart::Text(text.to_string())]), Z)
}

fn call(name: &str, args: Vec<A::Spanned<A::Expr>>) -> A::Spanned<A::Expr> {
    sp(
        A::Expr::Call {
            callee: Box::new(ident(name)),
            type_args: Vec::new(),
            args: args
                .into_iter()
                .map(|value| A::CallArg { name: None, value })
                .collect(),
            trailing: None,
        },
        Z,
    )
}

fn binop(op: A::BinOp, lhs: A::Spanned<A::Expr>, rhs: A::Spanned<A::Expr>) -> A::Spanned<A::Expr> {
    sp(A::Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, Z)
}

fn stmt(node: A::Stmt) -> A::Spanned<A::Stmt> {
    sp(node, Z)
}

fn var(name: &str, ty: &str, value: A::Spanned<A::Expr>) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::Var {
        mutable: true,
        is_static: false,
        name: name.to_string(),
        ty: Some(A::Type::named(ty)),
        value,
    })
}

fn assign(name: &str, value: A::Spanned<A::Expr>) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::Assign { target: ident(name), op: A::AssignOp::Eq, value })
}

fn expr_stmt(e: A::Spanned<A::Expr>) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::Expr(e))
}

fn while_stmt(cond: A::Spanned<A::Expr>, body: Vec<A::Spanned<A::Stmt>>) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::While { cond, body: A::Block { stmts: body } })
}

fn if_stmt(cond: A::Spanned<A::Expr>, body: Vec<A::Spanned<A::Stmt>>) -> A::Spanned<A::Stmt> {
    stmt(A::Stmt::If {
        cond: A::IfCond::Expr(cond),
        then: A::Block { stmts: body },
        otherwise: None,
    })
}

fn varname(base: &str, suffix: &str) -> String {
    format!("{base}_{suffix}")
}

fn sample_block(
    internal: &str,
    batch: &str,
    out: &str,
    ctr: &str,
    t0: &str,
) -> Vec<A::Spanned<A::Stmt>> {
    vec![
        assign(t0, call("__rnx_clock_mono", vec![])),
        assign(ctr, intlit(0)),
        while_stmt(
            binop(A::BinOp::Lt, ident(ctr), ident(batch)),
            vec![
                expr_stmt(call(internal, vec![])),
                assign(ctr, binop(A::BinOp::Add, ident(ctr), intlit(1))),
            ],
        ),
        assign(
            out,
            binop(
                A::BinOp::Div,
                call(
                    "Float",
                    vec![binop(
                        A::BinOp::Sub,
                        call("__rnx_clock_mono", vec![]),
                        ident(t0),
                    )],
                ),
                ident(batch),
            ),
        ),
    ]
}

fn bench_block(display: &str, internal: &str) -> Vec<A::Spanned<A::Stmt>> {
    let base: String =
        internal.chars().map(|c| if c.is_ascii_alphanumeric() { c } else { '_' }).collect();
    let iters = varname(&base, "iters");
    let dt = varname(&base, "dt");
    let batch = varname(&base, "batch");
    let ctr = varname(&base, "i");
    let t0 = varname(&base, "t0");
    let mut stmts = vec![
        var(&iters, "Int", intlit(1)),
        var(&dt, "Int", intlit(0)),
        var(&t0, "Int", intlit(0)),
        var(&ctr, "Int", intlit(0)),
        while_stmt(
            boollit(true),
            vec![
                assign(&t0, call("__rnx_clock_mono", vec![])),
                assign(&ctr, intlit(0)),
                while_stmt(
                    binop(A::BinOp::Lt, ident(&ctr), ident(&iters)),
                    vec![
                        expr_stmt(call(internal, vec![])),
                        assign(&ctr, binop(A::BinOp::Add, ident(&ctr), intlit(1))),
                    ],
                ),
                assign(&dt, binop(A::BinOp::Sub, call("__rnx_clock_mono", vec![]), ident(&t0))),
                if_stmt(
                    binop(A::BinOp::GtEq, ident(&dt), intlit(20000000)),
                    vec![stmt(A::Stmt::Break)],
                ),
                assign(&iters, binop(A::BinOp::Mul, ident(&iters), intlit(10))),
                if_stmt(
                    binop(A::BinOp::Gt, ident(&iters), intlit(1000000000)),
                    vec![stmt(A::Stmt::Break)],
                ),
            ],
        ),
        if_stmt(binop(A::BinOp::Eq, ident(&dt), intlit(0)), vec![assign(&dt, intlit(1))]),
        var(
            &batch,
            "Int",
            binop(
                A::BinOp::Div,
                binop(A::BinOp::Mul, ident(&iters), intlit(100000000)),
                ident(&dt),
            ),
        ),
        if_stmt(binop(A::BinOp::Lt, ident(&batch), intlit(1)), vec![assign(&batch, intlit(1))]),
    ];
    let mut means = Vec::new();
    for k in 0..5 {
        let m = varname(&base, &format!("m{k}"));
        means.push(m.clone());
        stmts.push(var(&m, "Float", floatlit(0.0)));
        let st0 = varname(&base, &format!("st{k}"));
        stmts.push(var(&st0, "Int", intlit(0)));
        stmts.extend(sample_block(internal, &batch, &m, &ctr, &st0));
    }
    let mut mean_expr = ident(&means[0]);
    for m in means.iter().skip(1) {
        mean_expr = binop(A::BinOp::Add, mean_expr, ident(m));
    }
    let mean_expr = binop(A::BinOp::Div, mean_expr, intlit(5));
    stmts.push(var(&varname(&base, "mean"), "Float", mean_expr));
    let mean = varname(&base, "mean");
    let mut var_expr: Option<A::Spanned<A::Expr>> = None;
    for m in &means {
        let d = binop(A::BinOp::Sub, ident(m), ident(&mean));
        let sq = binop(A::BinOp::Mul, d.clone(), d);
        var_expr = Some(match var_expr {
            None => sq,
            Some(e) => binop(A::BinOp::Add, e, sq),
        });
    }
    let var_expr = binop(A::BinOp::Div, var_expr.expect("samples"), floatlit(4.0));
    stmts.push(var(&varname(&base, "var"), "Float", var_expr));
    let sd = varname(&base, "sd");
    stmts.push(var(
        &sd,
        "Float",
        call("__rnx_math_sqrt", vec![ident(&varname(&base, "var"))]),
    ));
    let mut line = binop(A::BinOp::Add, strlit("bench "), strlit(display));
    line = binop(A::BinOp::Add, line, strlit(" ... "));
    line = binop(A::BinOp::Add, line, ident(&mean));
    line = binop(A::BinOp::Add, line, strlit(" ns/iter (+/- "));
    line = binop(A::BinOp::Add, line, ident(&sd));
    line = binop(A::BinOp::Add, line, strlit(") ["));
    line = binop(A::BinOp::Add, line, ident(&batch));
    line = binop(A::BinOp::Add, line, strlit(" iters]"));
    stmts.push(expr_stmt(call("print", vec![line])));
    stmts
}

pub fn generate_bench_harness(
    module: &mut A::Module,
    benches: &[(String, String)],
) -> Result<(), Diagnostic> {
    module.decls.retain(|d| !matches!(&d.node, A::Decl::Fn(f) if f.name == "Main"));
    let mut stmts = Vec::new();
    for (display, internal) in benches {
        stmts.extend(bench_block(display, internal));
    }
    stmts.push(stmt(A::Stmt::Return(Some(intlit(0)))));
    let main = A::Decl::Fn(A::FnDecl {
        docs: String::new(),
        access: A::Access::Internal,
        name: "Main".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        ret: Some(A::Type::named("Int")),
        throws: false,
        is_unsafe: false,
        is_async: false,
        is_static: false,
        is_test: false,
        is_bench: false,
        body: A::FnBody::Block(A::Block { stmts }),
        attrs: Vec::new(),
    });
    module.decls.push(sp(main, Z));
    Ok(())
}
