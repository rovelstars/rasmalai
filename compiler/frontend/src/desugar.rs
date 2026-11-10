use crate::ast::*;
use diagnostics::{decl_file, tag_new, Code, DeclFiles, Diagnostic, Span};

pub fn desugar(module: &mut Module) -> Vec<Diagnostic> {
    desugar_with_files(module, &DeclFiles::new())
}

pub fn desugar_with_files(module: &mut Module, files: &DeclFiles) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    synthesize_entry_main(module, &mut diags, files);
    diags.extend(crate::async_expand::check_await_outside_async(module, files));
    diags.extend(crate::async_expand::check_async_ret_types(module, files));
    diags.extend(crate::async_expand::check_no_poll_decls(module, files));
    diags.extend(crate::async_expand::check_wait_inside_async(module, files));
    crate::async_expand::expand_async(module, &mut diags);
    lift_static_methods(module, &mut diags, files);
    for decl in &mut module.decls {
        let from = diags.len();
        let file = decl_file(files, decl.span);
        match &mut decl.node {
            Decl::Class { members, .. }
            | Decl::Struct { members, .. }
            | Decl::Trait { members, .. }
            | Decl::Interface { members, .. }
            | Decl::Extension { members, .. } => {
                for m in members {
                    desugar_member(&mut m.node, &mut diags);
                }
            }
            Decl::Fn(f) => desugar_body(&mut f.body, &mut diags),
            _ => {}
        }
        tag_new(&mut diags, from, file);
    }
    diags
}

fn top_expr_has_await(e: &Expr) -> bool {
    let ex = |x: &Spanned<Expr>| top_expr_has_await(&x.node);
    match e {
        Expr::Await(_) => true,
        Expr::Closure { .. } => false,
        Expr::Binary { lhs, rhs, .. } => ex(lhs) || ex(rhs),
        Expr::Unary { rhs, .. } => ex(rhs),
        Expr::Postfix { expr, .. } => ex(expr),
        Expr::Ternary { cond, then, otherwise } => ex(cond) || ex(then) || ex(otherwise),
        Expr::Coalesce { lhs, rhs } => ex(lhs) || ex(rhs),
        Expr::OptChain { base, .. } => ex(base),
        Expr::OptCall { base, args, .. } => {
            ex(base) || args.iter().any(|a| ex(&a.value))
        }
        Expr::Range { lo, hi, .. } => ex(lo) || ex(hi),
        Expr::Call { callee, args, trailing, .. } => {
            ex(callee)
                || args.iter().any(|a| ex(&a.value))
                || trailing.as_ref().map(top_block_has_await).unwrap_or(false)
        }
        Expr::New { args, .. } => args.iter().any(|a| ex(&a.value)),
        Expr::Index { base, index } => ex(base) || ex(index),
        Expr::Member { base, .. } => ex(base),
        Expr::Cast { expr, .. } => ex(expr),
        Expr::Is { base, .. } => ex(base),
        Expr::Interp(parts) => parts.iter().any(|p| match p {
            InterpPart::Expr(x) => ex(x),
            _ => false,
        }),
        Expr::Array(items) => items.iter().any(|i| ex(&i.expr)),
        Expr::Record(fields) => fields.iter().any(|e| ex(e.value())),
        Expr::MapLiteral(entries) => entries.iter().any(|e| ex(e.value())),
        Expr::Macro { args, .. } => args.iter().any(ex),
        Expr::Propagate(inner) => ex(inner),
        Expr::Tuple(items) => items.iter().any(ex),
        Expr::TupleGet { base, .. } => ex(base),
        Expr::Switch { scrutinee, cases, default } => {
            ex(scrutinee)
                || cases.iter().any(|c| {
                    c.guard.as_ref().map(ex).unwrap_or(false)
                        || match &c.body {
                            SwitchExprBody::Expr(x) => ex(x),
                            SwitchExprBody::Block(b) => top_block_has_await(b),
                        }
                })
                || default.as_ref().map(|d| match d {
                    SwitchExprBody::Expr(x) => ex(x),
                    SwitchExprBody::Block(b) => top_block_has_await(b),
                }).unwrap_or(false)
        }
        Expr::UnsafeBlock(b) => top_block_has_await(b),
        _ => false,
    }
}

fn top_block_has_await(b: &Block) -> bool {
    b.stmts.iter().any(|s| top_stmt_has_await(&s.node))
}

fn top_else_has_await(e: &Else) -> bool {
    match e {
        Else::Block(b) => top_block_has_await(b),
        Else::If(s) => top_stmt_has_await(&s.node),
    }
}

