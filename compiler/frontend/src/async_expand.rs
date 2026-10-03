use crate::ast::*;
use diagnostics::{decl_file, tag_new, Code, DeclFiles, Diagnostic, Span};
use std::collections::BTreeSet;

const Z: Span = Span { start: 0, end: 0 };

fn ty_int() -> Type {
    Type { path: vec!["std.prelude.Int".to_string()], args: Vec::new(), nullable: false, fn_sig: None, tuple: Vec::new() }
}


fn ex(e: Expr) -> Spanned<Expr> {
    sp(e, Z)
}

fn mk_stmt(s: Stmt) -> Spanned<Stmt> {
    sp(s, Z)
}

fn ident(name: &str) -> Spanned<Expr> {
    ex(Expr::Ident(name.to_string()))
}

fn int_lit(v: i64) -> Spanned<Expr> {
    ex(Expr::Int(v))
}

fn call(callee: Spanned<Expr>, args: Vec<Spanned<Expr>>) -> Spanned<Expr> {
    ex(Expr::Call {
        callee: Box::new(callee),
        type_args: Vec::new(),
        args: args.into_iter().map(|v| CallArg { name: None, value: v }).collect(),
        trailing: None,
    })
}


fn expr_has_await(e: &Expr) -> bool {
    match e {
        Expr::Await(_) => true,
        Expr::Binary { lhs, rhs, .. } => expr_has_await(&lhs.node) || expr_has_await(&rhs.node),
        Expr::Is { base, .. } => expr_has_await(&base.node),
        Expr::Unary { rhs, .. } => expr_has_await(&rhs.node),
        Expr::Postfix { expr, .. } => expr_has_await(&expr.node),
        Expr::Ternary { cond, then, otherwise } => {
            expr_has_await(&cond.node) || expr_has_await(&then.node) || expr_has_await(&otherwise.node)
        }        Expr::Coalesce { lhs, rhs } => {
            expr_has_await(&lhs.node) || expr_has_await(&rhs.node)
        }
        Expr::OptChain { base, .. } => expr_has_await(&base.node),
        Expr::OptCall { base, args, .. } => {
            expr_has_await(&base.node) || args.iter().any(|a| expr_has_await(&a.value.node))
        }
        Expr::Range { lo, hi, .. } => expr_has_await(&lo.node) || expr_has_await(&hi.node),
        Expr::Call { callee, args, trailing, .. } => {
            expr_has_await(&callee.node)
                || args.iter().any(|a| expr_has_await(&a.value.node))
                || trailing.as_ref().map(block_has_await).unwrap_or(false)
        }
        Expr::Index { base, index } => expr_has_await(&base.node) || expr_has_await(&index.node),
        Expr::Member { base, .. } => expr_has_await(&base.node),
        Expr::Interp(parts) => parts.iter().any(|p| match p {
            InterpPart::Expr(x) => expr_has_await(&x.node),
            _ => false,
        }),
        Expr::Array(items) => items.iter().any(|i| expr_has_await(&i.expr.node)),
        Expr::Record(fields) => fields.iter().any(|e| expr_has_await(&e.value().node)),
        Expr::MapLiteral(entries) => entries.iter().any(|e| expr_has_await(&e.value().node)),
        Expr::Macro { args, .. } => args.iter().any(|a| expr_has_await(&a.node)),
        Expr::Closure { is_async: true, .. } => false,
        Expr::Closure { body, .. } => match body {
            FnBody::Block(b) => block_has_await(b),
            FnBody::Expr(x) => expr_has_await(&x.node),
        },
        Expr::UnsafeBlock(b) => block_has_await(b),
        _ => false,
    }
}

fn block_has_await(b: &Block) -> bool {
    b.stmts.iter().any(|s| stmt_has_await(&s.node))
}

fn stmt_has_await(s: &Stmt) -> bool {
    match s {
        Stmt::Var { value, .. } => expr_has_await(&value.node),
        Stmt::Assign { target, value, .. } => {
            expr_has_await(&target.node) || expr_has_await(&value.node)
        }
        Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => {
            expr_has_await(&e.node)
        }
        Stmt::If { cond, then, otherwise } => {
            let c = match cond {
                IfCond::Expr(e) => expr_has_await(&e.node),
                IfCond::Let { value, .. } => expr_has_await(&value.node),
            };
            c || block_has_await(then)
                || otherwise.as_ref().map(else_has_await).unwrap_or(false)
        }
        Stmt::While { cond, body } | Stmt::DoWhile { cond, body } => {
            expr_has_await(&cond.node) || block_has_await(body)
        }
        Stmt::For { iter, body, .. } => expr_has_await(&iter.node) || block_has_await(body),
        Stmt::Switch { scrutinee, cases, default } => {
            expr_has_await(&scrutinee.node)
                || cases.iter().any(|c| {
                    c.guard.as_ref().map(|g| expr_has_await(&g.node)).unwrap_or(false)
                        || c.body.iter().any(|x| stmt_has_await(&x.node))
                })
                || default.as_ref().map(|d| d.iter().any(|x| stmt_has_await(&x.node))).unwrap_or(false)
        }
        Stmt::Defer(b) | Stmt::UnsafeBlock(b) => block_has_await(b),
        Stmt::Guard { value, otherwise, .. } => {
            expr_has_await(&value.node) || block_has_await(otherwise)
        }
        Stmt::Try { body, catch, finally } => {
            block_has_await(body)
                || catch.as_ref().map(|(_, b)| block_has_await(b)).unwrap_or(false)
                || finally.as_ref().map(block_has_await).unwrap_or(false)
        }
        _ => false,
    }
}

fn else_has_await(e: &Else) -> bool {
    match e {
        Else::Block(b) => block_has_await(b),
        Else::If(s) => stmt_has_await(&s.node),
    }
}

fn await_position_error(span: Span) -> Diagnostic {
    Diagnostic::new(
        Code::E108,
        "await is only supported in `let`, expression, `return`, or `throw` position; loop and branch bodies may suspend, but conditions, assignment targets, assertions, and destructuring cannot",
    )
    .with_span(span)
}

fn check_await_positions_block(body: &Block, diags: &mut Vec<Diagnostic>) -> bool {
    check_await_positions_vec(&body.stmts, diags)
}

fn check_await_positions_vec(stmts: &[Spanned<Stmt>], diags: &mut Vec<Diagnostic>) -> bool {
    for s in stmts {
        if !check_await_positions_stmt(s, diags) {
            return false;
        }
    }
    true
}

fn check_await_positions_stmt(s: &Spanned<Stmt>, diags: &mut Vec<Diagnostic>) -> bool {
    match &s.node {
        Stmt::Var { .. } | Stmt::Expr(_) | Stmt::Return(_) | Stmt::Throw(_) => true,
        Stmt::Assign { target, .. } => {
            if expr_has_await(&target.node) {
                diags.push(await_position_error(s.span));
                return false;
            }
            true
        }
        Stmt::If { cond, then, otherwise } => {
            let cond_ok = match cond {
                IfCond::Expr(e) => !expr_has_await(&e.node),
                IfCond::Let { value, .. } => !expr_has_await(&value.node),
            };
            if !cond_ok {
                diags.push(await_position_error(s.span));
                return false;
            }
            check_await_positions_block(then, diags)
                && match otherwise {
                    None => true,
                    Some(Else::Block(b)) => check_await_positions_block(b, diags),
                    Some(Else::If(inner)) => check_await_positions_stmt(inner, diags),
                }
        }
        Stmt::While { cond, body } | Stmt::DoWhile { body, cond } => {
            if expr_has_await(&cond.node) {
                diags.push(await_position_error(s.span));
                return false;
            }
            check_await_positions_block(body, diags)
        }
        Stmt::For { iter, body, .. } => {
            if expr_has_await(&iter.node) {
                diags.push(await_position_error(s.span));
                return false;
            }
            check_await_positions_block(body, diags)
        }
        Stmt::Switch { scrutinee, cases, default } => {
            if expr_has_await(&scrutinee.node) {
                diags.push(await_position_error(s.span));
                return false;
            }
            for c in cases {
                if let Some(g) = &c.guard {
                    if expr_has_await(&g.node) {
                        diags.push(await_position_error(g.span));
                        return false;
                    }
                }
                if !check_await_positions_vec(&c.body, diags) {
                    return false;
                }
            }
            if let Some(d) = default {
                if !check_await_positions_vec(d, diags) {
                    return false;
                }
            }
            true
        }
        Stmt::Try { body, catch, finally } => {
            check_await_positions_block(body, diags)
                && catch.as_ref().map(|(_, b)| check_await_positions_block(b, diags)).unwrap_or(true)
                && finally.as_ref().map(|b| check_await_positions_block(b, diags)).unwrap_or(true)
        }
        Stmt::Defer(b) | Stmt::UnsafeBlock(b) => check_await_positions_block(b, diags),
        Stmt::Guard { value, otherwise, .. } => {
            if expr_has_await(&value.node) {
                diags.push(await_position_error(s.span));
                return false;
            }
            check_await_positions_block(otherwise, diags)
        }
        _ => {
            if stmt_has_await(&s.node) {
                diags.push(await_position_error(s.span));
                return false;
            }
            true
        }
    }
}

