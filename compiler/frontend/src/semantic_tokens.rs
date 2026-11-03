use crate::ast::*;
use diagnostics::Span;

pub fn parameter_spans(module: &Module) -> Vec<Span> {
    let mut out = Vec::new();
    for decl in &module.decls {
        visit_decl(&decl.node, &mut out);
    }
    out.sort_by_key(|s| (s.start, s.end));
    out.dedup_by_key(|s| (s.start, s.end));
    out
}

fn push_params(out: &mut Vec<Span>, params: &[Param]) {
    for p in params {
        out.push(p.span);
    }
}

fn visit_fn(out: &mut Vec<Span>, f: &FnDecl) {
    push_params(out, &f.params);
    visit_fn_body(out, &f.body);
}

fn visit_fn_body(out: &mut Vec<Span>, body: &FnBody) {
    match body {
        FnBody::Block(b) => visit_block(out, b),
        FnBody::Expr(e) => visit_expr(out, &e.node),
    }
}

fn visit_block(out: &mut Vec<Span>, block: &Block) {
    for stmt in &block.stmts {
        visit_stmt(out, &stmt.node);
    }
}

fn visit_stmts(out: &mut Vec<Span>, stmts: &[Spanned<Stmt>]) {
    for stmt in stmts {
        visit_stmt(out, &stmt.node);
    }
}

fn visit_exprs(out: &mut Vec<Span>, exprs: &[Spanned<Expr>]) {
    for expr in exprs {
        visit_expr(out, &expr.node);
    }
}

fn visit_decl(decl: &Decl, out: &mut Vec<Span>) {
    match decl {
        Decl::Fn(f) => visit_fn(out, f),
        Decl::Class { members, .. }
        | Decl::Struct { members, .. }
        | Decl::Trait { members, .. } => {
            for member in members {
                match &member.node {
                    ClassMember::Method(f) => visit_fn(out, f),
                    ClassMember::Init { params, body } => {
                        push_params(out, params);
                        visit_block(out, body);
                    }
                    ClassMember::OnReload { params, body } => {
                        push_params(out, params);
                        visit_block(out, body);
                    }
                    ClassMember::Field(f) => {
                        if let Some(value) = &f.value {
                            visit_expr(out, &value.node);
                        }
                    }
                    _ => {}
                }
            }
        }
        Decl::Const { value, .. } => visit_expr(out, &value.node),
        _ => {}
    }
}

fn visit_stmt(out: &mut Vec<Span>, stmt: &Stmt) {
    match stmt {
        Stmt::Var { ty: _, value, .. } => visit_expr(out, &value.node),
        Stmt::Assign { target, value, .. } => {
            visit_expr(out, &target.node);
            visit_expr(out, &value.node);
        }
        Stmt::Expr(e) => visit_expr(out, &e.node),
        Stmt::If { cond, then, otherwise } => {
            match cond {
                IfCond::Expr(e) => visit_expr(out, &e.node),
                IfCond::Let { value, .. } => visit_expr(out, &value.node),
            }
            visit_block(out, then);
            if let Some(otherwise) = otherwise {
                match otherwise {
                    Else::Block(b) => visit_block(out, b),
                    Else::If(s) => visit_stmt(out, &s.node),
                }
            }
        }
        Stmt::For { iter, body, .. } => {
            visit_expr(out, &iter.node);
            visit_block(out, body);
        }
        Stmt::While { cond, body } | Stmt::DoWhile { cond, body } => {
            visit_expr(out, &cond.node);
            visit_block(out, body);
        }
        Stmt::Switch { scrutinee, cases, default } => {
            visit_expr(out, &scrutinee.node);
            for case in cases {
                if let Some(guard) = &case.guard {
                    visit_expr(out, &guard.node);
                }
                visit_stmts(out, &case.body);
            }
            if let Some(default) = default {
                visit_stmts(out, default);
            }
        }
        Stmt::Return(e) => {
            if let Some(e) = e {
                visit_expr(out, &e.node);
            }
        }
        Stmt::Assert(e) => visit_expr(out, &e.node),
        Stmt::Defer(b) | Stmt::UnsafeBlock(b) => visit_block(out, b),
        Stmt::Guard { value, otherwise, .. } => {
            visit_expr(out, &value.node);
            visit_block(out, otherwise);
        }
        Stmt::Try { body, catch, finally } => {
            visit_block(out, body);
            if let Some((_, block)) = catch {
                visit_block(out, block);
            }
            if let Some(block) = finally {
                visit_block(out, block);
            }
        }
        Stmt::Throw(e) => {
            if let Some(e) = e {
                visit_expr(out, &e.node);
            }
        }
        _ => {}
    }
}

