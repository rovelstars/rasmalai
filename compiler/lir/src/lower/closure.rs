use super::*;

pub(super) fn infer_closure_ret(
    body: &A::FnBody,
    params: &[A::Param],
    hints: &[LirType],
    resolve: &dyn Fn(&str) -> Option<LirType>,
) -> Option<LirType> {
    match body {
        A::FnBody::Block(_) => None,
        A::FnBody::Expr(e) => infer_simple(&e.node, params, hints, resolve),
    }
}

pub(super) fn infer_simple(
    e: &A::Expr,
    params: &[A::Param],
    hints: &[LirType],
    resolve: &dyn Fn(&str) -> Option<LirType>,
) -> Option<LirType> {
    match e {
        A::Expr::Int(_) => Some(LirType::I64),
        A::Expr::Float(_) => Some(LirType::F64(FloatKind::Strict)),
        A::Expr::Bool(_) => Some(LirType::Bool),
        A::Expr::Interp(_) => Some(LirType::Str),
        A::Expr::Null => None,
        A::Expr::Array(_) => Some(LirType::Array(Box::new(LirType::Any))),
        A::Expr::Ident(n) => params
            .iter()
            .position(|p| &p.name == n)
            .and_then(|i| {
                params[i]
                    .ty
                    .as_ref()
                    .map(|t| map_ty(t))
                    .or_else(|| hints.get(i).cloned())
            })
            .filter(|t| *t != LirType::Any)
            .or_else(|| resolve(n)),
        A::Expr::Binary { op, lhs, rhs } => match op {
            A::BinOp::Eq
            | A::BinOp::NotEq
            | A::BinOp::Lt
            | A::BinOp::LtEq
            | A::BinOp::Gt
            | A::BinOp::GtEq
            | A::BinOp::And
            | A::BinOp::Or => Some(LirType::Bool),
            _ => match (
                infer_simple(&lhs.node, params, hints, resolve),
                infer_simple(&rhs.node, params, hints, resolve),
            ) {
                (Some(LirType::F64(_)), _) | (_, Some(LirType::F64(_))) => {
                    Some(LirType::F64(FloatKind::Strict))
                }
                (Some(LirType::I64), Some(LirType::I64)) => Some(LirType::I64),
                (Some(LirType::Bool), Some(LirType::Bool)) => Some(LirType::Bool),
                (Some(LirType::Str), Some(LirType::Str)) => Some(LirType::Str),
                _ => None,
            },
        },
        A::Expr::Unary { rhs, .. } => infer_simple(&rhs.node, params, hints, resolve),
        A::Expr::Postfix { expr, .. } => infer_simple(&expr.node, params, hints, resolve),
        A::Expr::Ternary { then, .. } => infer_simple(&then.node, params, hints, resolve),
        A::Expr::Cast { ty, .. } => Some(map_ty(ty)),
        A::Expr::Call { callee, .. } => match &callee.node {
            A::Expr::Member { base, field } => {
                let base_ty = infer_simple(&base.node, params, hints, resolve)?;
                if matches!(base_ty, LirType::I64 | LirType::F64(_) | LirType::Bool)
                    && field.as_str() == "toString"
                {
                    return Some(LirType::Str);
                }
                let want = match base_ty {
                    LirType::Str => "String",
                    LirType::Array(_) => "Array",
                    _ => return None,
                };
                match (want, field.as_str()) {
                    ("String", "length") | ("String", "len")
                    | ("String", "indexOf") | ("String", "charCodeAt") => Some(LirType::I64),
                    ("String", "trim") | ("String", "concat") | ("String", "slice") => {
                        Some(LirType::Str)
                    }
                    ("Array", "length") | ("Array", "len") => Some(LirType::I64),
                    _ => None,
                }
            }
            _ => None,
        },
        _ => None,
    }
}

pub(super) fn closure_free_vars(body: &A::FnBody, params: &[A::Param]) -> BTreeSet<String> {    let mut bound: BTreeSet<String> = params.iter().map(|p| p.name.clone()).collect();
    let mut used = BTreeSet::new();
    for p in params {
        if let Some(d) = p.default.as_ref() {
            free_expr(&d.node, &bound, &mut used);
        }
    }
    match body {
        A::FnBody::Block(b) => free_block(b, &mut bound, &mut used),
        A::FnBody::Expr(e) => free_expr(&e.node, &bound, &mut used),
    }
    for p in params {
        used.remove(&p.name);
    }
    used
}

