use crate::ast::*;
use diagnostics::{decl_file, Code, DeclFiles, Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

pub(super) fn check_w104(class: &str, members: &[Spanned<ClassMember>], diags: &mut Vec<Diagnostic>) {
    for m in members {
        let body = match &m.node {
            ClassMember::Method(f) => match &f.body {
                FnBody::Block(b) => b,
                FnBody::Expr(_) => continue,
            },
            ClassMember::Init { body, .. } => body,
            _ => continue,
        };
        for st in &body.stmts {
            if let Stmt::Assign { target, value, .. } = &st.node {
                if !is_this_member(&target.node) {
                    continue;
                }
                let mut spans = Vec::new();
                find_this_closures(&value.node, value.span, &mut spans);
                for span in spans {
                    diags.push(
                        Diagnostic::new(
                            Code::W104,
                            format!("self-capturing closure stored on `{class}`"),
                        )
                        .with_span(span)
                        .with_hint("use `fn decay(this)` so the closure expires with the owner"),
                    );
                }
            }
        }
    }
}

fn is_this_member(e: &Expr) -> bool {
    matches!(e, Expr::Member { base, .. } if matches!(&base.node, Expr::This))
}

fn find_this_closures(e: &Expr, span: Span, out: &mut Vec<Span>) {
    match e {
        Expr::Closure { decay, body, .. } => {
            if !decay {
                let uses = match body {
                    FnBody::Block(b) => block_uses_this(b),
                    FnBody::Expr(body_expr) => expr_uses_this(&body_expr.node),
                };
                if uses {
                    out.push(span);
                }
            }
            return;
        }
        Expr::Binary { lhs, rhs, .. } => {
            find_this_closures(&lhs.node, lhs.span, out);
            find_this_closures(&rhs.node, rhs.span, out);
        }
        Expr::Unary { rhs, .. } => find_this_closures(&rhs.node, rhs.span, out),
        Expr::Await(e) | Expr::Propagate(e) => find_this_closures(&e.node, e.span, out),
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            find_this_closures(&cond.node, cond.span, out);
            find_this_closures(&then.node, then.span, out);
            find_this_closures(&otherwise.node, otherwise.span, out);
        }
        Expr::Range { lo, hi, .. } => {
            find_this_closures(&lo.node, lo.span, out);
            find_this_closures(&hi.node, hi.span, out);
        }
        Expr::Call {
            callee, args, ..
        } => {
            find_this_closures(&callee.node, callee.span, out);
            for a in args {
                find_this_closures(&a.value.node, a.value.span, out);
            }
        }
        Expr::Index { base, index } => {
            find_this_closures(&base.node, base.span, out);
            find_this_closures(&index.node, index.span, out);
        }
        Expr::Member { base, .. } => find_this_closures(&base.node, base.span, out),
        Expr::Coalesce { lhs, rhs } => {
            find_this_closures(&lhs.node, lhs.span, out);
            find_this_closures(&rhs.node, rhs.span, out);
        }
        Expr::OptChain { base, .. } => find_this_closures(&base.node, base.span, out),
        Expr::OptCall { base, args, .. } => {
            find_this_closures(&base.node, base.span, out);
            for a in args {
                find_this_closures(&a.value.node, a.value.span, out);
            }
        }
        Expr::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(e) = p {
                    find_this_closures(&e.node, e.span, out);
                }
            }
        }
        Expr::Array(items) => {
            for i in items {
                find_this_closures(&i.expr.node, i.expr.span, out);
            }
        }
        Expr::Record(fields) => {
            for e in fields {
                let v = e.value();
                find_this_closures(&v.node, v.span, out);
            }
        }
        Expr::MapLiteral(entries) => {
            for e in entries {
                let v = e.value();
                find_this_closures(&v.node, v.span, out);
            }
        }
        Expr::Macro { args, .. } => {
            for a in args {
                find_this_closures(&a.node, a.span, out);
            }
        }
        Expr::UnsafeBlock(b) => {
            for st in &b.stmts {
                find_unsafe_this_closures(&st.node, out);
            }
        }
        _ => {}
    }
}