/// Number of expression and statement nodes below `expr`, including itself.
/// Macro expansion uses this to measure the size of what a macro produced.
pub(crate) fn expr_node_count(expr: &Expr) -> usize {
    expr_nodes(expr)
}

fn fn_body_nodes(body: &FnBody) -> usize {
    match body {
        FnBody::Block(b) => block_nodes(b),
        FnBody::Expr(e) => expr_nodes(&e.node),
    }
}

fn block_nodes(block: &Block) -> usize {
    block.stmts.iter().map(|s| stmt_nodes(&s.node)).sum()
}

fn stmt_nodes(stmt: &Stmt) -> usize {
    let mut n = 1;
    match stmt {
        Stmt::Var { value, .. }
        | Stmt::DestructureTuple { value, .. }
        | Stmt::DestructureRecord { value, .. }
        | Stmt::DestructureArray { value, .. } => n += expr_nodes(&value.node),
        Stmt::Assign { target, value, .. } => {
            n += expr_nodes(&target.node) + expr_nodes(&value.node)
        }
        Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) => n += expr_nodes(&e.node),
        Stmt::If { cond, then, otherwise } => {
            n += match cond {
                IfCond::Expr(e) | IfCond::Let { value: e, .. } => expr_nodes(&e.node),
            };
            n += block_nodes(then);
            n += match otherwise {
                Some(Else::Block(b)) => block_nodes(b),
                Some(Else::If(s)) => stmt_nodes(&s.node),
                None => 0,
            };
        }
        Stmt::For { iter, body, .. } => n += expr_nodes(&iter.node) + block_nodes(body),
        Stmt::While { cond, body } => n += expr_nodes(&cond.node) + block_nodes(body),
        Stmt::DoWhile { body, cond } => n += block_nodes(body) + expr_nodes(&cond.node),
        Stmt::Switch { scrutinee, cases, default } => {
            n += expr_nodes(&scrutinee.node);
            for c in cases {
                if let Some(g) = &c.guard {
                    n += expr_nodes(&g.node);
                }
                n += c.body.iter().map(|s| stmt_nodes(&s.node)).sum::<usize>();
            }
            if let Some(d) = default {
                n += d.iter().map(|s| stmt_nodes(&s.node)).sum::<usize>();
            }
        }
        Stmt::Defer(b) | Stmt::UnsafeBlock(b) => n += block_nodes(b),
        Stmt::Guard { value, otherwise, .. } => {
            n += expr_nodes(&value.node) + block_nodes(otherwise)
        }
        Stmt::Try { body, catch, finally } => {
            n += block_nodes(body);
            if let Some((_, b)) = catch {
                n += block_nodes(b);
            }
            if let Some(b) = finally {
                n += block_nodes(b);
            }
        }
        Stmt::Throw(Some(e)) => n += expr_nodes(&e.node),
        Stmt::Return(None) | Stmt::Throw(None) | Stmt::Break | Stmt::Continue
        | Stmt::Fallthrough | Stmt::Pass | Stmt::Empty => {}
    }
    n
}