pub(super) fn free_block(b: &A::Block, bound: &mut BTreeSet<String>, used: &mut BTreeSet<String>) {
    for s in &b.stmts {
        free_stmt(&s.node, bound, used);
    }
}

pub(super) fn free_else(e: &A::Else, bound: &mut BTreeSet<String>, used: &mut BTreeSet<String>) {
    match e {
        A::Else::Block(b) => {
            let mut inner = bound.clone();
            free_block(b, &mut inner, used);
        }
        A::Else::If(s) => free_stmt(&s.node, bound, used),
    }
}

pub(super) fn free_pattern_binds(pat: &A::Pattern, out: &mut Vec<String>) {
    match pat {
        A::Pattern::Literal(e) => {
            if let A::Expr::Ident(n) = &e.node {
                out.push(n.clone());
            }
        }
        A::Pattern::Range { .. } => {}
        A::Pattern::Is(_) | A::Pattern::Wildcard => {}
        A::Pattern::Enum { args, .. } => {
            for a in args {
                free_pattern_binds(a, out);
            }
        }
    }
}

pub(super) fn free_pattern_uses(pat: &A::Pattern, bound: &BTreeSet<String>, used: &mut BTreeSet<String>) {
    match pat {
        A::Pattern::Literal(e) => {
            if !matches!(&e.node, A::Expr::Ident(_)) {
                free_expr(&e.node, bound, used);
            }
        }
        A::Pattern::Range { lo, hi, .. } => {
            free_expr(&lo.node, bound, used);
            free_expr(&hi.node, bound, used);
        }
        A::Pattern::Enum { args, .. } => {
            for a in args {
                free_pattern_uses(a, bound, used);
            }
        }
        A::Pattern::Is(_) | A::Pattern::Wildcard => {}
    }
}

pub(super) fn free_stmt(s: &A::Stmt, bound: &mut BTreeSet<String>, used: &mut BTreeSet<String>) {
    match s {
        A::Stmt::Var { name, value, .. } => {
            free_expr(&value.node, bound, used);
            bound.insert(name.clone());
        }
        A::Stmt::DestructureTuple { names, value, .. } => {
            free_expr(&value.node, bound, used);
            for n in names {
                bound.insert(n.clone());
            }
        }
        A::Stmt::DestructureRecord { fields, rest, value, .. } => {
            free_expr(&value.node, bound, used);
            for (_, local) in fields {
                bound.insert(local.clone());
            }
            if let Some(r) = rest {
                bound.insert(r.clone());
            }
        }
        A::Stmt::DestructureArray { names, rest, value, .. } => {
            free_expr(&value.node, bound, used);
            for n in names.iter().chain(rest.iter()) {
                bound.insert(n.clone());
            }
        }
        A::Stmt::Assign { target, value, .. } => {
            free_expr(&target.node, bound, used);
            free_expr(&value.node, bound, used);
        }
        A::Stmt::Expr(e) | A::Stmt::Assert(e) => free_expr(&e.node, bound, used),
        A::Stmt::If { cond, then, otherwise } => {
            match cond {
                A::IfCond::Expr(e) => free_expr(&e.node, bound, used),
                A::IfCond::Let { name, value } => {
                    free_expr(&value.node, bound, used);
                    let mut inner = bound.clone();
                    inner.insert(name.clone());
                    free_block(then, &mut inner, used);
                    if let Some(el) = otherwise {
                        free_else(el, bound, used);
                    }
                    return;
                }
            }
            let mut inner = bound.clone();
            free_block(then, &mut inner, used);
            if let Some(el) = otherwise {
                free_else(el, bound, used);
            }
        }
        A::Stmt::For { binding, iter, body } => {
            free_expr(&iter.node, bound, used);
            let mut inner = bound.clone();
            match binding {
                A::ForBinding::One(n) => {
                    inner.insert(n.clone());
                }
                A::ForBinding::Many(ns) => {
                    for n in ns {
                        inner.insert(n.clone());
                    }
                }
            }
            free_block(body, &mut inner, used);
        }
        A::Stmt::While { cond, body } => {
            free_expr(&cond.node, bound, used);
            let mut inner = bound.clone();
            free_block(body, &mut inner, used);
        }
        A::Stmt::DoWhile { body, cond } => {
            let mut inner = bound.clone();
            free_block(body, &mut inner, used);
            free_expr(&cond.node, bound, used);
        }
        A::Stmt::Switch { scrutinee, cases, default } => {
            free_expr(&scrutinee.node, bound, used);
            for c in cases {
                let mut binds = Vec::new();
                free_pattern_binds(&c.pattern, &mut binds);
                free_pattern_uses(&c.pattern, bound, used);
                let mut inner = bound.clone();
                for b in binds {
                    inner.insert(b);
                }
                if let Some(g) = c.guard.as_ref() {
                    free_expr(&g.node, &inner, used);
                }
                for st in &c.body {
                    free_stmt(&st.node, &mut inner, used);
                }
            }
            if let Some(stmts) = default {
                let mut inner = bound.clone();
                for st in stmts {
                    free_stmt(&st.node, &mut inner, used);
                }
            }
        }
        A::Stmt::Return(e) => {
            if let Some(e) = e {
                free_expr(&e.node, bound, used);
            }
        }
        A::Stmt::Defer(b) | A::Stmt::UnsafeBlock(b) => {
            let mut inner = bound.clone();
            free_block(b, &mut inner, used);
        }
        A::Stmt::Guard { name, value, otherwise } => {
            free_expr(&value.node, bound, used);
            let mut inner = bound.clone();
            inner.insert(name.clone());
            free_block(otherwise, &mut inner, used);
            bound.insert(name.clone());
        }
        A::Stmt::Try { body, catch, finally } => {
            let mut inner = bound.clone();
            free_block(body, &mut inner, used);
            if let Some((name, block)) = catch {
                let mut inner = bound.clone();
                inner.insert(name.clone());
                free_block(block, &mut inner, used);
            }
            if let Some(b) = finally {
                let mut inner = bound.clone();
                free_block(b, &mut inner, used);
            }
        }
        A::Stmt::Throw(e) => {
            if let Some(e) = e {
                free_expr(&e.node, bound, used);
            }
        }
        A::Stmt::Break | A::Stmt::Continue | A::Stmt::Fallthrough | A::Stmt::Pass | A::Stmt::Empty => {}
    }
}