fn direct_awaits(e: &Expr, out: &mut Vec<Spanned<Expr>>) {
    match e {
        Expr::Await(inner) => out.push((**inner).clone()),
        Expr::Binary { lhs, rhs, .. } => {
            direct_awaits(&lhs.node, out);
            direct_awaits(&rhs.node, out);
        }
        Expr::Is { base, .. } => direct_awaits(&base.node, out),
        Expr::Unary { rhs, .. } => direct_awaits(&rhs.node, out),
        Expr::Postfix { expr, .. } => direct_awaits(&expr.node, out),
        Expr::Ternary { cond, then, otherwise } => {
            direct_awaits(&cond.node, out);
            direct_awaits(&then.node, out);
            direct_awaits(&otherwise.node, out);
        }        Expr::Coalesce { lhs, rhs } => {
            direct_awaits(&lhs.node, out);
            direct_awaits(&rhs.node, out);
        }
        Expr::OptChain { base, .. } => direct_awaits(&base.node, out),
        Expr::OptCall { base, args, .. } => {
            direct_awaits(&base.node, out);
            for a in args {
                direct_awaits(&a.value.node, out);
            }
        }
        Expr::Range { lo, hi, .. } => {
            direct_awaits(&lo.node, out);
            direct_awaits(&hi.node, out);
        }
        Expr::Call { callee, args, .. } => {
            direct_awaits(&callee.node, out);
            for a in args {
                direct_awaits(&a.value.node, out);
            }
        }
        Expr::Index { base, index } => {
            direct_awaits(&base.node, out);
            direct_awaits(&index.node, out);
        }
        Expr::Member { base, .. } => direct_awaits(&base.node, out),
        Expr::Array(items) => {
            for i in items {
                direct_awaits(&i.expr.node, out);
            }
        }
        Expr::Record(fields) => {
            for e in fields {
                direct_awaits(&e.value().node, out);
            }
        }
        Expr::MapLiteral(entries) => {
            for e in entries {
                direct_awaits(&e.value().node, out);
            }
        }
        Expr::Macro { args, .. } => {
            for a in args {
                direct_awaits(&a.node, out);
            }
        }
        Expr::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    direct_awaits(&x.node, out);
                }
            }
        }
        _ => {}
    }
}

fn subst_direct_awaits(e: Spanned<Expr>, tmps: &mut Vec<String>) -> Spanned<Expr> {
    let span = e.span;
    match e.node {
        Expr::Await(_) => {
            let t = format!("__aw{}", tmps.len());
            tmps.push(t.clone());
            sp(Expr::Ident(t), span)
        }
        Expr::Binary { op, lhs, rhs } => {
            let lhs = subst_direct_awaits(*lhs, tmps);
            let rhs = subst_direct_awaits(*rhs, tmps);
            sp(Expr::Binary { op, lhs: Box::new(lhs), rhs: Box::new(rhs) }, span)
        }
        Expr::Is { base, target } => {
            let base = subst_direct_awaits(*base, tmps);
            sp(Expr::Is { base: Box::new(base), target }, span)
        }
        Expr::Unary { op, rhs } => {
            let rhs = subst_direct_awaits(*rhs, tmps);
            sp(Expr::Unary { op, rhs: Box::new(rhs) }, span)
        }
        Expr::Postfix { op, expr } => {
            let expr = subst_direct_awaits(*expr, tmps);
            sp(Expr::Postfix { op, expr: Box::new(expr) }, span)
        }
        Expr::Ternary { cond, then, otherwise } => {
            let cond = subst_direct_awaits(*cond, tmps);
            let then = subst_direct_awaits(*then, tmps);
            let otherwise = subst_direct_awaits(*otherwise, tmps);
            sp(
                Expr::Ternary {
                    cond: Box::new(cond),
                    then: Box::new(then),
                    otherwise: Box::new(otherwise),
                },
                span,
            )
        }        Expr::Coalesce { lhs, rhs } => {
            let lhs = subst_direct_awaits(*lhs, tmps);
            let rhs = subst_direct_awaits(*rhs, tmps);
            sp(
                Expr::Coalesce {
                    lhs: Box::new(lhs),
                    rhs: Box::new(rhs),
                },
                span,
            )
        }
        Expr::OptChain { base, field } => {
            let base = subst_direct_awaits(*base, tmps);
            sp(Expr::OptChain { base: Box::new(base), field }, span)
        }
        Expr::OptCall { base, field, args } => {
            let base = subst_direct_awaits(*base, tmps);
            let args = args
                .into_iter()
                .map(|mut a| {
                    a.value = subst_direct_awaits(a.value, tmps);
                    a
                })
                .collect();
            sp(
                Expr::OptCall {
                    base: Box::new(base),
                    field,
                    args,
                },
                span,
            )
        }
        Expr::Range { lo, hi, inclusive } => {
            let lo = subst_direct_awaits(*lo, tmps);
            let hi = subst_direct_awaits(*hi, tmps);
            sp(Expr::Range { lo: Box::new(lo), hi: Box::new(hi), inclusive }, span)
        }
        Expr::Call { callee, type_args, args, trailing } => {
            let callee = subst_direct_awaits(*callee, tmps);
            let mut nargs = Vec::with_capacity(args.len());
            for a in args {
                nargs.push(CallArg { name: a.name, value: subst_direct_awaits(a.value, tmps) });
            }
            sp(Expr::Call { callee: Box::new(callee), type_args, args: nargs, trailing }, span)
        }
        Expr::Index { base, index } => {
            let base = subst_direct_awaits(*base, tmps);
            let index = subst_direct_awaits(*index, tmps);
            sp(Expr::Index { base: Box::new(base), index: Box::new(index) }, span)
        }
        Expr::Member { base, field } => {
            let base = subst_direct_awaits(*base, tmps);
            sp(Expr::Member { base: Box::new(base), field }, span)
        }
        Expr::Array(items) => {
            sp(
                Expr::Array(
                    items
                        .into_iter()
                        .map(|mut i| {
                            i.expr = subst_direct_awaits(i.expr, tmps);
                            i
                        })
                        .collect(),
                ),
                span,
            )
        }
        Expr::Record(fields) => {
            sp(
                Expr::Record(
                    fields.into_iter().map(|e| e.map_value(|v| subst_direct_awaits(v, tmps))).collect(),
                ),
                span,
            )
        }
        Expr::MapLiteral(entries) => {
            sp(
                Expr::MapLiteral(
                    entries.into_iter().map(|e| e.map_value(|v| subst_direct_awaits(v, tmps))).collect(),
                ),
                span,
            )
        }
        Expr::Macro { name, args, bracket } => {
            sp(
                Expr::Macro {
                    name,
                    args: args.into_iter().map(|a| subst_direct_awaits(a, tmps)).collect(),
                    bracket,
                },
                span,
            )
        }
        Expr::Interp(parts) => {
            sp(
                Expr::Interp(
                    parts
                        .into_iter()
                        .map(|p| match p {
                            InterpPart::Text(t) => InterpPart::Text(t),
                            InterpPart::Expr(x) => InterpPart::Expr(subst_direct_awaits(x, tmps)),
                        })
                        .collect(),
                ),
                span,
            )
        }
        other => sp(other, span),
    }
}

pub fn expand_async(module: &mut Module, diags: &mut Vec<Diagnostic>) {
    lift_async_closures(module);
    let asyncs: Vec<FnDecl> = module
        .decls
        .iter()
        .filter_map(|d| match &d.node {
            Decl::Fn(f) if f.is_async => Some(f.clone()),
            _ => None,
        })
        .collect();
    for f in &asyncs {
        let is_entry = f.name == "Main" || f.name == "main";
        let out_name = if is_entry {
            Some(format!("__{}_async", f.name))
        } else {
            None
        };
        expand_promise_fn(module, f, out_name.as_deref(), !is_entry, diags);
    }    for entry in ["Main", "main"] {
        expand_entry_driver(module, entry);
    }
}

const LIFT_BUILTINS: &[&str] = &[
    "print",
    "assert",
    "typeOf",
    "IO",
    "Thread",
    "ThreadPool",
    "Pointer",
    "Address",
];

fn ty_promise(inner: Type) -> Type {
    Type { path: vec!["std.prelude.Promise".to_string()], args: vec![inner], nullable: false, fn_sig: None, tuple: Vec::new() }
}

fn is_void_ret(ret: &Option<Type>) -> bool {
    match ret {
        Some(t) => {
            t.fn_sig.is_none()
                && t.tuple.is_empty()
                && t.args.is_empty()
                && t.path.last().map(|s| s == "Void").unwrap_or(false)
        }
        None => false,
    }
}

fn box_tag_for_promise(ret: &Type) -> Option<i64> {
    let inner = ret.args.first()?;
    let short = inner.path.last().map(|s| s.rsplit('.').next().unwrap_or(s.as_str()))?;
    match short {
        "Int" | "Int64" | "Int32" | "Int16" | "Int8" | "UInt" | "UInt64" | "UInt32" | "UInt16" | "UInt8" | "Byte" | "Short" => Some(0),
        "Bool" => Some(1),
        "Float" | "Float32" | "FastFloat" | "FastFloat32" => Some(2),
        "String" | "Char" => Some(3),
        _ => None,
    }
}