fn top_stmt_has_await(s: &Stmt) -> bool {
    let ex = |x: &Spanned<Expr>| top_expr_has_await(&x.node);
    match s {
        Stmt::Var { value, .. }
        | Stmt::DestructureTuple { value, .. }
        | Stmt::DestructureRecord { value, .. }
        | Stmt::DestructureArray { value, .. } => ex(value),
        Stmt::Assign { target, value, .. } => ex(target) || ex(value),
        Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => ex(e),
        Stmt::If { cond, then, otherwise } => {
            let c = match cond {
                IfCond::Expr(e) => ex(e),
                IfCond::Let { value, .. } => ex(value),
            };
            c || top_block_has_await(then)
                || otherwise.as_ref().map(top_else_has_await).unwrap_or(false)
        }
        Stmt::While { cond, body } | Stmt::DoWhile { cond, body } => {
            ex(cond) || top_block_has_await(body)
        }
        Stmt::For { iter, body, .. } => ex(iter) || top_block_has_await(body),
        Stmt::Switch { scrutinee, cases, default } => {
            ex(scrutinee)
                || cases.iter().any(|c| {
                    c.guard.as_ref().map(ex).unwrap_or(false)
                        || c.body.iter().any(|x| top_stmt_has_await(&x.node))
                })
                || default.as_ref().map(|d| d.iter().any(|x| top_stmt_has_await(&x.node))).unwrap_or(false)
        }
        Stmt::Defer(b) | Stmt::UnsafeBlock(b) => top_block_has_await(b),
        Stmt::Guard { value, otherwise, .. } => ex(value) || top_block_has_await(otherwise),
        Stmt::Try { body, catch, finally } => {
            top_block_has_await(body)
                || catch.as_ref().map(|(_, b)| top_block_has_await(b)).unwrap_or(false)
                || finally.as_ref().map(top_block_has_await).unwrap_or(false)
        }
        _ => false,
    }
}

fn synthesize_entry_main(module: &mut Module, diags: &mut Vec<Diagnostic>, files: &DeclFiles) {
    let lower = module.decls.iter().find(|d| matches!(&d.node, Decl::Fn(f) if f.name == "main"));
    let upper = module.decls.iter().find(|d| matches!(&d.node, Decl::Fn(f) if f.name == "Main"));
    if let (Some(lo), Some(hi)) = (lower, upper) {
        let mut d = Diagnostic::new(
            Code::E108,
            "Multiple conflicting entry functions ('main' and 'Main') found in entry file; define exactly one entry point",
        )
        .with_span(hi.span);
        if let Some(f) = decl_file(files, lo.span) {
            d.file = Some(f);
        }
        diags.push(d);
        return;
    }
    let has_top = module.decls.iter().any(|d| matches!(&d.node, Decl::Stmt(_)));
    let has_main = module.decls.iter().any(|d| matches!(&d.node, Decl::Fn(f) if f.name == "Main" || f.name == "main"));
    if !has_top {
        if has_main {
            return;
        }
        let has_const = module.decls.iter().any(|d| matches!(&d.node, Decl::Const { .. }));
        if !has_const {
            return;
        }
        let span = module.decls.iter().find_map(|d| {
            matches!(&d.node, Decl::Const { .. }).then_some(d.span)
        }).unwrap_or(Span { start: 0, end: 0 });
        let int = Type { path: vec!["std.prelude.Int".to_string()], args: Vec::new(), nullable: false, fn_sig: None, tuple: Vec::new() };
        let entry = Decl::Fn(FnDecl {
            docs: String::new(),
            access: Access::Internal,
            name: "Main".to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            ret: Some(int),
            throws: false,
            is_unsafe: false,
            is_async: false,
            is_static: false,
            is_test: false,
            is_bench: false,
            is_macro: false,
            body: FnBody::Block(Block { stmts: vec![sp(Stmt::Return(Some(sp(Expr::Int(0), span))), span)] }),
            attrs: Vec::new(),
        });
        module.decls.push(sp(entry, span));
        return;
    }
    let decls = std::mem::take(&mut module.decls);
    let mut top: Vec<Spanned<Stmt>> = Vec::new();
    let mut kept: Vec<Spanned<Decl>> = Vec::with_capacity(decls.len());
    for d in decls {
        match d.node {
            Decl::Stmt(s) => top.push(*s),
            node => kept.push(Spanned { node, span: d.span }),
        }
    }
    module.decls = kept;
    if has_main {
        let mut d = Diagnostic::new(
            Code::E108,
            "top-level statements need an implicit entrypoint; remove the explicit `main`/`Main` or move the statements into it",
        )
        .with_span(top[0].span);
        if let Some(f) = decl_file(files, top[0].span) {
            d.file = Some(f);
        }
        diags.push(d);
        return;
    }
    let span = top[0].span;
    let ends_return = matches!(top.last().map(|s| &s.node), Some(Stmt::Return(_)));
    if !ends_return {
        top.push(sp(Stmt::Return(Some(sp(Expr::Int(0), span))), span));
    }
    let int = Type { path: vec!["std.prelude.Int".to_string()], args: Vec::new(), nullable: false, fn_sig: None, tuple: Vec::new() };
    // A top-level script without `await` runs synchronously: a plain `Main`
    // keeps `Promise`/`Result` machinery out of the module entirely. Nested
    // fn bodies and closures never leak await outward (see top_*_has_await).
    let needs_async = top.iter().any(|s| top_stmt_has_await(&s.node));
    let (ret, is_async) = if needs_async {
        (Type { path: vec![crate::async_expand::promise_path()], args: vec![int], nullable: false, fn_sig: None, tuple: Vec::new() }, true)
    } else {
        (int, false)
    };
    let entry = Decl::Fn(FnDecl {
        docs: String::new(),
        access: Access::Internal,
        name: "Main".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        ret: Some(ret),
        throws: false,
        is_unsafe: false,
        is_async,
        is_static: false,
        is_test: false,
        is_bench: false,
        is_macro: false,
        body: FnBody::Block(Block { stmts: top }),
        attrs: Vec::new(),
    });
    module.decls.push(sp(entry, span));
}