pub(super) fn free_expr(e: &A::Expr, bound: &BTreeSet<String>, used: &mut BTreeSet<String>) {
    match e {
        A::Expr::Is { base, .. } => {
            free_expr(&base.node, bound, used);
            return;
        }
        A::Expr::Ident(n) => {
            if !bound.contains(n) {
                used.insert(n.clone());
            }
        }
        A::Expr::This => {}
        A::Expr::Super => {}
        A::Expr::Bool(_) | A::Expr::Null | A::Expr::Int(_) | A::Expr::Float(_) => {}
        A::Expr::Interp(parts) => {
            for p in parts {
                if let A::InterpPart::Expr(inner) = p {
                    free_expr(&inner.node, bound, used);
                }
            }
        }
        A::Expr::Array(items) => {
            for it in items {
                free_expr(&it.expr.node, bound, used);
            }
        }
        A::Expr::Record(fields) => {
            for e in fields {
                free_expr(&e.value().node, bound, used);
            }
        }
        A::Expr::MapLiteral(entries) => {
            for e in entries {
                free_expr(&e.value().node, bound, used);
            }
        }
        A::Expr::Binary { lhs, rhs, .. } => {
            free_expr(&lhs.node, bound, used);
            free_expr(&rhs.node, bound, used);
        }
        A::Expr::Unary { rhs, .. } => free_expr(&rhs.node, bound, used),
        A::Expr::Postfix { expr, .. } => free_expr(&expr.node, bound, used),
        A::Expr::Await(inner) => free_expr(&inner.node, bound, used),
        A::Expr::Propagate(inner) => free_expr(&inner.node, bound, used),
        A::Expr::Tuple(items) => {
            for it in items {
                free_expr(&it.node, bound, used);
            }
        }
        A::Expr::TupleGet { base, .. } => free_expr(&base.node, bound, used),
        A::Expr::Switch { scrutinee, cases, default } => {
            free_expr(&scrutinee.node, bound, used);
            for c in cases {
                if let Some(g) = &c.guard {
                    free_expr(&g.node, bound, used);
                }
                match &c.body {
                    A::SwitchExprBody::Expr(e) => free_expr(&e.node, bound, used),
                    A::SwitchExprBody::Block(b) => {
                        let mut inner = bound.clone();
                        free_block(b, &mut inner, used);
                    },
                }
            }
            if let Some(d) = default {
                match d {
                    A::SwitchExprBody::Expr(e) => free_expr(&e.node, bound, used),
                    A::SwitchExprBody::Block(b) => {
                        let mut inner = bound.clone();
                        free_block(b, &mut inner, used);
                    },
                }
            }
        },
        A::Expr::Ternary { cond, then, otherwise } => {
            free_expr(&cond.node, bound, used);
            free_expr(&then.node, bound, used);
            free_expr(&otherwise.node, bound, used);
        }
        A::Expr::Range { lo, hi, .. } => {
            free_expr(&lo.node, bound, used);
            free_expr(&hi.node, bound, used);
        }
        A::Expr::Call { callee, args, trailing, .. } => {
            free_expr(&callee.node, bound, used);
            for a in args {
                free_expr(&a.value.node, bound, used);
            }
            if let Some(b) = trailing {
                let mut inner = bound.clone();
                free_block(b, &mut inner, used);
            }
        }
        A::Expr::New { args, .. } => {
            for a in args {
                free_expr(&a.value.node, bound, used);
            }
        }
        A::Expr::Index { base, index } => {
            free_expr(&base.node, bound, used);
            free_expr(&index.node, bound, used);
        }
        A::Expr::Member { base, .. } => free_expr(&base.node, bound, used),
        A::Expr::Coalesce { lhs, rhs } => {
            free_expr(&lhs.node, bound, used);
            free_expr(&rhs.node, bound, used);
        }
        A::Expr::OptChain { base, .. } => free_expr(&base.node, bound, used),
        A::Expr::OptCall { base, args, .. } => {
            free_expr(&base.node, bound, used);
            for a in args {
                free_expr(&a.value.node, bound, used);
            }
        }
        A::Expr::Macro { args, .. } => {
            for a in args {
                free_expr(&a.node, bound, used);
            }
        }
        A::Expr::Closure { params, body, .. } => {
            let mut inner_bound = bound.clone();
            for p in params {
                if let Some(d) = p.default.as_ref() {
                    free_expr(&d.node, &inner_bound, used);
                }
            }
            for p in params {
                inner_bound.insert(p.name.clone());
            }
            match body {
                A::FnBody::Block(b) => free_block(b, &mut inner_bound, used),
                A::FnBody::Expr(x) => free_expr(&x.node, &inner_bound, used),
            }
        }
        A::Expr::UnsafeBlock(b) => {
            let mut inner = bound.clone();
            free_block(b, &mut inner, used);
        }
        A::Expr::Cast { expr, .. } => free_expr(&expr.node, bound, used),
        A::Expr::ImplicitMember(_) => {}
    }
}