fn box_value(tag: Option<i64>, value: Spanned<Expr>) -> Spanned<Expr> {
    match tag {
        Some(t) => call(ident("__rnx_any_box"), vec![int_lit(t), value]),
        None => value,
    }
}

fn fulfill_call(wr: &str, value: Spanned<Expr>, box_tag: Option<i64>) -> Spanned<Stmt> {
    mk_stmt(Stmt::Expr(call(
        ex(Expr::Member { base: Box::new(ident(wr)), field: "resolve".to_string() }),
        vec![box_value(box_tag, value)],
    )))
}

fn reject_call(wr: &str, reason: Spanned<Expr>) -> Spanned<Stmt> {
    mk_stmt(Stmt::Expr(call(
        ex(Expr::Member { base: Box::new(ident(wr)), field: "reject".to_string() }),
        vec![reason],
    )))
}

fn wait_call(target: Spanned<Expr>) -> Spanned<Expr> {
    let span = target.span;
    sp(
        Expr::Call {
            callee: Box::new(sp(
                Expr::Member {
                    base: Box::new(target),
                    field: "wait".to_string(),
                },
                span,
            )),
            type_args: Vec::new(),
            args: Vec::new(),
            trailing: None,
        },
        span,
    )
}

fn unwrap_call(target: Spanned<Expr>) -> Spanned<Expr> {
    let span = target.span;
    sp(
        Expr::Call {
            callee: Box::new(sp(
                Expr::Member { base: Box::new(target), field: "unwrap".to_string() },
                span,
            )),
            type_args: Vec::new(),
            args: Vec::new(),
            trailing: None,
        },
        span,
    )
}

fn rewrite_awaits_in_expr(e: Spanned<Expr>, prefix: &mut Vec<Spanned<Stmt>>) -> Spanned<Expr> {
    let mut kids = Vec::new();
    direct_awaits(&e.node, &mut kids);
    if kids.is_empty() {
        return e;
    }
    let mut tmps = Vec::new();
    let rewritten = subst_direct_awaits(e, &mut tmps);
    for (k, child) in kids.into_iter().enumerate() {
        let span = child.span;
        prefix.push(sp(
            Stmt::Var {
                mutable: false,
                is_static: false,
                name: tmps[k].clone(),
                ty: None,
                value: wait_call(child),
            },
            span,
        ));
    }
    await_tmps_to_unwrap(rewritten, &tmps)
}

fn await_tmps_to_unwrap(e: Spanned<Expr>, tmps: &[String]) -> Spanned<Expr> {
    let span = e.span;
    match e.node {
        Expr::Ident(n) if tmps.contains(&n) => unwrap_call(ex(Expr::Ident(n))),
        Expr::Binary { op, lhs, rhs } => {
            let lhs = Box::new(await_tmps_to_unwrap(*lhs, tmps));
            let rhs = Box::new(await_tmps_to_unwrap(*rhs, tmps));
            sp(Expr::Binary { op, lhs, rhs }, span)
        }
        Expr::Is { base, target } => {
            sp(Expr::Is { base: Box::new(await_tmps_to_unwrap(*base, tmps)), target }, span)
        }
        Expr::Unary { op, rhs } => {
            sp(Expr::Unary { op, rhs: Box::new(await_tmps_to_unwrap(*rhs, tmps)) }, span)
        }
        Expr::Postfix { op, expr } => {
            sp(Expr::Postfix { op, expr: Box::new(await_tmps_to_unwrap(*expr, tmps)) }, span)
        }
        Expr::Ternary { cond, then, otherwise } => sp(
            Expr::Ternary {
                cond: Box::new(await_tmps_to_unwrap(*cond, tmps)),
                then: Box::new(await_tmps_to_unwrap(*then, tmps)),
                otherwise: Box::new(await_tmps_to_unwrap(*otherwise, tmps)),
            },
            span,
        ),
        Expr::Coalesce { lhs, rhs } => sp(
            Expr::Coalesce {
                lhs: Box::new(await_tmps_to_unwrap(*lhs, tmps)),
                rhs: Box::new(await_tmps_to_unwrap(*rhs, tmps)),
            },
            span,
        ),
        Expr::OptChain { base, field } => {
            sp(Expr::OptChain { base: Box::new(await_tmps_to_unwrap(*base, tmps)), field }, span)
        }
        Expr::OptCall { base, field, args } => sp(
            Expr::OptCall {
                base: Box::new(await_tmps_to_unwrap(*base, tmps)),
                field,
                args: args
                    .into_iter()
                    .map(|mut a| {
                        a.value = await_tmps_to_unwrap(a.value, tmps);
                        a
                    })
                    .collect(),
            },
            span,
        ),
        Expr::Range { lo, hi, inclusive } => sp(
            Expr::Range {
                lo: Box::new(await_tmps_to_unwrap(*lo, tmps)),
                hi: Box::new(await_tmps_to_unwrap(*hi, tmps)),
                inclusive,
            },
            span,
        ),
        Expr::Call { callee, type_args, args, trailing } => sp(
            Expr::Call {
                callee: Box::new(await_tmps_to_unwrap(*callee, tmps)),
                type_args,
                args: args
                    .into_iter()
                    .map(|mut a| {
                        a.value = await_tmps_to_unwrap(a.value, tmps);
                        a
                    })
                    .collect(),
                trailing,
            },
            span,
        ),
        Expr::Index { base, index } => sp(
            Expr::Index {
                base: Box::new(await_tmps_to_unwrap(*base, tmps)),
                index: Box::new(await_tmps_to_unwrap(*index, tmps)),
            },
            span,
        ),
        Expr::Member { base, field } => {
            sp(Expr::Member { base: Box::new(await_tmps_to_unwrap(*base, tmps)), field }, span)
        }
        Expr::Array(items) => sp(
            Expr::Array(
                items
                    .into_iter()
                    .map(|mut i| {
                        i.expr = await_tmps_to_unwrap(i.expr, tmps);
                        i
                    })
                    .collect(),
            ),
            span,
        ),
        Expr::Record(fields) => sp(
            Expr::Record(
                fields.into_iter().map(|e| e.map_value(|v| await_tmps_to_unwrap(v, tmps))).collect(),
            ),
            span,
        ),
        Expr::MapLiteral(entries) => sp(
            Expr::MapLiteral(
                entries.into_iter().map(|e| e.map_value(|v| await_tmps_to_unwrap(v, tmps))).collect(),
            ),
            span,
        ),
        Expr::Macro { name, args, bracket } => sp(
            Expr::Macro {
                name,
                args: args.into_iter().map(|a| await_tmps_to_unwrap(a, tmps)).collect(),
                bracket,
            },
            span,
        ),
        Expr::Interp(parts) => sp(
            Expr::Interp(
                parts
                    .into_iter()
                    .map(|p| match p {
                        InterpPart::Expr(x) => InterpPart::Expr(await_tmps_to_unwrap(x, tmps)),
                        other => other,
                    })
                    .collect(),
            ),
            span,
        ),
        other => sp(other, span),
    }
}


fn settle_return(wr: &str, inline: bool) -> Spanned<Stmt> {
    if inline {
        mk_stmt(Stmt::Return(Some(ex(Expr::Member {
            base: Box::new(ident(wr)),
            field: "promise".to_string(),
        }))))
    } else {
        mk_stmt(Stmt::Return(None))
    }
}

fn convert_returns_vec(
    stmts: &mut Vec<Spanned<Stmt>>,
    wr: &str,
    inline: bool,
    box_tag: Option<i64>,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    convert_returns_vec_in(stmts, wr, inline, box_tag, false, diags)
}