fn lift_static_methods(module: &mut Module, diags: &mut Vec<Diagnostic>, files: &DeclFiles) {
    let mut lifted: Vec<Spanned<Decl>> = Vec::new();
    for decl in module.decls.iter_mut() {
        let from = diags.len();
        let file = decl_file(files, decl.span);
        let (name, members) = match &mut decl.node {
            Decl::Class { name, members, .. } | Decl::Struct { name, members, .. } => {
                (name.clone(), members)
            }
            _ => continue,
        };
        let mut kept: Vec<Spanned<ClassMember>> = Vec::new();
        for m in std::mem::take(members).into_iter() {
            match m.node {
                ClassMember::Method(mut f) if f.is_static => {
                    if f.is_async {
                        diags.push(
                            Diagnostic::new(Code::E108, "async static methods pending")
                                .with_span(m.span),
                        );
                        kept.push(sp(ClassMember::Method(f), m.span));
                        continue;
                    }
                    let q = format!("{name}.{}", f.name);
                    f.is_static = false;
                    f.name = q;
                    lifted.push(sp(Decl::Fn(f), m.span));
                }
                other => kept.push(sp(other, m.span)),
            }
        }
        for m in kept.iter() {
            if let ClassMember::Method(f) = &m.node {
                let q = format!("{name}.{}", f.name);
                if lifted.iter().any(|d| matches!(&d.node, Decl::Fn(g) if g.name == q)) {
                    diags.push(
                        Diagnostic::new(
                            Code::E108,
                            format!("static and instance methods share `{q}`"),
                        )
                        .with_span(m.span),
                    );
                }
            }
        }
        *members = kept;
        tag_new(diags, from, file);
    }
    module.decls.extend(lifted);
}

fn desugar_member(m: &mut ClassMember, diags: &mut Vec<Diagnostic>) {
    match m {
        ClassMember::Method(f) => desugar_body(&mut f.body, diags),
        ClassMember::Init { body, .. } => desugar_block(body, diags),
        ClassMember::Deinit(body) => desugar_block(body, diags),
        ClassMember::OnReload { body, .. } => desugar_block(body, diags),
        ClassMember::Field(f) => {
            if let Some(v) = &mut f.value {
                desugar_expr(&mut v.node, diags);
            }
        }
    }
}

fn desugar_body(b: &mut FnBody, diags: &mut Vec<Diagnostic>) {
    match b {
        FnBody::Block(block) => desugar_block(block, diags),
        FnBody::Expr(e) => desugar_expr(&mut e.node, diags),
    }
}