pub(super) fn body_uses_this(body: &A::FnBody) -> bool {
    match body {
        A::FnBody::Block(b) => b.stmts.iter().any(|s| stmt_uses_this(&s.node)),
        A::FnBody::Expr(e) => expr_uses_this(&e.node),
    }
}

pub(super) fn stmt_uses_this(s: &A::Stmt) -> bool {
    match s {
        A::Stmt::Pass | A::Stmt::Empty => false,
        A::Stmt::Var { value, .. } => expr_uses_this(&value.node),
        A::Stmt::DestructureTuple { value, .. }
        | A::Stmt::DestructureRecord { value, .. }
        | A::Stmt::DestructureArray { value, .. } => expr_uses_this(&value.node),
        A::Stmt::Assign { target, value, .. } => {
            expr_uses_this(&target.node) || expr_uses_this(&value.node)
        }
        A::Stmt::Expr(e) | A::Stmt::Assert(e) => expr_uses_this(&e.node),
        A::Stmt::Return(Some(e)) | A::Stmt::Throw(Some(e)) => expr_uses_this(&e.node),
        A::Stmt::Return(None)
        | A::Stmt::Throw(None)
        | A::Stmt::Break
        | A::Stmt::Continue
        | A::Stmt::Fallthrough => false,
        A::Stmt::If { cond, then, .. } => {
            let c = match cond {
                A::IfCond::Expr(e) | A::IfCond::Let { value: e, .. } => expr_uses_this(&e.node),
            };
            c || then.stmts.iter().any(|x| stmt_uses_this(&x.node))
        }
        A::Stmt::While { cond, body } => {
            expr_uses_this(&cond.node) || body.stmts.iter().any(|x| stmt_uses_this(&x.node))
        }
        A::Stmt::DoWhile { body, cond } => {
            body.stmts.iter().any(|x| stmt_uses_this(&x.node)) || expr_uses_this(&cond.node)
        }
        A::Stmt::For { iter, body, .. } => {
            expr_uses_this(&iter.node) || body.stmts.iter().any(|x| stmt_uses_this(&x.node))
        }
        A::Stmt::Switch { scrutinee, .. } => expr_uses_this(&scrutinee.node),
        A::Stmt::Defer(b)
        | A::Stmt::UnsafeBlock(b)
        | A::Stmt::Guard { otherwise: b, .. } => {
            b.stmts.iter().any(|x| stmt_uses_this(&x.node))
        }
        A::Stmt::Try { body, .. } => body.stmts.iter().any(|x| stmt_uses_this(&x.node)),
    }
}