fn find_unsafe_this_closures(s: &Stmt, out: &mut Vec<Span>) {
    match s {
        Stmt::Var { value, .. } => find_this_closures(&value.node, value.span, out),
        Stmt::Assign { value, .. } => find_this_closures(&value.node, value.span, out),
        Stmt::Expr(e) | Stmt::Assert(e) => find_this_closures(&e.node, e.span, out),
        Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => {
            find_this_closures(&e.node, e.span, out)
        }
        _ => {}
    }
}

fn block_uses_this(b: &Block) -> bool {
    b.stmts.iter().any(|s| stmt_uses_this(&s.node))
}

fn stmt_uses_this(s: &Stmt) -> bool {
    match s {
        Stmt::Var { value, .. } => expr_uses_this(&value.node),
        Stmt::DestructureTuple { value, .. }
        | Stmt::DestructureRecord { value, .. }
        | Stmt::DestructureArray { value, .. } => expr_uses_this(&value.node),
        Stmt::Assign { target, value, .. } => {
            expr_uses_this(&target.node) || expr_uses_this(&value.node)
        }
        Stmt::Expr(e) | Stmt::Assert(e) => expr_uses_this(&e.node),
        Stmt::If { cond, then, .. } => {
            let c = match cond {
                IfCond::Expr(e) | IfCond::Let { value: e, .. } => expr_uses_this(&e.node),
            };
            c || block_uses_this(then)
        }
        Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => expr_uses_this(&e.node),
        Stmt::Return(None)
        | Stmt::Throw(None)
        | Stmt::Break
        | Stmt::Continue
        | Stmt::Fallthrough
        | Stmt::Pass
        | Stmt::Empty => {
            false
        }
        Stmt::While { cond, body } => expr_uses_this(&cond.node) || block_uses_this(body),
        Stmt::DoWhile { body, cond } => block_uses_this(body) || expr_uses_this(&cond.node),
        Stmt::For { iter, body, .. } => expr_uses_this(&iter.node) || block_uses_this(body),
        Stmt::Switch {
            scrutinee, cases, ..
        } => {
            expr_uses_this(&scrutinee.node)
                || cases.iter().any(|c| {
                    c.guard
                        .as_ref()
                        .map(|g| expr_uses_this(&g.node))
                        .unwrap_or(false)
                        || c.body.iter().any(|s| stmt_uses_this(&s.node))
                })
        }
        Stmt::Defer(b) => block_uses_this(b),
        Stmt::UnsafeBlock(b) => block_uses_this(b),
        Stmt::Guard { value, .. } => expr_uses_this(&value.node),
        Stmt::Try { body, catch, .. } => {
            block_uses_this(body)
                || catch
                    .as_ref()
                    .map(|(_, b)| block_uses_this(b))
                    .unwrap_or(false)
        }
    }
}