fn desugar_block(block: &mut Block, diags: &mut Vec<Diagnostic>) {
    let mut out: Vec<Spanned<Stmt>> = Vec::with_capacity(block.stmts.len() + 1);
    for mut st in std::mem::take(&mut block.stmts) {
        desugar_stmt(&mut st.node, diags);
        match &mut st.node {
            Stmt::Try { body, catch, finally } => {
                if let Some(mut fin) = finally.take() {
                    desugar_block(&mut fin, diags);
                    let may_complete = !block_terminates(body)
                        || catch.as_ref().is_some_and(|(_, b)| !block_terminates(b));
                    let span = st.span;
                    rewrite_propagates(body, &fin);
                    if let Some((_, cbody)) = catch {
                        rewrite_propagates(cbody, &fin);
                    }
                    rewrite_exits(body, &fin, catch.is_none(), 0);
                    if let Some((_, cbody)) = catch {
                        rewrite_exits(cbody, &fin, true, 0);
                    }
                    out.push(st);
                    if may_complete && !fin.stmts.is_empty() {
                        // A plain statement list has no scope of its own, so the
                        // one-shot `if true` keeps `finally` locals out of the
                        // enclosing block without adding a block statement.
                        out.push(sp(
                            Stmt::If {
                                cond: IfCond::Expr(sp(Expr::Bool(true), span)),
                                then: fin,
                                otherwise: None,
                            },
                            span,
                        ));
                    }
                    continue;
                }
            }
            Stmt::Switch { cases, default, .. } => {
                check_fallthrough(cases, default.is_some(), diags);
            }
            _ => {}
        }
        out.push(st);
    }
    block.stmts = out;
}

fn block_terminates(block: &Block) -> bool {
    matches!(
        block.stmts.last().map(|s| &s.node),
        Some(Stmt::Return(_) | Stmt::Throw(_) | Stmt::Break | Stmt::Continue | Stmt::Fallthrough)
    )
}

/// Runs `finally` on every path that leaves a `try` statement. The trailing
/// copy is appended by the caller for the fallthrough path; these copies run
/// before `return`, before an escaping `throw`, and before a `break` or
/// `continue` that targets a loop outside the `try`.
fn rewrite_exits(block: &mut Block, fin: &Block, throw_escapes: bool, loop_depth: usize) {
    let mut out = Vec::with_capacity(block.stmts.len());
    for mut st in std::mem::take(&mut block.stmts) {
        out.extend(exit_cleanup(&mut st, fin, throw_escapes, loop_depth));
        descend_exits(&mut st, fin, throw_escapes, loop_depth);
        out.push(st);
    }
    block.stmts = out;
}

fn exit_cleanup(
    st: &mut Spanned<Stmt>,
    fin: &Block,
    throw_escapes: bool,
    loop_depth: usize,
) -> Vec<Spanned<Stmt>> {
    let mut pre = Vec::new();
    match &mut st.node {
        Stmt::Return(value) => {
            if let Some(e) = value {
                let span = e.span;
                let name = format!("__rnx_finally_ret{}", span.start);
                let init = std::mem::replace(e, sp(Expr::Ident(name.clone()), span));
                pre.push(sp(
                    Stmt::Var { name, ty: None, value: init, is_static: false, mutable: false },
                    span,
                ));
            }
            pre.extend(fin.stmts.iter().cloned());
        }
        Stmt::Throw(value) if throw_escapes => {
            if let Some(e) = value {
                let span = e.span;
                let name = format!("__rnx_finally_err{}", span.start);
                let init = std::mem::replace(e, sp(Expr::Ident(name.clone()), span));
                pre.push(sp(
                    Stmt::Var { name, ty: None, value: init, is_static: false, mutable: false },
                    span,
                ));
            }
            pre.extend(fin.stmts.iter().cloned());
        }
        Stmt::Break | Stmt::Continue if loop_depth == 0 => {
            pre.extend(fin.stmts.iter().cloned());
        }
        _ => {}
    }
    pre
}