fn convert_returns_vec_in(
    stmts: &mut Vec<Spanned<Stmt>>,
    wr: &str,
    inline: bool,
    box_tag: Option<i64>,
    in_try: bool,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    let mut out = Vec::with_capacity(stmts.len());
    for s in std::mem::take(stmts) {
        let span = s.span;
        match s.node {
            Stmt::Return(None) => {
                diags.push(
                    Diagnostic::new(
                        Code::E108,
                        "async fn must return a value; bare `return` cannot settle its Promise",
                    )
                    .with_span(span),
                );
                return false;
            }
            Stmt::Return(Some(e)) => {
                let mut prefix = Vec::new();
                let rewritten = rewrite_awaits_in_expr(e, &mut prefix);
                out.extend(prefix);
                out.push(fulfill_call(wr, rewritten, box_tag));
                out.push(settle_return(wr, inline));
            }
            Stmt::Throw(None) => out.push(sp(Stmt::Throw(None), span)),
            Stmt::Throw(Some(e)) => {
                let mut prefix = Vec::new();
                let rewritten = rewrite_awaits_in_expr(e, &mut prefix);
                out.extend(prefix);
                if in_try {
                    out.push(sp(Stmt::Throw(Some(rewritten)), span));
                } else {
                    out.push(reject_call(wr, rewritten));
                    out.push(settle_return(wr, inline));
                }
            }
            Stmt::Var { mutable, is_static, name, ty, value } => {
                let mut prefix = Vec::new();
                let rewritten = rewrite_awaits_in_expr(value, &mut prefix);
                out.extend(prefix);
                out.push(sp(Stmt::Var { mutable, is_static, name, ty, value: rewritten }, span));
            }
            Stmt::Expr(e) => {
                let mut prefix = Vec::new();
                let rewritten = rewrite_awaits_in_expr(e, &mut prefix);
                out.extend(prefix);
                out.push(sp(Stmt::Expr(rewritten), span));
            }
            Stmt::Assign { target, op, value } => {
                let mut prefix = Vec::new();
                let rewritten = rewrite_awaits_in_expr(value, &mut prefix);
                out.extend(prefix);
                out.push(sp(Stmt::Assign { target, op, value: rewritten }, span));
            }
            Stmt::If { cond, mut then, otherwise } => {
                if !convert_returns_block_in(&mut then, wr, inline, box_tag, in_try, diags) {
                    return false;
                }
                let otherwise = match otherwise {
                    None => None,
                    Some(Else::Block(mut b)) => {
                        if !convert_returns_block_in(&mut b, wr, inline, box_tag, in_try, diags) {
                            return false;
                        }
                        Some(Else::Block(b))
                    }
                    Some(Else::If(inner)) => {
                        let mut tmp = vec![*inner];
                        if !convert_returns_vec_in(&mut tmp, wr, inline, box_tag, in_try, diags) {
                            return false;
                        }
                        Some(Else::Block(Block { stmts: tmp }))
                    }
                };
                out.push(sp(Stmt::If { cond, then, otherwise }, span));
            }
            Stmt::While { cond, mut body } => {
                if !convert_returns_block_in(&mut body, wr, inline, box_tag, in_try, diags) {
                    return false;
                }
                out.push(sp(Stmt::While { cond, body }, span));
            }
            Stmt::DoWhile { mut body, cond } => {
                if !convert_returns_block_in(&mut body, wr, inline, box_tag, in_try, diags) {
                    return false;
                }
                out.push(sp(Stmt::DoWhile { body, cond }, span));
            }
            Stmt::For { binding, iter, mut body } => {
                if !convert_returns_block_in(&mut body, wr, inline, box_tag, in_try, diags) {
                    return false;
                }
                out.push(sp(Stmt::For { binding, iter, body }, span));
            }
            Stmt::Switch { scrutinee, mut cases, default } => {
                for c in cases.iter_mut() {
                    if !convert_returns_vec_in(&mut c.body, wr, inline, box_tag, in_try, diags) {
                        return false;
                    }
                }
                let mut default = default;
                if let Some(d) = default.as_mut() {
                    if !convert_returns_vec_in(d, wr, inline, box_tag, in_try, diags) {
                        return false;
                    }
                }
                out.push(sp(Stmt::Switch { scrutinee, cases, default }, span));
            }
            Stmt::Try { mut body, catch, finally } => {
                if !convert_returns_block_in(&mut body, wr, inline, box_tag, true, diags) {
                    return false;
                }
                let catch = match catch {
                    None => None,
                    Some((name, mut b)) => {
                        if !convert_returns_block_in(&mut b, wr, inline, box_tag, true, diags) {
                            return false;
                        }
                        Some((name, b))
                    }
                };
                let finally = match finally {
                    None => None,
                    Some(mut b) => {
                        if !convert_returns_block_in(&mut b, wr, inline, box_tag, true, diags) {
                            return false;
                        }
                        Some(b)
                    }
                };
                out.push(sp(Stmt::Try { body, catch, finally }, span));
            }
            Stmt::Defer(mut b) => {
                if !convert_returns_block_in(&mut b, wr, inline, box_tag, in_try, diags) {
                    return false;
                }
                out.push(sp(Stmt::Defer(b), span));
            }
            Stmt::UnsafeBlock(mut b) => {
                if !convert_returns_block_in(&mut b, wr, inline, box_tag, in_try, diags) {
                    return false;
                }
                out.push(sp(Stmt::UnsafeBlock(b), span));
            }
            Stmt::Guard { name, value, mut otherwise } => {
                if !convert_returns_block_in(&mut otherwise, wr, inline, box_tag, in_try, diags) {
                    return false;
                }
                out.push(sp(Stmt::Guard { name, value, otherwise }, span));
            }
            other => out.push(sp(other, span)),
        }
    }
    *stmts = out;
    true
}

fn convert_returns_block(
    b: &mut Block,
    wr: &str,
    inline: bool,
    box_tag: Option<i64>,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    convert_returns_block_in(b, wr, inline, box_tag, false, diags)
}

fn convert_returns_block_in(
    b: &mut Block,
    wr: &str,
    inline: bool,
    box_tag: Option<i64>,
    in_try: bool,
    diags: &mut Vec<Diagnostic>,
) -> bool {
    convert_returns_vec_in(&mut b.stmts, wr, inline, box_tag, in_try, diags)
}

fn block_diverges(b: &Block) -> bool {
    b.stmts.last().map(|s| stmt_diverges(&s.node)).unwrap_or(false)
}

fn stmt_diverges(s: &Stmt) -> bool {
    match s {
        Stmt::Return(_) | Stmt::Throw(_) => true,
        Stmt::If { then, otherwise, .. } => {
            let t = block_diverges(then);
            let e = match otherwise {
                None => false,
                Some(Else::Block(b)) => block_diverges(b),
                Some(Else::If(x)) => stmt_diverges(&x.node),
            };
            t && e
        }
        Stmt::Switch { cases, default, .. } => {
            !cases.is_empty()
                && cases
                    .iter()
                    .all(|c| c.body.last().map(|x| stmt_diverges(&x.node)).unwrap_or(false))
                && default
                    .as_ref()
                    .map(|d| d.last().map(|x| stmt_diverges(&x.node)).unwrap_or(false))
                    .unwrap_or(false)
        }
        Stmt::While { cond, body } => is_true_lit(cond) && block_diverges(body),
        Stmt::DoWhile { body, .. } => block_diverges(body),
        Stmt::Try { body, catch, .. } => {
            block_diverges(body)
                && catch.as_ref().map(|(_, b)| block_diverges(b)).unwrap_or(true)
        }
        Stmt::UnsafeBlock(b) => block_diverges(b),
        _ => false,
    }
}

fn is_true_lit(e: &Spanned<Expr>) -> bool {
    matches!(e.node, Expr::Bool(true))
}

fn expand_promise_fn(
    module: &mut Module,
    f: &FnDecl,
    out_name: Option<&str>,
    spawn: bool,
    diags: &mut Vec<Diagnostic>,
) {
    let pos = match module.decls.iter().position(|d| match &d.node {
        Decl::Fn(x) if x.name == f.name && x.is_async => true,
        _ => false,
    }) {
        Some(i) => i,
        None => return,
    };
    let span = module.decls[pos].span;
    let (_params, ret_opt, mut body) = match &module.decls[pos].node {
        Decl::Fn(fd) => (
            fd.params.clone(),
            fd.ret.clone(),
            match &fd.body {
                FnBody::Block(b) => b.clone(),
                FnBody::Expr(e) => Block { stmts: vec![mk_stmt(Stmt::Return(Some((**e).clone())))] },
            },
        ),
        _ => return,
    };
    if is_void_ret(&ret_opt) {
        diags.push(
            Diagnostic::new(
                Code::E108,
                format!(
                    "async fn `{}` cannot return Void; declare a value type T (use Int for status codes)",
                    f.name
                ),
            )
            .with_span(span),
        );
        return;
    }
    let implicit_int = ret_opt.is_none();
    let t = ret_opt.unwrap_or_else(ty_int);
    hoist_await_returns(&mut body);
    if !check_await_positions_block(&body, diags) {
        return;
    }
    if !implicit_int && !block_diverges(&body) {
        diags.push(
            Diagnostic::new(
                Code::E108,
                format!(
                    "async fn `{}` may complete without returning a value, leaving its Promise unsettled; add a `return` on all paths",
                    f.name
                ),
            )
            .with_span(span),
        );
        return;
    }
    let wr = "__wr";
    let box_tag = box_tag_for_promise(&t);
    if !convert_returns_vec(&mut body.stmts, wr, !spawn, box_tag, diags) {
        return;
    }
    if implicit_int && !block_diverges(&body) {
        body.stmts.push(fulfill_call(wr, int_lit(0), Some(0)));
    }
    let try_stmt = mk_stmt(Stmt::Try {
        body,
        catch: Some((
            "__er".to_string(),
            Block { stmts: vec![reject_call(wr, ident("__er"))] },
        )),
        finally: None,
    });
    if !spawn {
        // Entry bodies run inline on the caller's thread so their prints
        // land in the caller's output buffer; the promise is settled
        // before `__wr.promise` is returned.
        return inline_entry_body(module, pos, f, t, try_stmt, wr);
    }
    let worker = ex(Expr::Closure {
        decay: false,
        is_async: false,
        params: Vec::new(),
        ret: Some(Type {
            path: vec!["std.prelude.Void".to_string()],
            args: Vec::new(),
            nullable: false,
            fn_sig: None,
            tuple: Vec::new(),
        }),
        throws: false,
        body: FnBody::Block(Block { stmts: vec![try_stmt] }),
    });
    let tok = "__tok";
    let spawn_call = call(
        ex(Expr::Member { base: Box::new(ident("Thread")), field: "spawn".to_string() }),
        vec![worker],
    );
    let spawn = mk_stmt(Stmt::Var {
        mutable: false,
        is_static: false,
        name: tok.to_string(),
        ty: None,
        value: spawn_call,
    });
    let guard = mk_stmt(Stmt::If {
        cond: IfCond::Expr(ex(Expr::Binary {
            op: BinOp::Lt,
            lhs: Box::new(ident(tok)),
            rhs: Box::new(int_lit(0)),
        })),
        then: Block {
            stmts: vec![reject_call(
                wr,
                ex(Expr::Interp(vec![InterpPart::Text(
                    "async worker thread failed to spawn".to_string(),
                )])),
            )],
        },
        otherwise: None,
    });
    let with_resolvers = ex(Expr::Call {
        callee: Box::new(ex(Expr::Member {
            base: Box::new(ident("std.prelude.Promise")),
            field: "withResolvers".to_string(),
        })),
        type_args: vec![t.clone()],
        args: Vec::new(),
        trailing: None,
    });
    let outer = vec![
        mk_stmt(Stmt::Var {
            mutable: false,
            is_static: false,
            name: wr.to_string(),
            ty: None,
            value: with_resolvers,
        }),
        spawn,
        guard,
        mk_stmt(Stmt::Return(Some(ex(Expr::Member {
            base: Box::new(ident(wr)),
            field: "promise".to_string(),
        })))),
    ];
    let name = out_name.unwrap_or(&f.name).to_string();
    if let Decl::Fn(fd) = &mut module.decls[pos].node {
        fd.name = name;
        fd.is_async = false;
        fd.ret = Some(ty_promise(t));
        fd.body = FnBody::Block(Block { stmts: outer });
    }
}