fn expr_uses_this(e: &Expr) -> bool {
    match e {
        Expr::This | Expr::Super => true,
        Expr::Ident(_) | Expr::Bool(_) | Expr::Null | Expr::Int(_) | Expr::Float(_) => false,
        Expr::Binary { lhs, rhs, .. } => expr_uses_this(&lhs.node) || expr_uses_this(&rhs.node),
        Expr::Is { base, .. } => expr_uses_this(&base.node),
        Expr::Unary { rhs, .. } => expr_uses_this(&rhs.node),
        Expr::Postfix { expr, .. } => expr_uses_this(&expr.node),
        Expr::Await(e) | Expr::Propagate(e) => expr_uses_this(&e.node),
        Expr::Tuple(items) => items.iter().any(|i| expr_uses_this(&i.node)),
        Expr::TupleGet { base, .. } => expr_uses_this(&base.node),
        Expr::Switch { scrutinee, cases, default } => {
            let mut out = expr_uses_this(&scrutinee.node);
            for c in cases {
                if let Some(g) = &c.guard {
                    out = out || expr_uses_this(&g.node);
                }
                out = out
                    || match &c.body {
                        crate::ast::SwitchExprBody::Expr(e) => expr_uses_this(&e.node),
                        crate::ast::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_uses_this(&s.node))
                        }
                    };
            }
            if let Some(d) = default {
                out = out
                    || match d {
                        crate::ast::SwitchExprBody::Expr(e) => expr_uses_this(&e.node),
                        crate::ast::SwitchExprBody::Block(b) => {
                            b.stmts.iter().any(|s| stmt_uses_this(&s.node))
                        }
                    };
            }
            out
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            expr_uses_this(&cond.node)
                || expr_uses_this(&then.node)
                || expr_uses_this(&otherwise.node)
        }
        Expr::Range { lo, hi, .. } => expr_uses_this(&lo.node) || expr_uses_this(&hi.node),
        Expr::Call { callee, args, .. } => {
            expr_uses_this(&callee.node)
                || args.iter().any(|a| expr_uses_this(&a.value.node))
        }
        Expr::New { args, .. } => args.iter().any(|a| expr_uses_this(&a.value.node)),
        Expr::Index { base, index } => {
            expr_uses_this(&base.node) || expr_uses_this(&index.node)
        }
        Expr::Member { base, .. } => expr_uses_this(&base.node),
        Expr::Coalesce { lhs, rhs } => {
            expr_uses_this(&lhs.node) || expr_uses_this(&rhs.node)
        }
        Expr::OptChain { base, .. } => expr_uses_this(&base.node),
        Expr::OptCall { base, args, .. } => {
            expr_uses_this(&base.node)
                || args.iter().any(|a| expr_uses_this(&a.value.node))
        }
        Expr::Cast { expr, .. } => expr_uses_this(&expr.node),
        Expr::Interp(parts) => parts.iter().any(|p| match p {
            InterpPart::Text(_) => false,
            InterpPart::Expr(e) => expr_uses_this(&e.node),
        }),
        Expr::Array(items) => items.iter().any(|i| expr_uses_this(&i.expr.node)),
        Expr::Record(fields) => fields.iter().any(|e| expr_uses_this(&e.value().node)),
        Expr::MapLiteral(entries) => entries.iter().any(|e| expr_uses_this(&e.value().node)),
        Expr::Macro { args, .. } => args.iter().any(|a| expr_uses_this(&a.node)),
        Expr::ImplicitMember(_) => false,
        Expr::Closure { .. } => false,
        Expr::UnsafeBlock(b) => b.stmts.iter().any(|st| stmt_uses_this(&st.node)),
    }
}

pub(super) fn check_w109(class: &str, members: &[Spanned<ClassMember>], diags: &mut Vec<Diagnostic>) {
    let mut allowed: Vec<(&str, Span)> = Vec::new();
    for m in members {
        if let ClassMember::Field(f) = &m.node {
            if f.attrs.iter().any(|a| a.name == "Allow") {
                allowed.push((f.name.as_str(), m.span));
            }
        }
    }
    if allowed.is_empty() {
        return;
    }
    for m in members {
        let is_cleanup = match &m.node {
            ClassMember::Deinit(_) => true,
            ClassMember::Method(f) => {
                f.name.starts_with("clear")
                    || f.name.starts_with("close")
                    || f.name.starts_with("reset")
            }
            _ => continue,
        };
        if !is_cleanup {
            continue;
        }
        let body = match &m.node {
            ClassMember::Method(f) => match &f.body {
                FnBody::Block(b) => b,
                FnBody::Expr(_) => continue,
            },
            ClassMember::Deinit(b) => b,
            _ => continue,
        };
        for st in &body.stmts {
            if let Stmt::Assign { target, value, .. } = &st.node {
                if !matches!(&value.node, Expr::Null) {
                    continue;
                }
                if let Expr::Member { base, field } = &target.node {
                    if matches!(&base.node, Expr::This | Expr::Super) {
                        allowed.retain(|(n, _)| *n != field.as_str());
                    }
                }
            }
        }
    }
    for (name, span) in allowed {
        diags.push(
            Diagnostic::new(
                Code::W109,
                format!("`{class}.{name}` allows cycles but is never cleared"),
            )
            .with_span(span)
            .with_hint("set it to `null` in `deinit` or a `clear*`/`close*`/`reset*` method"),
        );
    }
}