fn descend_exits(st: &mut Spanned<Stmt>, fin: &Block, throw_escapes: bool, loop_depth: usize) {
    match &mut st.node {
        Stmt::If { then, otherwise, .. } => {
            rewrite_exits(then, fin, throw_escapes, loop_depth);
            match otherwise {
                Some(Else::Block(b)) => rewrite_exits(b, fin, throw_escapes, loop_depth),
                Some(Else::If(inner)) => descend_exits(inner, fin, throw_escapes, loop_depth),
                None => {}
            }
        }
        Stmt::For { body, .. } | Stmt::While { body, .. } | Stmt::DoWhile { body, .. } => {
            rewrite_exits(body, fin, throw_escapes, loop_depth + 1);
        }
        Stmt::Switch { cases, default, .. } => {
            for c in cases {
                let mut b = Block { stmts: std::mem::take(&mut c.body) };
                rewrite_exits(&mut b, fin, throw_escapes, loop_depth);
                c.body = b.stmts;
            }
            if let Some(d) = default {
                let mut b = Block { stmts: std::mem::take(d) };
                rewrite_exits(&mut b, fin, throw_escapes, loop_depth);
                *d = b.stmts;
            }
        }
        Stmt::Try { body, catch, finally } => {
            let body_throws_escape = catch.is_none() && throw_escapes;
            rewrite_exits(body, fin, body_throws_escape, loop_depth);
            if let Some((_, cbody)) = catch {
                rewrite_exits(cbody, fin, throw_escapes, loop_depth);
            }
            if let Some(fbody) = finally {
                rewrite_exits(fbody, fin, throw_escapes, loop_depth);
            }
        }
        Stmt::UnsafeBlock(b) => rewrite_exits(b, fin, throw_escapes, loop_depth),
        Stmt::Guard { otherwise, .. } => {
            rewrite_exits(otherwise, fin, throw_escapes, loop_depth);
        }
        _ => {}
    }
}

/// A postfix `?` returns the error directly, bypassing the statement-level
/// cleanup copies `rewrite_exits` installs. Rebuild it as a switch so the
/// error arm runs the same `finally` body before the `?` propagates.
fn rewrite_propagates(block: &mut Block, fin: &Block) {
    for st in &mut block.stmts {
        rewrite_propagates_stmt(&mut st.node, fin);
    }
}

fn rewrite_propagates_stmt(st: &mut Stmt, fin: &Block) {
    match st {
        Stmt::Var { value, .. }
        | Stmt::DestructureTuple { value, .. }
        | Stmt::DestructureRecord { value, .. }
        | Stmt::DestructureArray { value, .. } => {
            rewrite_propagates_expr(&mut value.node, fin, value.span)
        }
        Stmt::Assign { target, value, .. } => {
            rewrite_propagates_expr(&mut target.node, fin, target.span);
            rewrite_propagates_expr(&mut value.node, fin, value.span);
        }
        Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => {
            rewrite_propagates_expr(&mut e.node, fin, e.span)
        }
        Stmt::If { cond, then, otherwise } => {
            match cond {
                IfCond::Expr(e) | IfCond::Let { value: e, .. } => {
                    rewrite_propagates_expr(&mut e.node, fin, e.span)
                }
            }
            rewrite_propagates(then, fin);
            match otherwise {
                Some(Else::Block(b)) => rewrite_propagates(b, fin),
                Some(Else::If(inner)) => {
                    rewrite_propagates_stmt(&mut inner.node, fin)
                }
                None => {}
            }
        }
        Stmt::For { iter, body, .. } => {
            rewrite_propagates_expr(&mut iter.node, fin, iter.span);
            rewrite_propagates(body, fin);
        }
        Stmt::While { cond, body } => {
            rewrite_propagates_expr(&mut cond.node, fin, cond.span);
            rewrite_propagates(body, fin);
        }
        Stmt::DoWhile { body, cond } => {
            rewrite_propagates(body, fin);
            rewrite_propagates_expr(&mut cond.node, fin, cond.span);
        }
        Stmt::Switch { scrutinee, cases, default } => {
            rewrite_propagates_expr(&mut scrutinee.node, fin, scrutinee.span);
            for c in cases {
                if let Some(g) = &mut c.guard {
                    rewrite_propagates_expr(&mut g.node, fin, g.span);
                }
                let mut b = Block { stmts: std::mem::take(&mut c.body) };
                rewrite_propagates(&mut b, fin);
                c.body = b.stmts;
            }
            if let Some(d) = default {
                let mut b = Block { stmts: std::mem::take(d) };
                rewrite_propagates(&mut b, fin);
                *d = b.stmts;
            }
        }
        Stmt::Guard { value, otherwise, .. } => {
            rewrite_propagates_expr(&mut value.node, fin, value.span);
            rewrite_propagates(otherwise, fin);
        }
        Stmt::Try { body, catch, finally } => {
            rewrite_propagates(body, fin);
            if let Some((_, cbody)) = catch {
                rewrite_propagates(cbody, fin);
            }
            if let Some(fbody) = finally {
                rewrite_propagates(fbody, fin);
            }
        }
        Stmt::UnsafeBlock(b) => rewrite_propagates(b, fin),
        Stmt::Slot { body, .. } => rewrite_propagates(body, fin),
        Stmt::Spread { value, .. } => {
            rewrite_propagates_expr(&mut value.node, fin, value.span)
        }
        Stmt::Defer(_) | Stmt::Return(None) | Stmt::Throw(None) | Stmt::Break
        | Stmt::Continue | Stmt::Fallthrough | Stmt::Pass | Stmt::Empty => {}
    }
}