fn inline_entry_body(
    module: &mut Module,
    pos: usize,
    f: &FnDecl,
    t: Type,
    try_stmt: Spanned<Stmt>,
    wr: &str,
) {
    let with_resolvers = ex(Expr::Call {
        callee: Box::new(ex(Expr::Member {
            base: Box::new(ident("std.prelude.Promise")),
            field: "withResolvers".to_string(),
        })),
        type_args: vec![t.clone()],
        args: Vec::new(),
        trailing: None,
    });
    let outer = vec![
        mk_stmt(Stmt::Var {
            mutable: false,
            is_static: false,
            name: wr.to_string(),
            ty: None,
            value: with_resolvers,
        }),
        try_stmt,
        mk_stmt(Stmt::Return(Some(ex(Expr::Member {
            base: Box::new(ident(wr)),
            field: "promise".to_string(),
        })))),
    ];
    let name = format!("__{}_async", f.name);
    if let Decl::Fn(fd) = &mut module.decls[pos].node {
        fd.name = name;
        fd.is_async = false;
        fd.ret = Some(ty_promise(t));
        fd.body = FnBody::Block(Block { stmts: outer });
    }
}

fn expand_entry_driver(module: &mut Module, entry: &str) {
    let inner = format!("__{entry}_async");
    if module.decls.iter().any(|d| match &d.node {
        Decl::Fn(f) if f.name == entry && !f.is_async => true,
        _ => false,
    }) {
        return;
    }
    let pos = match module.decls.iter().position(|d| match &d.node {
        Decl::Fn(f) if f.name == inner && !f.is_async => true,
        _ => false,
    }) {
        Some(i) => i,
        None => return,
    };
    let span = module.decls[pos].span;
    let waited = wait_call(call(ident(&inner), Vec::new()));
    let driver = Decl::Fn(FnDecl {
        docs: String::new(),
        access: Access::Internal,
        name: entry.to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        ret: Some(ty_int()),
        throws: false,
        is_unsafe: false,
        is_async: false,
        is_static: false,
        is_test: false,
        is_bench: false,
        body: FnBody::Block(Block {
            stmts: vec![
                mk_stmt(Stmt::Var {
                    mutable: false,
                    is_static: false,
                    name: "__r".to_string(),
                    ty: None,
                    value: waited,
                }),
                mk_stmt(Stmt::Switch {
                    scrutinee: ident("__r"),
                    cases: vec![
                        SwitchCase {
                            pattern: Pattern::Enum {
                                path: vec!["Ok".to_string()],
                                args: vec![Pattern::Literal(ident("__v"))],
                            },
                            guard: None,
                            body: vec![mk_stmt(Stmt::Return(Some(ident("__v"))))],
                        },
                        SwitchCase {
                            pattern: Pattern::Enum {
                                path: vec!["Err".to_string()],
                                args: vec![Pattern::Literal(ident("__e"))],
                            },
                            guard: None,
                            body: vec![
                                mk_stmt(Stmt::Expr(call(ident("print"), vec![ident("__e")]))),
                                mk_stmt(Stmt::Return(Some(int_lit(1)))),
                            ],
                        },
                    ],
                    default: None,
                }),
            ],
        }),
        attrs: Vec::new(),
    });
    module.decls.insert(pos + 1, sp(driver, span));
}

pub fn check_async_ret_types(module: &Module, files: &DeclFiles) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for decl in &module.decls {
        let from = out.len();
        let file = decl_file(files, decl.span);
        if let Decl::Fn(f) = &decl.node {
            if !f.is_async {
                tag_new(&mut out, from, file);
                continue;
            }
            if is_void_ret(&f.ret) {
                out.push(
                    Diagnostic::new(
                        Code::E108,
                        format!(
                            "async fn `{}` cannot return Void; declare a value type T (use Int for status codes)",
                            f.name
                        ),
                    )
                    .with_span(decl.span),
                );
            }
        }
        tag_new(&mut out, from, file);
    }
    out
}

pub fn check_no_poll_decls(module: &Module, files: &DeclFiles) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for decl in &module.decls {
        let from = out.len();
        let file = decl_file(files, decl.span);
        let name = match &decl.node {
            Decl::Enum { name, .. }
            | Decl::Class { name, .. }
            | Decl::Struct { name, .. }
            | Decl::Record { name, .. }
            | Decl::Interface { name, .. }
            | Decl::Trait { name, .. } => Some(name),
            _ => None,
        };
        if name.map(|n| n == "Poll").unwrap_or(false) {
            out.push(
                Diagnostic::new(
                    Code::E111,
                    "`Poll` was retired with the polling model; use `Promise` with `await` or `.wait()` instead",
                )
                .with_span(decl.span),
            );
        }
        tag_new(&mut out, from, file);
    }
    out
}

fn module_names(module: &Module) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    for decl in &module.decls {
        match &decl.node {
            Decl::Fn(f) => {
                out.insert(f.name.clone());
            }
            Decl::Class { name, .. }
            | Decl::Struct { name, .. }
            | Decl::Record { name, .. }
            | Decl::Enum { name, .. }
            | Decl::Trait { name, .. }
            | Decl::Interface { name, .. } => {
                out.insert(name.clone());
            }
            Decl::Const { name, .. } => {
                out.insert(name.clone());
            }
            _ => {}
        }
    }
    out
}

struct FreeVars {
    scopes: Vec<BTreeSet<String>>,
    used: BTreeSet<String>,
}

impl FreeVars {
    fn bound(&self, name: &str) -> bool {
        self.scopes.iter().any(|s| s.contains(name))
    }

    fn walk_fnbody(&mut self, body: &FnBody) {
        match body {
            FnBody::Block(b) => self.walk_block(b),
            FnBody::Expr(e) => self.walk_expr(&e.node),
        }
    }

    fn walk_block(&mut self, block: &Block) {
        self.scopes.push(BTreeSet::new());
        for s in &block.stmts {
            self.walk_stmt(&s.node);
        }
        self.scopes.pop();
    }

    fn walk_stmts_scoped(&mut self, stmts: &[Spanned<Stmt>]) {
        self.scopes.push(BTreeSet::new());
        for s in stmts {
            self.walk_stmt(&s.node);
        }
        self.scopes.pop();
    }