pub(super) fn body_uses_super(body: &A::FnBody) -> bool {
    match body {
        A::FnBody::Block(b) => b.stmts.iter().any(|s| stmt_uses_super(&s.node)),
        A::FnBody::Expr(e) => expr_uses_super(&e.node),
    }
}

pub(super) fn stmt_uses_super(s: &A::Stmt) -> bool {
    match s {
        A::Stmt::Pass | A::Stmt::Empty => false,
        A::Stmt::Var { value, .. } => expr_uses_super(&value.node),
        A::Stmt::DestructureTuple { value, .. }
        | A::Stmt::DestructureRecord { value, .. }
        | A::Stmt::DestructureArray { value, .. } => expr_uses_super(&value.node),
        A::Stmt::Assign { target, value, .. } => {
            expr_uses_super(&target.node) || expr_uses_super(&value.node)
        }
        A::Stmt::Expr(e) | A::Stmt::Assert(e) => expr_uses_super(&e.node),
        A::Stmt::Return(Some(e)) | A::Stmt::Throw(Some(e)) => expr_uses_super(&e.node),
        A::Stmt::Return(None)
        | A::Stmt::Throw(None)
        | A::Stmt::Break
        | A::Stmt::Continue
        | A::Stmt::Fallthrough => false,
        A::Stmt::If { cond, then, .. } => {
            let c = match cond {
                A::IfCond::Expr(e) | A::IfCond::Let { value: e, .. } => expr_uses_super(&e.node),
            };
            c || then.stmts.iter().any(|x| stmt_uses_super(&x.node))
        }
        A::Stmt::While { cond, body } => {
            expr_uses_super(&cond.node) || body.stmts.iter().any(|x| stmt_uses_super(&x.node))
        }
        A::Stmt::DoWhile { body, cond } => {
            body.stmts.iter().any(|x| stmt_uses_super(&x.node)) || expr_uses_super(&cond.node)
        }
        A::Stmt::For { iter, body, .. } => {
            expr_uses_super(&iter.node) || body.stmts.iter().any(|x| stmt_uses_super(&x.node))
        }
        A::Stmt::Switch { scrutinee, .. } => expr_uses_super(&scrutinee.node),
        A::Stmt::Defer(b)
        | A::Stmt::UnsafeBlock(b)
        | A::Stmt::Guard { otherwise: b, .. } => {
            b.stmts.iter().any(|x| stmt_uses_super(&x.node))
        }
        A::Stmt::Try { body, .. } => body.stmts.iter().any(|x| stmt_uses_super(&x.node)),
    }
}