fn rewrite_propagates_expr(e: &mut Expr, fin: &Block, span: Span) {
    match e {
        Expr::Propagate(inner) => {
            rewrite_propagates_expr(&mut inner.node, fin, inner.span);
            let original = std::mem::replace(&mut inner.node, Expr::Null);
            *e = guard_propagate(original, span, fin);
        }
        Expr::Binary { lhs, rhs, .. } | Expr::Coalesce { lhs, rhs } => {
            rewrite_propagates_expr(&mut lhs.node, fin, lhs.span);
            rewrite_propagates_expr(&mut rhs.node, fin, rhs.span);
        }
        Expr::Unary { rhs, .. } | Expr::Await(rhs) => {
            rewrite_propagates_expr(&mut rhs.node, fin, rhs.span)
        }
        Expr::Postfix { expr, .. }
        | Expr::Member { base: expr, .. }
        | Expr::TupleGet { base: expr, .. }
        | Expr::Cast { expr, .. }
        | Expr::Is { base: expr, .. } => rewrite_propagates_expr(&mut expr.node, fin, expr.span),
        Expr::OptChain { base, .. } => rewrite_propagates_expr(&mut base.node, fin, base.span),
        Expr::OptCall { base, args, .. } => {
            rewrite_propagates_expr(&mut base.node, fin, base.span);
            for a in args {
                rewrite_propagates_expr(&mut a.value.node, fin, a.value.span);
            }
        }
        Expr::Ternary { cond, then, otherwise } => {
            rewrite_propagates_expr(&mut cond.node, fin, cond.span);
            rewrite_propagates_expr(&mut then.node, fin, then.span);
            rewrite_propagates_expr(&mut otherwise.node, fin, otherwise.span);
        }
        Expr::Range { lo, hi, .. } => {
            rewrite_propagates_expr(&mut lo.node, fin, lo.span);
            rewrite_propagates_expr(&mut hi.node, fin, hi.span);
        }
        Expr::Call { callee, args, .. } => {
            rewrite_propagates_expr(&mut callee.node, fin, callee.span);
            for a in args {
                rewrite_propagates_expr(&mut a.value.node, fin, a.value.span);
            }
        }
        Expr::New { args, .. } => {
            for a in args {
                rewrite_propagates_expr(&mut a.value.node, fin, a.value.span);
            }
        }
        Expr::Index { base, index } => {
            rewrite_propagates_expr(&mut base.node, fin, base.span);
            rewrite_propagates_expr(&mut index.node, fin, index.span);
        }
        Expr::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(x) = p {
                    rewrite_propagates_expr(&mut x.node, fin, x.span);
                }
            }
        }
        Expr::Array(items) => {
            for i in items {
                rewrite_propagates_expr(&mut i.expr.node, fin, i.expr.span);
            }
        }
        Expr::Record(fields) => {
            for f in fields {
                let v = f.value_mut();
                rewrite_propagates_expr(&mut v.node, fin, v.span);
            }
        }
        Expr::MapLiteral(entries) => {
            for f in entries {
                let v = f.value_mut();
                rewrite_propagates_expr(&mut v.node, fin, v.span);
            }
        }
        Expr::Macro { args, .. } => {
            for a in args {
                rewrite_propagates_expr(&mut a.node, fin, a.span);
            }
        }
        Expr::UnsafeBlock(b) => rewrite_propagates(b, fin),
        Expr::Tuple(items) => {
            for i in items {
                rewrite_propagates_expr(&mut i.node, fin, i.span);
            }
        }
        Expr::Switch { scrutinee, cases, default } => {
            rewrite_propagates_expr(&mut scrutinee.node, fin, scrutinee.span);
            for c in cases {
                if let Some(g) = &mut c.guard {
                    rewrite_propagates_expr(&mut g.node, fin, g.span);
                }
                match &mut c.body {
                    SwitchExprBody::Expr(e) => {
                        rewrite_propagates_expr(&mut e.node, fin, e.span)
                    }
                    SwitchExprBody::Block(b) => rewrite_propagates(b, fin),
                }
            }
            if let Some(d) = default {
                match d {
                    SwitchExprBody::Expr(e) => {
                        rewrite_propagates_expr(&mut e.node, fin, e.span)
                    }
                    SwitchExprBody::Block(b) => rewrite_propagates(b, fin),
                }
            }
        }
        Expr::Closure { .. }
        | Expr::Ident(_)
        | Expr::This
        | Expr::Super
        | Expr::Bool(_)
        | Expr::Null
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::ImplicitMember(_) => {}
    }
}