    fn walk_stmt(&mut self, s: &Stmt) {
        match s {
            Stmt::Var { name, value, .. } => {
                self.walk_expr(&value.node);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.clone());
                }
            }
            Stmt::DestructureTuple { names, value, .. }
            | Stmt::DestructureArray { names, value, .. } => {
                self.walk_expr(&value.node);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.extend(names.iter().cloned());
                }
            }
            Stmt::DestructureRecord { fields, rest, value, .. } => {
                self.walk_expr(&value.node);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.extend(fields.iter().map(|(_, alias)| alias.clone()));
                    scope.extend(rest.iter().cloned());
                }
            }
            Stmt::Assign { target, value, .. } => {
                self.walk_expr(&target.node);
                self.walk_expr(&value.node);
            }
            Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => {
                self.walk_expr(&e.node);
            }
            Stmt::If { cond, then, otherwise } => {
                match cond {
                    IfCond::Expr(e) | IfCond::Let { value: e, .. } => self.walk_expr(&e.node),
                }
                if let IfCond::Let { name, .. } = cond
                    && let Some(scope) = self.scopes.last_mut()
                {
                    scope.insert(name.clone());
                }
                self.walk_block(then);
                if let Some(otherwise) = otherwise {
                    match otherwise {
                        Else::Block(b) => self.walk_block(b),
                        Else::If(s) => self.walk_stmt(&s.node),
                    }
                }
            }
            Stmt::For { binding, iter, body } => {
                self.walk_expr(&iter.node);
                self.scopes.push(BTreeSet::new());
                match binding {
                    ForBinding::One(n) => {
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.insert(n.clone());
                        }
                    }
                    ForBinding::Many(ns) => {
                        if let Some(scope) = self.scopes.last_mut() {
                            scope.extend(ns.iter().cloned());
                        }
                    }
                }
                for st in &body.stmts {
                    self.walk_stmt(&st.node);
                }
                self.scopes.pop();
            }
            Stmt::While { cond, body } => {
                self.walk_expr(&cond.node);
                self.walk_block(body);
            }
            Stmt::DoWhile { body, cond } => {
                self.walk_block(body);
                self.walk_expr(&cond.node);
            }
            Stmt::Switch { scrutinee, cases, default } => {
                self.walk_expr(&scrutinee.node);
                for c in cases {
                    self.scopes.push(BTreeSet::new());
                    self.walk_pattern(&c.pattern);
                    if let Some(g) = &c.guard {
                        self.walk_expr(&g.node);
                    }
                    for st in &c.body {
                        self.walk_stmt(&st.node);
                    }
                    self.scopes.pop();
                }
                if let Some(d) = default {
                    self.walk_stmts_scoped(d);
                }
            }
            Stmt::Defer(b) | Stmt::UnsafeBlock(b) => self.walk_block(b),
            Stmt::Guard { name, value, otherwise } => {
                self.walk_expr(&value.node);
                if let Some(scope) = self.scopes.last_mut() {
                    scope.insert(name.clone());
                }
                self.walk_block(otherwise);
            }
            Stmt::Try { body, catch, finally } => {
                self.walk_block(body);
                if let Some((name, b)) = catch {
                    self.scopes.push(BTreeSet::new());
                    if let Some(scope) = self.scopes.last_mut() {
                        scope.insert(name.clone());
                    }
                    for st in &b.stmts {
                        self.walk_stmt(&st.node);
                    }
                    self.scopes.pop();
                }
                if let Some(b) = finally {
                    self.walk_block(b);
                }
            }
            _ => {}
        }
    }

    fn walk_pattern(&mut self, pat: &Pattern) {
        match pat {
            Pattern::Literal(e) => match &e.node {
                Expr::Ident(_) => {}
                other => self.walk_expr(other),
            },
            Pattern::Range { lo, hi, .. } => {
                self.walk_expr(&lo.node);
                self.walk_expr(&hi.node);
            }
            Pattern::Enum { args, .. } => {
                for a in args {
                    self.walk_pattern(a);
                }
            }
            Pattern::Wildcard => {}
            Pattern::Is(_) => {}
        }
    }

    fn walk_expr(&mut self, e: &Expr) {
        match e {
            Expr::Ident(name) => {
                if !self.bound(name) {
                    self.used.insert(name.clone());
                }
            }
            Expr::Binary { lhs, rhs, .. } => {
                self.walk_expr(&lhs.node);
                self.walk_expr(&rhs.node);
            }
            Expr::Is { base, .. } => self.walk_expr(&base.node),
            Expr::Unary { rhs, .. } => self.walk_expr(&rhs.node),
            Expr::Postfix { expr, .. } => self.walk_expr(&expr.node),
            Expr::Ternary { cond, then, otherwise } => {
                self.walk_expr(&cond.node);
                self.walk_expr(&then.node);
                self.walk_expr(&otherwise.node);
            }
            Expr::Coalesce { lhs, rhs } => {
                self.walk_expr(&lhs.node);
                self.walk_expr(&rhs.node);
            }
            Expr::OptChain { base, .. } => self.walk_expr(&base.node),
            Expr::OptCall { base, args, .. } => {
                self.walk_expr(&base.node);
                for a in args {
                    self.walk_expr(&a.value.node);
                }
            }
            Expr::Range { lo, hi, .. } => {
                self.walk_expr(&lo.node);
                self.walk_expr(&hi.node);
            }
            Expr::Call { callee, args, trailing, .. } => {
                self.walk_expr(&callee.node);
                for a in args {
                    self.walk_expr(&a.value.node);
                }
                if let Some(b) = trailing {
                    self.walk_block(b);
                }
            }
            Expr::Index { base, index } => {
                self.walk_expr(&base.node);
                self.walk_expr(&index.node);
            }
            Expr::Member { base, .. } => self.walk_expr(&base.node),
            Expr::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.walk_expr(&x.node);
                    }
                }
            }
            Expr::Array(items) => {
                for i in items {
                    self.walk_expr(&i.expr.node);
                }
            }
            Expr::Tuple(items) => {
                for i in items {
                    self.walk_expr(&i.node);
                }
            }
            Expr::TupleGet { base, .. } => self.walk_expr(&base.node),
            Expr::Record(fields) => {
                for e in fields {
                    self.walk_expr(&e.value().node);
                }
            }
            Expr::MapLiteral(entries) => {
                for e in entries {
                    self.walk_expr(&e.value().node);
                }
            }
            Expr::Macro { args, .. } => {
                for a in args {
                    self.walk_expr(&a.node);
                }
            }
            Expr::Await(inner) | Expr::Propagate(inner) => self.walk_expr(&inner.node),
            Expr::Closure { params, body, .. } => {
                self.scopes.push(BTreeSet::new());
                if let Some(scope) = self.scopes.last_mut() {
                    scope.extend(params.iter().map(|p| p.name.clone()));
                }
                self.walk_fnbody(body);
                self.scopes.pop();
            }
            Expr::Switch { scrutinee, cases, default } => {
                self.walk_expr(&scrutinee.node);
                for c in cases {
                    self.scopes.push(BTreeSet::new());
                    self.walk_pattern(&c.pattern);
                    if let Some(g) = &c.guard {
                        self.walk_expr(&g.node);
                    }
                    match &c.body {
                        SwitchExprBody::Expr(x) => self.walk_expr(&x.node),
                        SwitchExprBody::Block(b) => {
                            for st in &b.stmts {
                                self.walk_stmt(&st.node);
                            }
                        }
                    }
                    self.scopes.pop();
                }
                if let Some(d) = default {
                    match d {
                        SwitchExprBody::Expr(x) => self.walk_expr(&x.node),
                        SwitchExprBody::Block(b) => {
                            self.scopes.push(BTreeSet::new());
                            for st in &b.stmts {
                                self.walk_stmt(&st.node);
                            }
                            self.scopes.pop();
                        }
                    }
                }
            }
            Expr::UnsafeBlock(b) => self.walk_block(b),
            _ => {}
        }
    }
}

struct ClosureLifter {
    counter: usize,
    lifted: usize,
    new_decls: Vec<Spanned<Decl>>,
    globals: BTreeSet<String>,
}

impl ClosureLifter {
    fn lift_module(&mut self, module: &mut Module) {
        for decl in &mut module.decls {
            match &mut decl.node {
                Decl::Fn(f) => self.lift_fnbody(&mut f.body),
                Decl::Const { value, .. } => self.lift_expr(&mut value.node),
                Decl::Class { members, .. }
                | Decl::Struct { members, .. }
                | Decl::Trait { members, .. }
                | Decl::Interface { members, .. }
                | Decl::Extension { members, .. } => {
                    for m in members {
                        self.lift_member(&mut m.node);
                    }
                }
                _ => {}
            }
        }
    }

    fn lift_member(&mut self, m: &mut ClassMember) {
        match m {
            ClassMember::Method(f) => self.lift_fnbody(&mut f.body),
            ClassMember::Init { body, .. } | ClassMember::Deinit(body) => self.lift_block(body),
            _ => {}
        }
    }

    fn lift_fnbody(&mut self, body: &mut FnBody) {
        match body {
            FnBody::Block(b) => self.lift_block(b),
            FnBody::Expr(e) => self.lift_expr(&mut e.node),
        }
    }

    fn lift_block(&mut self, block: &mut Block) {
        for s in &mut block.stmts {
            self.lift_stmt(&mut s.node);
        }
    }