pub(super) fn check_w108(module: &Module, diags: &mut Vec<Diagnostic>, files: &DeclFiles) {
    let mut edges: BTreeMap<String, BTreeMap<String, (Span, Option<std::path::PathBuf>)>> =
        BTreeMap::new();
    for decl in &module.decls {
        if let Decl::Class { name, members, .. } = &decl.node {
            let file = decl_file(files, decl.span);
            for m in members {
                if let ClassMember::Field(f) = &m.node {
                    if let Some(t) = &f.ty {
                        if t.path.first().map(|s| s.as_str()) == Some("GenRef") {
                            continue;
                        }
                        if !t.args.is_empty() || t.fn_sig.is_some() {
                            continue;
                        }
                        if let Some(last) = t.path.last() {
                            edges
                                .entry(name.clone())
                                .or_default()
                                .insert(last.clone(), (m.span, file.clone()));
                        }
                    }
                }
            }
        }
    }
    let names: Vec<String> = edges.keys().cloned().collect();
    for (i, a) in names.iter().enumerate() {
        for b in &names[i + 1..] {
            let ab = edges.get(a).map(|m| m.contains_key(b)).unwrap_or(false);
            let ba = edges.get(b).map(|m| m.contains_key(a)).unwrap_or(false);
            if ab && ba {
                let (span, file) = edges[a][b].clone();
                let (first, second) = if a < b { (a, b) } else { (b, a) };
                let mut d = Diagnostic::new(
                    Code::W108,
                    format!("mutual strong fields `{first}` <-> `{second}`"),
                )
                .with_span(span)
                .with_hint("make one side `GenRef` or add `#[Allow(CyclicReference)]`");
                if let Some(f) = file {
                    d.file = Some(f);
                }
                diags.push(d);
            }
        }
    }
}

fn default_news_expr(e: &Expr, out: &mut Vec<String>) {
    match e {
        Expr::New { target, args, .. } => {
            out.push(crate::prelude::short_name(target).to_string());
            for a in args {
                default_news_expr(&a.value.node, out);
            }
        }
        Expr::Closure { .. } => {}
        Expr::Array(items) => {
            for i in items {
                default_news_expr(&i.expr.node, out);
            }
        }
        Expr::Record(fields) => {
            for e in fields {
                default_news_expr(&e.value().node, out);
            }
        }
        Expr::MapLiteral(fields) => {
            for e in fields {
                default_news_expr(&e.value().node, out);
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            default_news_expr(&lhs.node, out);
            default_news_expr(&rhs.node, out);
        }
        Expr::Unary { rhs, .. } => default_news_expr(&rhs.node, out),
        Expr::Postfix { expr, .. } => default_news_expr(&expr.node, out),
        Expr::Ternary { cond, .. } => default_news_expr(&cond.node, out),
        Expr::Coalesce { lhs, .. } => default_news_expr(&lhs.node, out),
        Expr::OptChain { base, .. } => default_news_expr(&base.node, out),
        Expr::OptCall { base, .. } => default_news_expr(&base.node, out),
        Expr::Range { lo, hi, .. } => {
            default_news_expr(&lo.node, out);
            default_news_expr(&hi.node, out);
        }
        Expr::Call { callee, args, .. } => {
            default_news_expr(&callee.node, out);
            for a in args {
                default_news_expr(&a.value.node, out);
            }
        }
        Expr::Index { base, index } => {
            default_news_expr(&base.node, out);
            default_news_expr(&index.node, out);
        }
        Expr::Member { base, .. } => default_news_expr(&base.node, out),
        Expr::Cast { expr, .. } => default_news_expr(&expr.node, out),
        Expr::Is { base, .. } => default_news_expr(&base.node, out),
        Expr::Macro { args, .. } => {
            for a in args {
                default_news_expr(&a.node, out);
            }
        }
        Expr::Tuple(items) => {
            for i in items {
                default_news_expr(&i.node, out);
            }
        }
        Expr::TupleGet { base, .. } => default_news_expr(&base.node, out),
        Expr::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    default_news_expr(&x.node, out);
                }
            }
        }
        Expr::UnsafeBlock(b) => default_news_block(b, out),
        Expr::Switch { scrutinee, .. } => default_news_expr(&scrutinee.node, out),
        Expr::Await(inner) | Expr::Propagate(inner) => default_news_expr(&inner.node, out),
        Expr::Ident(_)
        | Expr::This
        | Expr::Super
        | Expr::Bool(_)
        | Expr::Null
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::ImplicitMember(_) => {}
    }
}