fn guard_propagate(inner: Expr, span: Span, fin: &Block) -> Expr {
    let ok = format!("__rnx_q_ok{}", span.start);
    let err = format!("__rnx_q_err{}", span.start);
    let ok_pattern = Pattern::Enum {
        path: vec!["Result".to_string(), "Ok".to_string()],
        args: vec![Pattern::Literal(sp(Expr::Ident(ok.clone()), span))],
    };
    let err_pattern = Pattern::Enum {
        path: vec!["Result".to_string(), "Err".to_string()],
        args: vec![Pattern::Literal(sp(Expr::Ident(err.clone()), span))],
    };
    let variant = |field: &str| {
        Box::new(sp(
            Expr::Member {
                base: Box::new(sp(Expr::Ident("Result".to_string()), span)),
                field: field.to_string(),
            },
            span,
        ))
    };
    let ok_call = Expr::Call {
        callee: variant("Ok"),
        type_args: Vec::new(),
        args: vec![CallArg { name: None, value: sp(Expr::Ident(ok), span) }],
        trailing: None,
    };
    let err_call = Expr::Call {
        callee: variant("Err"),
        type_args: Vec::new(),
        args: vec![CallArg { name: None, value: sp(Expr::Ident(err), span) }],
        trailing: None,
    };
    let mut stmts = fin.stmts.clone();
    stmts.push(sp(Stmt::Expr(sp(err_call, span)), span));
    Expr::Propagate(Box::new(sp(
        Expr::Switch {
            scrutinee: Box::new(sp(inner, span)),
            cases: vec![
                SwitchExprCase {
                    pattern: ok_pattern,
                    guard: None,
                    body: SwitchExprBody::Expr(Box::new(sp(ok_call, span))),
                },
                SwitchExprCase {
                    pattern: err_pattern,
                    guard: None,
                    body: SwitchExprBody::Block(Block { stmts }),
                },
            ],
            default: None,
        },
        span,
    )))
}

fn check_fallthrough(cases: &[SwitchCase], has_default: bool, diags: &mut Vec<Diagnostic>) {
    for (i, case) in cases.iter().enumerate() {
        let falls_into = i + 1 < cases.len() || has_default;
        if !falls_into || case.body.is_empty() && !falls_into {
            continue;
        }
        if case.body.is_empty() {
            diags.push(
                Diagnostic::new(Code::E108, "empty `case` falls through; add a body or `fallthrough;`")
                    .with_hint("each `case` auto-breaks; cross-case flow needs explicit `fallthrough;`"),
            );
            continue;
        }
        let terminates = matches!(
            case.body.last().map(|s| &s.node),
            Some(
                Stmt::Break
                    | Stmt::Continue
                    | Stmt::Return(_)
                    | Stmt::Throw(_)
                    | Stmt::Fallthrough
                    | Stmt::Pass
                    | Stmt::Empty
            )
        );
        if !terminates && fallthrough_has_effect(cases, i, has_default) {
            diags.push(
                Diagnostic::new(
                    Code::E108,
                    "implicit fallthrough; terminate the `case` or add `fallthrough;`",
                )
                .with_hint("each `case` auto-breaks; cross-case flow needs explicit `fallthrough;`"),
            );
        }
    }
}

fn fallthrough_has_effect(cases: &[SwitchCase], i: usize, has_default: bool) -> bool {
    if i + 1 < cases.len() {
        return cases[i + 1].body.iter().any(|s| !matches!(s.node, Stmt::Pass | Stmt::Empty));
    }
    has_default
}

