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