fn expr_nodes(expr: &Expr) -> usize {
    let mut n = 1;
    match expr {
        Expr::Interp(parts) => {
            for part in parts {
                if let InterpPart::Expr(e) = part {
                    n += expr_nodes(&e.node);
                }
            }
        }
        Expr::Array(items) => {
            for i in items {
                n += expr_nodes(&i.expr.node);
            }
        }
        Expr::Record(fields) => {
            for f in fields {
                n += expr_nodes(&f.value().node);
            }
        }
        Expr::MapLiteral(entries) => {
            for e in entries {
                n += expr_nodes(&e.value().node);
            }
        }
        Expr::Binary { lhs, rhs, .. }
        | Expr::Coalesce { lhs, rhs }
        | Expr::Range { lo: lhs, hi: rhs, .. }
        | Expr::Index { base: lhs, index: rhs } => {
            n += expr_nodes(&lhs.node) + expr_nodes(&rhs.node)
        }
        Expr::Unary { rhs, .. }
        | Expr::Postfix { expr: rhs, .. }
        | Expr::Cast { expr: rhs, .. }
        | Expr::Is { base: rhs, .. }
        | Expr::OptChain { base: rhs, .. }
        | Expr::Member { base: rhs, .. }
        | Expr::TupleGet { base: rhs, .. }
        | Expr::Await(rhs)
        | Expr::Propagate(rhs) => n += expr_nodes(&rhs.node),
        Expr::Ternary { cond, then, otherwise } => {
            n += expr_nodes(&cond.node) + expr_nodes(&then.node) + expr_nodes(&otherwise.node)
        }
        Expr::OptCall { base, args, .. } | Expr::Call { callee: base, args, .. } => {
            n += expr_nodes(&base.node);
            for a in args {
                n += expr_nodes(&a.value.node);
            }
            if let Expr::Call { trailing: Some(b), .. } = expr {
                n += block_nodes(b);
            }
        }
        Expr::New { args, .. } => {
            for a in args {
                n += expr_nodes(&a.value.node);
            }
        }
        Expr::Macro { args, .. } => {
            for a in args {
                n += expr_nodes(&a.node);
            }
        }
        Expr::Closure { body, .. } => n += fn_body_nodes(body),
        Expr::UnsafeBlock(b) => n += block_nodes(b),
        Expr::Tuple(items) => {
            for i in items {
                n += expr_nodes(&i.node);
            }
        }
        Expr::Switch { scrutinee, cases, default } => {
            n += expr_nodes(&scrutinee.node);
            for c in cases {
                if let Some(g) = &c.guard {
                    n += expr_nodes(&g.node);
                }
                n += match &c.body {
                    SwitchExprBody::Expr(e) => expr_nodes(&e.node),
                    SwitchExprBody::Block(b) => block_nodes(b),
                };
            }
            if let Some(d) = default {
                n += match d {
                    SwitchExprBody::Expr(e) => expr_nodes(&e.node),
                    SwitchExprBody::Block(b) => block_nodes(b),
                };
            }
        }
        Expr::Ident(_) | Expr::This | Expr::Super | Expr::Bool(_) | Expr::Null
        | Expr::Int(_) | Expr::Float(_) | Expr::ImplicitMember(_) => {}
    }
    n
}

fn visit_expr(out: &mut Vec<Span>, expr: &Expr) {
    match expr {
        Expr::Closure { params, body, .. } => {
            push_params(out, params);
            visit_fn_body(out, body);
        }
        Expr::Binary { lhs, rhs, .. } => {
            visit_expr(out, &lhs.node);
            visit_expr(out, &rhs.node);
        }
        Expr::Unary { rhs, .. } => visit_expr(out, &rhs.node),
        Expr::Postfix { expr, .. } => visit_expr(out, &expr.node),
        Expr::Ternary { cond, then, otherwise } => {
            visit_expr(out, &cond.node);
            visit_expr(out, &then.node);
            visit_expr(out, &otherwise.node);
        }
        Expr::Coalesce { lhs, rhs } => {
            visit_expr(out, &lhs.node);
            visit_expr(out, &rhs.node);
        }
        Expr::OptChain { base, .. } => visit_expr(out, &base.node),
        Expr::OptCall { base, args, .. } => {
            visit_expr(out, &base.node);
            for arg in args {
                visit_expr(out, &arg.value.node);
            }
        }
        Expr::Range { lo, hi, .. } => {
            visit_expr(out, &lo.node);
            visit_expr(out, &hi.node);
        }
        Expr::Call { callee, args, trailing, .. } => {
            visit_expr(out, &callee.node);
            for arg in args {
                visit_expr(out, &arg.value.node);
            }
            if let Some(block) = trailing {
                visit_block(out, block);
            }
        }
        Expr::Index { base, index } => {
            visit_expr(out, &base.node);
            visit_expr(out, &index.node);
        }
        Expr::Member { base, .. } => visit_expr(out, &base.node),
        Expr::Cast { expr, .. } => visit_expr(out, &expr.node),
        Expr::Is { base, .. } => visit_expr(out, &base.node),
        Expr::Interp(parts) => {
            for part in parts {
                if let InterpPart::Expr(e) = part {
                    visit_expr(out, &e.node);
                }
            }
        }
        Expr::Array(items) => { for i in items { visit_expr(out, &i.expr.node); } }
        Expr::Record(fields) => {
            for e in fields {
                visit_expr(out, &e.value().node);
            }
        }
        Expr::MapLiteral(entries) => {
            for e in entries {
                visit_expr(out, &e.value().node);
            }
        }
        Expr::Macro { args, .. } => visit_exprs(out, args),
        Expr::UnsafeBlock(b) => visit_block(out, b),
        Expr::Await(e) => visit_expr(out, &e.node),
        _ => {}
    }
}