    fn lift_stmt(&mut self, s: &mut Stmt) {
        match s {
            Stmt::Var { value, .. } => self.lift_expr(&mut value.node),
            Stmt::DestructureTuple { value, .. }
            | Stmt::DestructureArray { value, .. }
            | Stmt::DestructureRecord { value, .. } => self.lift_expr(&mut value.node),
            Stmt::Assign { target, value, .. } => {
                self.lift_expr(&mut target.node);
                self.lift_expr(&mut value.node);
            }
            Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => {
                self.lift_expr(&mut e.node);
            }
            Stmt::If { cond, then, otherwise } => {
                match cond {
                    IfCond::Expr(e) | IfCond::Let { value: e, .. } => self.lift_expr(&mut e.node),
                }
                self.lift_block(then);
                if let Some(otherwise) = otherwise {
                    match otherwise {
                        Else::Block(b) => self.lift_block(b),
                        Else::If(st) => self.lift_stmt(&mut st.node),
                    }
                }
            }
            Stmt::For { iter, body, .. } => {
                self.lift_expr(&mut iter.node);
                self.lift_block(body);
            }
            Stmt::While { cond, body } => {
                self.lift_expr(&mut cond.node);
                self.lift_block(body);
            }
            Stmt::DoWhile { body, cond } => {
                self.lift_block(body);
                self.lift_expr(&mut cond.node);
            }
            Stmt::Switch { scrutinee, cases, default } => {
                self.lift_expr(&mut scrutinee.node);
                for c in cases {
                    if let Some(g) = &mut c.guard {
                        self.lift_expr(&mut g.node);
                    }
                    for st in &mut c.body {
                        self.lift_stmt(&mut st.node);
                    }
                }
                if let Some(d) = default {
                    for st in d.iter_mut() {
                        self.lift_stmt(&mut st.node);
                    }
                }
            }
            Stmt::Defer(b) | Stmt::UnsafeBlock(b) => self.lift_block(b),
            Stmt::Guard { value, otherwise, .. } => {
                self.lift_expr(&mut value.node);
                self.lift_block(otherwise);
            }
            Stmt::Try { body, catch, finally } => {
                self.lift_block(body);
                if let Some((_, b)) = catch {
                    self.lift_block(b);
                }
                if let Some(b) = finally {
                    self.lift_block(b);
                }
            }
            _ => {}
        }
    }

    fn lift_expr(&mut self, e: &mut Expr) {
        if matches!(e, Expr::Closure { is_async: true, .. }) {
            self.lift_one(e);
            return;
        }
        match e {
            Expr::Binary { lhs, rhs, .. } => {
                self.lift_expr(&mut lhs.node);
                self.lift_expr(&mut rhs.node);
            }
            Expr::Is { base, .. } => self.lift_expr(&mut base.node),
            Expr::Unary { rhs, .. } => self.lift_expr(&mut rhs.node),
            Expr::Postfix { expr, .. } => self.lift_expr(&mut expr.node),
            Expr::Ternary { cond, then, otherwise } => {
                self.lift_expr(&mut cond.node);
                self.lift_expr(&mut then.node);
                self.lift_expr(&mut otherwise.node);
            }
            Expr::Coalesce { lhs, rhs } => {
                self.lift_expr(&mut lhs.node);
                self.lift_expr(&mut rhs.node);
            }
            Expr::OptChain { base, .. } => self.lift_expr(&mut base.node),
            Expr::OptCall { base, args, .. } => {
                self.lift_expr(&mut base.node);
                for a in args {
                    self.lift_expr(&mut a.value.node);
                }
            }
            Expr::Range { lo, hi, .. } => {
                self.lift_expr(&mut lo.node);
                self.lift_expr(&mut hi.node);
            }
            Expr::Call { callee, args, trailing, .. } => {
                self.lift_expr(&mut callee.node);
                for a in args {
                    self.lift_expr(&mut a.value.node);
                }
                if let Some(b) = trailing {
                    self.lift_block(b);
                }
            }
            Expr::Index { base, index } => {
                self.lift_expr(&mut base.node);
                self.lift_expr(&mut index.node);
            }
            Expr::Member { base, .. } => self.lift_expr(&mut base.node),
            Expr::Interp(parts) => {
                for p in parts {
                    if let InterpPart::Expr(x) = p {
                        self.lift_expr(&mut x.node);
                    }
                }
            }
            Expr::Array(items) => {
                for i in items {
                    self.lift_expr(&mut i.expr.node);
                }
            }
            Expr::Tuple(items) => {
                for i in items {
                    self.lift_expr(&mut i.node);
                }
            }
            Expr::TupleGet { base, .. } => self.lift_expr(&mut base.node),
            Expr::Record(fields) => {
                for e in fields {
                    self.lift_expr(&mut e.value_mut().node);
                }
            }
            Expr::MapLiteral(entries) => {
                for e in entries {
                    self.lift_expr(&mut e.value_mut().node);
                }
            }
            Expr::Macro { args, .. } => {
                for a in args {
                    self.lift_expr(&mut a.node);
                }
            }
            Expr::Await(inner) | Expr::Propagate(inner) => self.lift_expr(&mut inner.node),
            Expr::Closure { body, .. } => match body {
                FnBody::Block(b) => self.lift_block(b),
                FnBody::Expr(x) => self.lift_expr(&mut x.node),
            },
            Expr::Switch { scrutinee, cases, default } => {
                self.lift_expr(&mut scrutinee.node);
                for c in cases {
                    if let Some(g) = &mut c.guard {
                        self.lift_expr(&mut g.node);
                    }
                    match &mut c.body {
                        SwitchExprBody::Expr(x) => self.lift_expr(&mut x.node),
                        SwitchExprBody::Block(b) => {
                            for st in &mut b.stmts {
                                self.lift_stmt(&mut st.node);
                            }
                        }
                    }
                }
                if let Some(d) = default {
                    match d {
                        SwitchExprBody::Expr(x) => self.lift_expr(&mut x.node),
                        SwitchExprBody::Block(b) => {
                            for st in &mut b.stmts {
                                self.lift_stmt(&mut st.node);
                            }
                        }
                    }
                }
            }
            Expr::UnsafeBlock(b) => self.lift_block(b),
            _ => {}
        }
    }

    fn lift_one(&mut self, slot: &mut Expr) {
        let (params, ret, body) = match std::mem::replace(slot, Expr::Null) {
            Expr::Closure { params, ret, body, .. } => (params, ret, body),
            other => {
                *slot = other;
                return;
            }
        };
        let mut fv = FreeVars { scopes: vec![BTreeSet::new()], used: BTreeSet::new() };
        if let Some(scope) = fv.scopes.last_mut() {
            scope.extend(params.iter().map(|p| p.name.clone()));
        }
        fv.walk_fnbody(&body);
        let caps: Vec<String> = fv
            .used
            .into_iter()
            .filter(|n| {
                !self.globals.contains(n)
                    && !LIFT_BUILTINS.contains(&n.as_str())
                    && !crate::prelude::provides(n)
            })
            .collect();
        let name = format!("__rnx_async_lam_{}", self.counter);
        self.counter += 1;
        let mut lam_params = params.clone();
        for c in &caps {
            lam_params.push(Param {
                name: c.clone(),
                span: Z,
                ty: None,
                default: None,
                promote: false,
            });
        }
        let fwd: Vec<Spanned<Expr>> = params
            .iter()
            .map(|p| ident(&p.name))
            .chain(caps.iter().map(|c| ident(c)))
            .collect();
        *slot = Expr::Closure {
            decay: false,
            is_async: false,
            params,
            ret: None,
            throws: false,
            body: FnBody::Expr(Box::new(call(ident(&name), fwd))),
        };
        self.new_decls.push(sp(
            Decl::Fn(FnDecl {
                docs: String::new(),
                access: Access::Internal,
                name,
                type_params: Vec::new(),
                params: lam_params,
                ret,
                throws: false,
                is_unsafe: false,
                is_async: true,
                is_static: false,
                is_test: false,
                is_bench: false,
                body,
                attrs: Vec::new(),
            }),
            Z,
        ));
        self.lifted += 1;
    }
}

fn lift_async_closures(module: &mut Module) {
    let mut counter = 0usize;
    loop {
        let globals = module_names(module);
        let mut lifter = ClosureLifter {
            counter,
            lifted: 0,
            new_decls: Vec::new(),
            globals,
        };
        lifter.lift_module(module);
        counter = lifter.counter;
        if lifter.lifted == 0 {
            break;
        }
        module.decls.append(&mut lifter.new_decls);
    }
}

fn hoist_await_returns(body: &mut Block) {
    let mut out: Vec<Spanned<Stmt>> = Vec::with_capacity(body.stmts.len());
    let mut n = 0usize;
    for s in std::mem::take(&mut body.stmts) {
        match s.node {
            Stmt::Return(Some(e)) if expr_has_await(&e.node) => {
                let name = format!("__ret{n}");
                n += 1;
                out.push(mk_stmt(Stmt::Var {
                    mutable: true,
                    is_static: false,
                    name: name.clone(),
                    ty: None,
                    value: e,
                }));
                out.push(mk_stmt(Stmt::Return(Some(ident(&name)))));
            }
            other => out.push(sp(other, s.span)),
        }
    }
    body.stmts = out;
}