pub(super) fn expr_uses_super(e: &A::Expr) -> bool {
    match e {
        A::Expr::Super => true,
        A::Expr::Binary { lhs, rhs, .. } => {
            expr_uses_super(&lhs.node) || expr_uses_super(&rhs.node)
        }
        A::Expr::Unary { rhs, .. } => expr_uses_super(&rhs.node),
        A::Expr::Postfix { expr, .. } => expr_uses_super(&expr.node),
        A::Expr::Await(e) => expr_uses_super(&e.node),
        A::Expr::Tuple(items) => items.iter().any(|i| expr_uses_super(&i.node)),
        A::Expr::TupleGet { base, .. } => expr_uses_super(&base.node),
        A::Expr::Switch { scrutinee, cases, default } => {
            let mut out = expr_uses_super(&scrutinee.node);
            for c in cases {
                if let Some(g) = &c.guard {
                    out = out || expr_uses_super(&g.node);
                }
                out = out
                    || match &c.body {
                        A::SwitchExprBody::Expr(e) => expr_uses_super(&e.node),
                        A::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_uses_super(&s.node))
                        }
                    };
            }
            if let Some(d) = default {
                out = out
                    || match d {
                        A::SwitchExprBody::Expr(e) => expr_uses_super(&e.node),
                        A::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_uses_super(&s.node))
                        }
                    };
            }
            out
        }
        A::Expr::Ternary { cond, then, otherwise } => {
            expr_uses_super(&cond.node)
                || expr_uses_super(&then.node)
                || expr_uses_super(&otherwise.node)
        }
        A::Expr::Range { lo, hi, .. } => expr_uses_super(&lo.node) || expr_uses_super(&hi.node),
        A::Expr::Call { callee, args, .. } => {
            expr_uses_super(&callee.node)
                || args.iter().any(|a| expr_uses_super(&a.value.node))
        }
        A::Expr::Index { base, index } => {
            expr_uses_super(&base.node) || expr_uses_super(&index.node)
        }
        A::Expr::Member { base, .. } => expr_uses_super(&base.node),
        A::Expr::Cast { expr, .. } => expr_uses_super(&expr.node),
        A::Expr::Interp(parts) => parts.iter().any(|p| match p {
            A::InterpPart::Expr(x) => expr_uses_super(&x.node),
            _ => false,
        }),
        A::Expr::Array(items) => items.iter().any(|i| expr_uses_super(&i.expr.node)),
        A::Expr::Record(fields) => fields.iter().any(|e| expr_uses_super(&e.value().node)),
        A::Expr::MapLiteral(entries) => entries.iter().any(|e| expr_uses_super(&e.value().node)),
        A::Expr::Macro { args, .. } => args.iter().any(|a| expr_uses_super(&a.node)),
        _ => false,
    }
}

pub(super) fn expr_uses_this(e: &A::Expr) -> bool {
    match e {
        A::Expr::This => true,
        A::Expr::Binary { lhs, rhs, .. } => {
            expr_uses_this(&lhs.node) || expr_uses_this(&rhs.node)
        }
        A::Expr::Unary { rhs, .. } => expr_uses_this(&rhs.node),
        A::Expr::Postfix { expr, .. } => expr_uses_this(&expr.node),
        A::Expr::Await(e) => expr_uses_this(&e.node),
        A::Expr::Tuple(items) => items.iter().any(|i| expr_uses_this(&i.node)),
        A::Expr::TupleGet { base, .. } => expr_uses_this(&base.node),
        A::Expr::Switch { scrutinee, cases, default } => {
            let mut out = expr_uses_this(&scrutinee.node);
            for c in cases {
                if let Some(g) = &c.guard {
                    out = out || expr_uses_this(&g.node);
                }
                out = out
                    || match &c.body {
                        A::SwitchExprBody::Expr(e) => expr_uses_this(&e.node),
                        A::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_uses_this(&s.node))
                        }
                    };
            }
            if let Some(d) = default {
                out = out
                    || match d {
                        A::SwitchExprBody::Expr(e) => expr_uses_this(&e.node),
                        A::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_uses_this(&s.node))
                        }
                    };
            }
            out
        }
        A::Expr::Ternary { cond, then, otherwise } => {
            expr_uses_this(&cond.node)
                || expr_uses_this(&then.node)
                || expr_uses_this(&otherwise.node)
        }
        A::Expr::Range { lo, hi, .. } => expr_uses_this(&lo.node) || expr_uses_this(&hi.node),
        A::Expr::Call { callee, args, .. } => {
            expr_uses_this(&callee.node)
                || args.iter().any(|a| expr_uses_this(&a.value.node))
        }
        A::Expr::Index { base, index } => {
            expr_uses_this(&base.node) || expr_uses_this(&index.node)
        }
        A::Expr::Member { base, .. } => expr_uses_this(&base.node),
        A::Expr::Cast { expr, .. } => expr_uses_this(&expr.node),
        A::Expr::Interp(parts) => parts.iter().any(|p| match p {
            A::InterpPart::Expr(x) => expr_uses_this(&x.node),
            _ => false,
        }),
        A::Expr::Array(items) => items.iter().any(|i| expr_uses_this(&i.expr.node)),
        A::Expr::Record(fields) => fields.iter().any(|e| expr_uses_this(&e.value().node)),
        A::Expr::MapLiteral(entries) => entries.iter().any(|e| expr_uses_this(&e.value().node)),
        A::Expr::Macro { args, .. } => args.iter().any(|a| expr_uses_this(&a.node)),
        _ => false,
    }
}