fn default_news_block(b: &Block, out: &mut Vec<String>) {
    for st in &b.stmts {
        match &st.node {
            Stmt::Var { value, .. } => default_news_expr(&value.node, out),
            Stmt::DestructureTuple { value, .. }
            | Stmt::DestructureRecord { value, .. }
            | Stmt::DestructureArray { value, .. } => default_news_expr(&value.node, out),
            Stmt::Assign { value, .. } => default_news_expr(&value.node, out),
            Stmt::Expr(e) => default_news_expr(&e.node, out),
            Stmt::Return(Some(e)) => default_news_expr(&e.node, out),
            _ => {}
        }
    }
}

pub(super) fn check_cyclic_defaults(module: &Module, diags: &mut Vec<Diagnostic>, files: &DeclFiles) {
    let mut known: BTreeSet<String> = BTreeSet::new();
    for decl in &module.decls {
        match &decl.node {
            Decl::Class { name, .. } | Decl::Struct { name, .. } => {
                known.insert(name.clone());
            }
            _ => {}
        }
    }
    let mut edges: BTreeMap<String, Vec<(String, Span, Option<std::path::PathBuf>)>> =
        BTreeMap::new();
    for decl in &module.decls {
        let (name, members) = match &decl.node {
            Decl::Class { name, members, .. } | Decl::Struct { name, members, .. } => (name, members),
            _ => continue,
        };
        let file = decl_file(files, decl.span);
        for m in members {
            if let ClassMember::Field(f) = &m.node {
                if let Some(v) = &f.value {
                    let mut news = Vec::new();
                    default_news_expr(&v.node, &mut news);
                    for t in news {
                        if known.contains(&t) {
                            edges
                                .entry(name.clone())
                                .or_default()
                                .push((t, m.span, file.clone()));
                        }
                    }
                }
            }
        }
    }
    let mut reported: BTreeSet<String> = BTreeSet::new();
    let mut black: BTreeSet<String> = BTreeSet::new();
    for start in edges.keys().cloned().collect::<Vec<_>>() {
        if black.contains(&start) {
            continue;
        }
        let mut stack: Vec<(String, usize)> = vec![(start.clone(), 0)];
        let mut gray: BTreeSet<String> = BTreeSet::new();
        gray.insert(start.clone());
        while let Some((top, idx)) = stack.last().cloned() {
            let nexts = edges.get(&top).cloned().unwrap_or_default();
            if idx < nexts.len() {
                stack.last_mut().expect("dfs stack").1 += 1;
                let (nxt, span, file) = &nexts[idx];
                if gray.contains(nxt) {
                    let pos = stack.iter().position(|(n, _)| n == nxt).unwrap_or(0);
                    let mut cycle: Vec<String> =
                        stack[pos..].iter().map(|(n, _)| n.clone()).collect();
                    cycle.push(nxt.clone());
                    let key = cycle.join("->");
                    if reported.insert(key) {
                        let mut d = Diagnostic::new(
                            Code::E108,
                            format!(
                                "cyclic default construction `{}` never terminates",
                                cycle.join(" -> ")
                            ),
                        )
                        .with_span(*span)
                        .with_hint(
                            "use a nullable field without a default and assign it in the constructor",
                        );
                        if let Some(f) = file.clone() {
                            d.file = Some(f);
                        }
                        diags.push(d);
                    }
                } else if !black.contains(nxt) {
                    gray.insert(nxt.clone());
                    stack.push((nxt.clone(), 0));
                }
            } else {
                let (done, _) = stack.pop().expect("dfs stack");
                gray.remove(&done);
                black.insert(done);
            }
        }
    }
}