pub fn check_await_outside_async(module: &Module, files: &DeclFiles) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    for decl in &module.decls {
        let from = out.len();
        let file = decl_file(files, decl.span);
        match &decl.node {
            Decl::Fn(f) if !f.is_async => {
                let mut found = Vec::new();
                match &f.body {
                    FnBody::Block(b) => {
                        if block_has_await(b) {
                            found.push(decl.span);
                        }
                    }
                    FnBody::Expr(e) => {
                        if expr_has_await(&e.node) {
                            found.push(e.span);
                        }
                    }
                }
                for span in found {
                    out.push(
                        Diagnostic::new(Code::E109, "await is only valid inside async functions")
                            .with_span(span),
                    );
                }
            }
            Decl::Class { members, .. }
            | Decl::Struct { members, .. }
            | Decl::Trait { members, .. } => {
                for m in members {
                    match &m.node {
                        ClassMember::Method(f) if !f.is_async => {
                            let hit = match &f.body {
                                FnBody::Block(b) => block_has_await(b),
                                FnBody::Expr(e) => expr_has_await(&e.node),
                            };
                            if hit {
                                out.push(
                                    Diagnostic::new(
                                        Code::E109,
                                        "await is only valid inside async functions",
                                    )
                                    .with_span(m.span),
                                );
                            }
                        }
                        ClassMember::Init { body, .. } | ClassMember::Deinit(body) => {
                            if block_has_await(body) {
                                out.push(
                                    Diagnostic::new(
                                        Code::E109,
                                        "await is only valid inside async functions",
                                    )
                                    .with_span(m.span),
                                );
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
        tag_new(&mut out, from, file);
    }
    out
}

fn expr_has_sync_wait(e: &Expr) -> bool {
    match e {
        Expr::Call { callee, args, trailing, .. } => {
            if args.is_empty() && trailing.is_none() {
                if let Expr::Member { field, .. } = &callee.node {
                    if field == "wait" {
                        return true;
                    }
                }
            }
            expr_has_sync_wait(&callee.node)
                || args.iter().any(|a| expr_has_sync_wait(&a.value.node))
                || trailing.as_ref().map(block_has_sync_wait).unwrap_or(false)
        }
        Expr::Closure { is_async: false, .. } => false,
        Expr::Closure { body, .. } => match body {
            FnBody::Block(b) => block_has_sync_wait(b),
            FnBody::Expr(x) => expr_has_sync_wait(&x.node),
        },
        Expr::Binary { lhs, rhs, .. } => {
            expr_has_sync_wait(&lhs.node) || expr_has_sync_wait(&rhs.node)
        }
        Expr::Is { base, .. } => expr_has_sync_wait(&base.node),
        Expr::Unary { rhs, .. } => expr_has_sync_wait(&rhs.node),
        Expr::Postfix { expr, .. } => expr_has_sync_wait(&expr.node),
        Expr::Ternary { cond, then, otherwise } => {
            expr_has_sync_wait(&cond.node)
                || expr_has_sync_wait(&then.node)
                || expr_has_sync_wait(&otherwise.node)
        }
        Expr::Coalesce { lhs, rhs } => {
            expr_has_sync_wait(&lhs.node) || expr_has_sync_wait(&rhs.node)
        }
        Expr::OptChain { base, .. } => expr_has_sync_wait(&base.node),
        Expr::OptCall { base, args, .. } => {
            expr_has_sync_wait(&base.node)
                || args.iter().any(|a| expr_has_sync_wait(&a.value.node))
        }
        Expr::Range { lo, hi, .. } => {
            expr_has_sync_wait(&lo.node) || expr_has_sync_wait(&hi.node)
        }
        Expr::Index { base, index } => {
            expr_has_sync_wait(&base.node) || expr_has_sync_wait(&index.node)
        }
        Expr::Member { base, .. } => expr_has_sync_wait(&base.node),
        Expr::Interp(parts) => parts.iter().any(|x| match x {
            InterpPart::Expr(x) => expr_has_sync_wait(&x.node),
            _ => false,
        }),
        Expr::Array(items) => items.iter().any(|i| expr_has_sync_wait(&i.expr.node)),
        Expr::Record(fields) => fields.iter().any(|e| expr_has_sync_wait(&e.value().node)),
        Expr::MapLiteral(entries) => entries.iter().any(|e| expr_has_sync_wait(&e.value().node)),
        Expr::Macro { args, .. } => args.iter().any(|a| expr_has_sync_wait(&a.node)),
        Expr::UnsafeBlock(b) => block_has_sync_wait(b),
        Expr::Await(inner) | Expr::Propagate(inner) => expr_has_sync_wait(&inner.node),
        Expr::Tuple(items) => items.iter().any(|i| expr_has_sync_wait(&i.node)),
        Expr::TupleGet { base, .. } => expr_has_sync_wait(&base.node),
        Expr::Switch { scrutinee, cases, default } => {
            expr_has_sync_wait(&scrutinee.node)
                || cases.iter().any(|c| {
                    c.guard.as_ref().map(|g| expr_has_sync_wait(&g.node)).unwrap_or(false)
                        || match &c.body {
                            SwitchExprBody::Expr(e) => expr_has_sync_wait(&e.node),
                            SwitchExprBody::Block(b) => block_has_sync_wait(b),
                        }
                })
                || default.as_ref().map(|d| match d {
                    SwitchExprBody::Expr(e) => expr_has_sync_wait(&e.node),
                    SwitchExprBody::Block(b) => block_has_sync_wait(b),
                }).unwrap_or(false)
        }
        _ => false,
    }
}

fn block_has_sync_wait(b: &Block) -> bool {
    b.stmts.iter().any(|s| stmt_has_sync_wait(&s.node))
}

fn stmt_has_sync_wait(s: &Stmt) -> bool {
    match s {
        Stmt::Var { value, .. } => expr_has_sync_wait(&value.node),
        Stmt::Assign { target, value, .. } => {
            expr_has_sync_wait(&target.node) || expr_has_sync_wait(&value.node)
        }
        Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => {
            expr_has_sync_wait(&e.node)
        }
        Stmt::If { cond, then, otherwise } => {
            let c = match cond {
                IfCond::Expr(e) => expr_has_sync_wait(&e.node),
                IfCond::Let { value, .. } => expr_has_sync_wait(&value.node),
            };
            c || block_has_sync_wait(then)
                || otherwise.as_ref().map(else_has_sync_wait).unwrap_or(false)
        }
        Stmt::While { cond, body } | Stmt::DoWhile { cond, body } => {
            expr_has_sync_wait(&cond.node) || block_has_sync_wait(body)
        }
        Stmt::For { iter, body, .. } => {
            expr_has_sync_wait(&iter.node) || block_has_sync_wait(body)
        }
        Stmt::Switch { scrutinee, cases, default } => {
            expr_has_sync_wait(&scrutinee.node)
                || cases.iter().any(|c| {
                    c.guard.as_ref().map(|g| expr_has_sync_wait(&g.node)).unwrap_or(false)
                        || c.body.iter().any(|x| stmt_has_sync_wait(&x.node))
                })
                || default.as_ref().map(|d| d.iter().any(|x| stmt_has_sync_wait(&x.node))).unwrap_or(false)
        }
        Stmt::Defer(b) | Stmt::UnsafeBlock(b) => block_has_sync_wait(b),
        Stmt::Guard { value, otherwise, .. } => {
            expr_has_sync_wait(&value.node) || block_has_sync_wait(otherwise)
        }
        Stmt::Try { body, catch, finally } => {
            block_has_sync_wait(body)
                || catch.as_ref().map(|(_, b)| block_has_sync_wait(b)).unwrap_or(false)
                || finally.as_ref().map(block_has_sync_wait).unwrap_or(false)
        }
        _ => false,
    }
}

fn else_has_sync_wait(e: &Else) -> bool {
    match e {
        Else::Block(b) => block_has_sync_wait(b),
        Else::If(s) => stmt_has_sync_wait(&s.node),
    }
}

fn body_has_sync_wait(body: &FnBody) -> bool {
    match body {
        FnBody::Block(b) => block_has_sync_wait(b),
        FnBody::Expr(e) => expr_has_sync_wait(&e.node),
    }
}

pub fn check_wait_inside_async(module: &Module, files: &DeclFiles) -> Vec<Diagnostic> {
    let mut out = Vec::new();
    let mut hit = |span: Span, current: &Option<std::path::PathBuf>, out: &mut Vec<Diagnostic>| {
        let mut d = Diagnostic::new(
            Code::E108,
            "Synchronous wait() cannot be invoked from an async function; use 'await' instead",
        )
        .with_span(span);
        if let Some(f) = current.clone()
            && d.file.is_none()
        {
            d.file = Some(f);
        }
        out.push(d);
    };
    for decl in &module.decls {
        let current = decl_file(files, decl.span);
        match &decl.node {
            Decl::Fn(f) if f.is_async => {
                if body_has_sync_wait(&f.body) {
                    hit(decl.span, &current, &mut out);
                }
            }
            Decl::Class { members, .. }
            | Decl::Struct { members, .. }
            | Decl::Trait { members, .. } => {
                for m in members {
                    match &m.node {
                        ClassMember::Method(f) if f.is_async => {
                            if body_has_sync_wait(&f.body) {
                                hit(m.span, &current, &mut out);
                            }
                        }
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    out
}