fn desugar_stmt(st: &mut Stmt, diags: &mut Vec<Diagnostic>) {
    match st {
        Stmt::Var { value, .. } => desugar_expr(&mut value.node, diags),
        Stmt::Assign { target, value, .. } => {
            desugar_expr(&mut target.node, diags);
            desugar_expr(&mut value.node, diags);
        }
        Stmt::Expr(e) | Stmt::Assert(e) | Stmt::Return(Some(e)) | Stmt::Throw(Some(e)) => {
            desugar_expr(&mut e.node, diags)
        }
        Stmt::If {
            cond,
            then,
            otherwise,
        } => {
            match cond {
                IfCond::Expr(e) | IfCond::Let { value: e, .. } => desugar_expr(&mut e.node, diags),
            }
            desugar_block(then, diags);
            if let Some(Else::Block(b)) = otherwise {
                desugar_block(b, diags);
            }
        }
        Stmt::For { iter, body, .. } => {
            desugar_expr(&mut iter.node, diags);
            desugar_block(body, diags);
        }
        Stmt::While { cond, body } => {
            desugar_expr(&mut cond.node, diags);
            desugar_block(body, diags);
        }
        Stmt::DoWhile { body, cond } => {
            desugar_block(body, diags);
            desugar_expr(&mut cond.node, diags);
        }
        Stmt::Switch {
            scrutinee,
            cases,
            default,
        } => {
            desugar_expr(&mut scrutinee.node, diags);
            for c in cases {
                if let Some(g) = &mut c.guard {
                    desugar_expr(&mut g.node, diags);
                }
                let mut b = Block {
                    stmts: std::mem::take(&mut c.body),
                };
                desugar_block(&mut b, diags);
                c.body = b.stmts;
            }
            if let Some(d) = default {
                let mut b = Block {
                    stmts: std::mem::take(d),
                };
                desugar_block(&mut b, diags);
                *d = b.stmts;
            }
        }
        Stmt::Defer(b) => desugar_block(b, diags),
        Stmt::UnsafeBlock(b) => desugar_block(b, diags),
        Stmt::Guard { value, otherwise, .. } => {
            desugar_expr(&mut value.node, diags);
            desugar_block(otherwise, diags);
        }
        Stmt::Try { body, catch, .. } => {
            desugar_block(body, diags);
            if let Some((_, b)) = catch {
                desugar_block(b, diags);
            }
        }
        _ => {}
    }
}

fn desugar_expr(e: &mut Expr, diags: &mut Vec<Diagnostic>) {
    match e {
        Expr::Binary { lhs, rhs, .. } => {
            desugar_expr(&mut lhs.node, diags);
            desugar_expr(&mut rhs.node, diags);
        }
        Expr::Is { base, .. } => {
            desugar_expr(&mut base.node, diags);
        }
        Expr::Unary { rhs, .. } => desugar_expr(&mut rhs.node, diags),
        Expr::Postfix { expr, .. } => desugar_expr(&mut expr.node, diags),
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            desugar_expr(&mut cond.node, diags);
            desugar_expr(&mut then.node, diags);
            desugar_expr(&mut otherwise.node, diags);
        }
        Expr::Range { lo, hi, .. } => {
            desugar_expr(&mut lo.node, diags);
            desugar_expr(&mut hi.node, diags);
        }
        Expr::Call {
            callee,
            args,
            trailing,
            ..
        } => {
            desugar_expr(&mut callee.node, diags);
            for a in args {
                desugar_expr(&mut a.value.node, diags);
            }
            if let Some(b) = trailing {
                desugar_block(b, diags);
            }
        }
        Expr::Index { base, index } => {
            desugar_expr(&mut base.node, diags);
            desugar_expr(&mut index.node, diags);
        }
        Expr::Member { base, .. } => desugar_expr(&mut base.node, diags),
        Expr::Coalesce { lhs, rhs } => {
            desugar_expr(&mut lhs.node, diags);
            desugar_expr(&mut rhs.node, diags);
        }
        Expr::OptChain { base, .. } => desugar_expr(&mut base.node, diags),
        Expr::OptCall { base, args, .. } => {
            desugar_expr(&mut base.node, diags);
            for a in args {
                desugar_expr(&mut a.value.node, diags);
            }
        }
        Expr::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(e) = p {
                    desugar_expr(&mut e.node, diags);
                }
            }
        }
        Expr::Array(items) => {
            for i in items {
                desugar_expr(&mut i.expr.node, diags);
            }
        }
        Expr::Record(fields) => {
            for e in fields {
                desugar_expr(&mut e.value_mut().node, diags);
            }
        }
        Expr::MapLiteral(entries) => {
            for e in entries {
                desugar_expr(&mut e.value_mut().node, diags);
            }
        }
        Expr::Macro { args, .. } => {
            for a in args {
                desugar_expr(&mut a.node, diags);
            }
        }
        Expr::Closure { body, .. } => desugar_body(body, diags),
        Expr::UnsafeBlock(b) => desugar_block(b, diags),
        _ => {}
    }
}
