use crate::ast as A;
use crate::parser::Parser;
use diagnostics::Span;
use std::path::{Path, PathBuf};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LintSeverity {
    Warning,
    Error,
}

#[derive(Clone, Debug)]
pub struct LintDiagnostic {
    pub code: &'static str,
    pub message: String,
    pub span: Span,
    pub file: Option<PathBuf>,
    pub severity: LintSeverity,
    pub fix_hint: Option<String>,
}

fn warn(
    code: &'static str,
    message: String,
    span: Span,
    file: Option<PathBuf>,
    fix_hint: Option<String>,
) -> LintDiagnostic {
    LintDiagnostic {
        code,
        message,
        span,
        file,
        severity: LintSeverity::Warning,
        fix_hint,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum BindingKind {
    Var,
    Param,
}

struct Binding {
    name: String,
    span: Span,
    kind: BindingKind,
    read: bool,
}

struct Walker {
    file: Option<PathBuf>,
    out: Vec<LintDiagnostic>,
    frames: Vec<Vec<Binding>>,
}

impl Walker {
    fn declare(&mut self, name: &str, span: Span, kind: BindingKind) {
        if let Some(frame) = self.frames.last_mut() {
            frame.push(Binding {
                name: name.to_string(),
                span,
                kind,
                read: false,
            });
        }
    }

    fn mark_read(&mut self, name: &str) {
        for frame in self.frames.iter_mut().rev() {
            if let Some(binding) = frame.iter_mut().rev().find(|b| b.name == name) {
                binding.read = true;
                return;
            }
        }
    }

    fn pop_frame(&mut self) {
        if let Some(frame) = self.frames.pop() {
            for binding in &frame {
                if !binding.read && !binding.name.starts_with('_') {
                    let dupes = frame.iter().filter(|b| b.name == binding.name).take(2).count() > 1;
                    let outer = self.shadowed_decl(&binding.name);
                    let (code, message) = match binding.kind {
                        BindingKind::Var => ("L001", format!("unused variable `{}`", binding.name)),
                        BindingKind::Param => ("L002", format!("unused parameter `{}`", binding.name)),
                    };
                    self.out.push(warn(
                        code,
                        message,
                        binding.span,
                        self.file.clone(),
                        (!(dupes || outer))
                            .then(|| format!("prefix `{}` with `_` to silence", binding.name)),
                    ));
                }
            }
        }
    }

    fn shadowed_decl(&self, name: &str) -> bool {
        self.frames
            .iter()
            .flat_map(|frame| frame.iter())
            .filter(|b| b.name == name)
            .take(2)
            .count()
            > 1
    }

    fn walk_expr(&mut self, expr: &A::Spanned<A::Expr>) {
        match &expr.node {
            A::Expr::Ident(name) => self.mark_read(name),
            A::Expr::Is { base, .. } => {
                self.walk_expr(base);
            }
            A::Expr::This
            | A::Expr::Super
            | A::Expr::Bool(_)
            | A::Expr::Null
            | A::Expr::Int(_)
            | A::Expr::Float(_) => {}
            A::Expr::Interp(parts) => {
                for part in parts {
                    if let A::InterpPart::Expr(inner) = part {
                        self.walk_expr(inner);
                    }
                }
            }
            A::Expr::Array(items) => {
                for item in items {
                    self.walk_expr(&item.expr);
                }
            }
            A::Expr::Record(fields) => {
                for e in fields {
                    let value = e.value();
                    self.walk_expr(value);
                }
            }
            A::Expr::MapLiteral(entries) => {
                for e in entries {
                    let value = e.value();
                    self.walk_expr(value);
                }
            }
            A::Expr::Binary { lhs, rhs, .. } | A::Expr::Range { lo: lhs, hi: rhs, .. } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            A::Expr::Coalesce { lhs, rhs } => {
                self.walk_expr(lhs);
                self.walk_expr(rhs);
            }
            A::Expr::OptChain { base, .. } => self.walk_expr(base),
            A::Expr::OptCall { base, args, .. } => {
                self.walk_expr(base);
                for arg in args {
                    self.walk_expr(&arg.value);
                }
            }
            A::Expr::Unary { rhs, .. } | A::Expr::Postfix { expr: rhs, .. } | A::Expr::Await(rhs) | A::Expr::Propagate(rhs) | A::Expr::Cast { expr: rhs, .. } => {
                self.walk_expr(rhs);
            }
            A::Expr::Tuple(items) => {
                for it in items {
                    self.walk_expr(it);
                }
            }
            A::Expr::TupleGet { base, .. } => {
                self.walk_expr(base);
            }
            A::Expr::Switch { scrutinee, cases, default } => {
                self.walk_expr(scrutinee);
                for c in cases {
                    if let Some(g) = &c.guard {
                        self.walk_expr(g);
                    }
                    match &c.body {
                        A::SwitchExprBody::Expr(e) => self.walk_expr(e),
                        A::SwitchExprBody::Block(b) => self.walk_block(b, false),
                    }
                }
                if let Some(d) = default {
                    match d {
                        A::SwitchExprBody::Expr(e) => self.walk_expr(e),
                        A::SwitchExprBody::Block(b) => self.walk_block(b, false),
                    }
                }
            }
            A::Expr::Ternary { cond, then, otherwise } => {
                self.walk_expr(cond);
                self.walk_expr(then);
                self.walk_expr(otherwise);
            }
            A::Expr::Call { callee, args, trailing, .. } => {
                self.walk_expr(callee);
                for arg in args {
                    self.walk_expr(&arg.value);
                }
                if let Some(block) = trailing {
                    self.walk_block(block, false);
                }
            }
            A::Expr::New { args, .. } => {
                for arg in args {
                    self.walk_expr(&arg.value);
                }
            }
            A::Expr::Index { base, index } => {
                self.walk_expr(base);
                self.walk_expr(index);
            }
            A::Expr::Member { base, .. } => self.walk_expr(base),
            A::Expr::ImplicitMember(_) => {}
            A::Expr::Macro { args, .. } => {
                for arg in args {
                    self.walk_expr(arg);
                }
            }
            A::Expr::Closure { params, body, .. } => {
                self.walk_fn_like(params, body, false, expr.span);
            }
            A::Expr::UnsafeBlock(block) => {
                self.check_empty(block, expr.span);
                self.walk_block(block, false);
            }
        }
    }

    fn walk_fn_like(
        &mut self,
        params: &[A::Param],
        body: &A::FnBody,
        abstract_trait: bool,
        fallback: Span,
    ) {
        self.frames.push(Vec::new());
        for param in params {
            if let Some(default) = &param.default {
                self.walk_expr(default);
            }
            if !abstract_trait {
                self.declare(&param.name, fallback, BindingKind::Param);
            }
        }
        match body {
            A::FnBody::Block(block) => self.walk_block(block, false),
            A::FnBody::Expr(expr) => self.walk_expr(expr),
        }
        self.pop_frame();
    }

    fn check_empty(&mut self, block: &A::Block, span: Span) {
        if block.stmts.is_empty() {
            self.out.push(warn(
                "L005",
                "empty block".to_string(),
                span,
                self.file.clone(),
                Some("remove the block or add a `pass` statement".to_string()),
            ));
        }
    }

    fn walk_block(&mut self, block: &A::Block, dead: bool) {
        self.frames.push(Vec::new());
        let mut unreachable = dead;
        for stmt in &block.stmts {
            if unreachable {
                self.out.push(warn(
                    "L003",
                    "unreachable statement".to_string(),
                    stmt.span,
                    self.file.clone(),
                    None,
                ));
            }
            self.walk_stmt(stmt, unreachable);
            match &stmt.node {
                A::Stmt::Return(_) | A::Stmt::Throw(_) | A::Stmt::Break | A::Stmt::Continue => {
                    unreachable = true;
                }
                _ => {}
            }
        }
        self.pop_frame();
    }

    fn walk_stmt(&mut self, stmt: &A::Spanned<A::Stmt>, dead: bool) {
        match &stmt.node {
            A::Stmt::Var { name, value, .. } => {
                self.walk_expr(value);
                if !dead && !name.starts_with('_') {
                    self.declare(name, stmt.span, BindingKind::Var);
                }
            }
            A::Stmt::DestructureTuple { names, value, .. } => {
                self.walk_expr(value);
                if !dead {
                    for n in names {
                        if !n.starts_with('_') {
                            self.declare(n, stmt.span, BindingKind::Var);
                        }
                    }
                }
            }
            A::Stmt::DestructureRecord { fields, rest, value, .. } => {
                self.walk_expr(value);
                if !dead {
                    for local in fields.iter().map(|(_, l)| l).chain(rest.iter()) {
                        if !local.starts_with('_') {
                            self.declare(local, stmt.span, BindingKind::Var);
                        }
                    }
                }
            }
            A::Stmt::DestructureArray { names, rest, value, .. } => {
                self.walk_expr(value);
                if !dead {
                    for n in names.iter().chain(rest.iter()) {
                        if !n.starts_with('_') {
                            self.declare(n, stmt.span, BindingKind::Var);
                        }
                    }
                }
            }
            A::Stmt::Assign { target, value, .. } => {
                self.walk_expr(target);
                self.walk_expr(value);
            }
            A::Stmt::Expr(expr) | A::Stmt::Assert(expr) => self.walk_expr(expr),
            A::Stmt::If { cond, then, otherwise } => {
                match cond {
                    A::IfCond::Expr(expr) => self.walk_expr(expr),
                    A::IfCond::Let { name, value } => {
                        self.walk_expr(value);
                        self.mark_read(name);
                    }
                }
                self.check_empty(then, stmt.span);
                self.walk_block(then, dead);
                match otherwise {
                    Some(A::Else::Block(block)) => {
                        self.check_empty(block, stmt.span);
                        self.walk_block(block, dead);
                    }
                    Some(A::Else::If(inner)) => self.walk_stmt(inner, dead),
                    None => {}
                }
            }
            A::Stmt::For { binding, iter, body } => {
                self.walk_expr(iter);
                match binding {
                    A::ForBinding::One(name) => self.mark_read(name),
                    A::ForBinding::Many(names) => {
                        for name in names {
                            self.mark_read(name);
                        }
                    }
                }
                self.check_empty(body, stmt.span);
                self.walk_block(body, dead);
            }
            A::Stmt::While { cond, body } | A::Stmt::DoWhile { cond, body } => {
                self.walk_expr(cond);
                self.check_empty(body, stmt.span);
                self.walk_block(body, dead);
            }
            A::Stmt::Switch { scrutinee, cases, default } => {
                self.walk_expr(scrutinee);
                for case in cases {
                    match &case.pattern {
                        A::Pattern::Literal(expr) => self.walk_expr(expr),
                        A::Pattern::Range { lo, hi, .. } => {
                            self.walk_expr(lo);
                            self.walk_expr(hi);
                        }
                        A::Pattern::Enum { args, .. } => {
                            for arg in args {
                                if let A::Pattern::Literal(expr) = arg {
                                    self.walk_expr(expr);
                                }
                            }
                        }
                        _ => {}
                    }
                    if let Some(guard) = &case.guard {
                        self.walk_expr(guard);
                    }
                    self.walk_case_body(&case.body, dead);
                }
                if let Some(stmts) = default {
                    self.walk_case_body(stmts, dead);
                }
            }
            A::Stmt::Return(expr) | A::Stmt::Throw(expr) => {
                if let Some(expr) = expr {
                    self.walk_expr(expr);
                }
            }
            A::Stmt::Defer(block) => {
                self.check_empty(block, stmt.span);
                self.walk_block(block, dead);
            }
            A::Stmt::Guard { name, value, otherwise } => {
                self.walk_expr(value);
                self.mark_read(name);
                self.check_empty(otherwise, stmt.span);
                self.walk_block(otherwise, dead);
            }
            A::Stmt::Try { body, catch, finally } => {
                self.check_empty(body, stmt.span);
                self.walk_block(body, dead);
                if let Some((name, block)) = catch {
                    self.mark_read(name);
                    self.check_empty(block, stmt.span);
                    self.walk_block(block, dead);
                }
                if let Some(block) = finally {
                    self.check_empty(block, stmt.span);
                    self.walk_block(block, dead);
                }
            }
            A::Stmt::UnsafeBlock(block) => {
                self.check_empty(block, stmt.span);
                self.walk_block(block, dead);
            }
            A::Stmt::Break | A::Stmt::Continue | A::Stmt::Fallthrough | A::Stmt::Pass | A::Stmt::Empty => {}
        }
    }

    fn walk_case_body(&mut self, stmts: &[A::Spanned<A::Stmt>], dead: bool) {
        let mut unreachable = dead;
        for stmt in stmts {
            if unreachable {
                self.out.push(warn(
                    "L003",
                    "unreachable statement".to_string(),
                    stmt.span,
                    self.file.clone(),
                    None,
                ));
            }
            self.walk_stmt(stmt, unreachable);
            match &stmt.node {
                A::Stmt::Return(_) | A::Stmt::Throw(_) | A::Stmt::Break | A::Stmt::Continue => {
                    unreachable = true;
                }
                _ => {}
            }
        }
    }

    fn walk_member(&mut self, member: &A::Spanned<A::ClassMember>, abstract_trait: bool) {
        match &member.node {
            A::ClassMember::Field(field) => {
                if let Some(value) = &field.value {
                    self.walk_expr(value);
                }
            }
            A::ClassMember::Method(func) => self.walk_fn(func, abstract_trait, member.span),
            A::ClassMember::Init { params, body } => {
                self.frames.push(Vec::new());
                for param in params {
                    self.declare(&param.name, member.span, BindingKind::Param);
                }
                self.walk_block(body, false);
                self.pop_frame();
            }
            A::ClassMember::Deinit(body) => {
                self.frames.push(Vec::new());
                self.walk_block(body, false);
                self.pop_frame();
            }
            A::ClassMember::OnReload { params, body } => {
                self.frames.push(Vec::new());
                for param in params {
                    self.declare(&param.name, member.span, BindingKind::Param);
                }
                self.walk_block(body, false);
                self.pop_frame();
            }
        }
    }

    fn walk_fn(&mut self, func: &A::FnDecl, abstract_trait: bool, fallback: Span) {
        self.walk_fn_like(&func.params, &func.body, abstract_trait, fallback);
    }

    fn walk_module(&mut self, module: &A::Module) {
        for decl in &module.decls {
            match &decl.node {
                A::Decl::Fn(func) => {
                    self.check_pub_docs(&func.access, &func.docs, "function", &func.name, decl.span);
                    self.walk_fn(func, false, decl.span);
                }
                A::Decl::Class { access, name, members, docs, .. } => {
                    self.check_pub_docs(access, docs, "class", name, decl.span);
                    for member in members {
                        self.walk_member(member, false);
                    }
                }
                A::Decl::Struct { access, name, members, docs, .. } => {
                    self.check_pub_docs(access, docs, "struct", name, decl.span);
                    for member in members {
                        self.walk_member(member, false);
                    }
                }
                A::Decl::Trait { access, name, members, docs, .. } => {
                    self.check_pub_docs(access, docs, "trait", name, decl.span);
                    for member in members {
                        let abstract_method = match &member.node {
                            A::ClassMember::Method(func) => match &func.body {
                                A::FnBody::Block(block) => block.stmts.is_empty(),
                                A::FnBody::Expr(_) => false,
                            },
                            _ => false,
                        };
                        self.walk_member(member, abstract_method);
                    }
                }
                A::Decl::Interface { access, name, members, docs, .. } => {
                    self.check_pub_docs(access, docs, "interface", name, decl.span);
                    for member in members {
                        let abstract_method = match &member.node {
                            A::ClassMember::Method(func) => match &func.body {
                                A::FnBody::Block(block) => block.stmts.is_empty(),
                                A::FnBody::Expr(_) => false,
                            },
                            _ => false,
                        };
                        self.walk_member(member, abstract_method);
                    }
                }
                A::Decl::Extension { members, docs, .. } => {
                    for member in members {
                        self.walk_member(member, false);
                    }
                    let _ = docs;
                }
                A::Decl::Enum { access, name, docs, .. } => {
                    self.check_pub_docs(access, docs, "enum", name, decl.span);
                }
                A::Decl::Const { value, .. } => {
                    self.walk_expr(value);
                }
                A::Decl::Stmt(s) => {
                    self.walk_stmt(s, false);
                }
                A::Decl::Record { .. }
                | A::Decl::Import(..)
                | A::Decl::ExportFrom(..)
                | A::Decl::ExportList(..)
                | A::Decl::ExportDefault(..) => {}
            }
        }
    }

    fn check_pub_docs(&mut self, access: &A::Access, docs: &str, kind: &str, name: &str, span: Span) {
        if *access == A::Access::Export && docs.is_empty() {
            self.out.push(warn(
                "L004",
                format!("public {kind} `{name}` is missing a doc comment"),
                span,
                self.file.clone(),
                Some("add a `/** */` doc comment above the item".to_string()),
            ));
        }
    }
}

pub fn lint_source(src: &str) -> Vec<LintDiagnostic> {
    lint_module_source(src, None)
}

fn lint_module_source(src: &str, file: Option<PathBuf>) -> Vec<LintDiagnostic> {
    let module = match Parser::parse_module(src) {
        Ok(m) => m,
        Err(e) => {
            return vec![LintDiagnostic {
                code: e.code.as_str(),
                message: format!("cannot lint: {}. Run `rnx check` for details", e.message),
                span: e.span.unwrap_or(Span { start: 0, end: 0 }),
                file,
                severity: LintSeverity::Error,
                fix_hint: None,
            }];
        }
    };
    let mut walker = Walker {
        file,
        out: Vec::new(),
        frames: Vec::new(),
    };
    walker.walk_module(&module);
    walker.out.sort_by(|a, b| a.span.start.cmp(&b.span.start).then(a.code.cmp(b.code)));
    walker.out
}

fn discover_rnx(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if root.is_file() {
        if root.extension().is_some_and(|ext| ext == "rnx") {
            out.push(root.to_path_buf());
        }
        return out;
    }
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                let name = entry.file_name().to_string_lossy().into_owned();
                if name == "target" || name == ".git" || name == ".rnx" || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
            } else if path.extension().is_some_and(|ext| ext == "rnx") {
                out.push(path);
            }
        }
    }
    out.sort();
    out
}

pub fn lint_package(root: &Path) -> Vec<LintDiagnostic> {
    let mut out = Vec::new();
    for path in discover_rnx(root) {
        let src = match std::fs::read_to_string(&path) {
            Ok(src) => src,
            Err(_) => continue,
        };
        out.extend(lint_module_source(&src, Some(path)));
    }
    out.sort_by(|a, b| {
        a.file
            .cmp(&b.file)
            .then(a.span.start.cmp(&b.span.start))
            .then(a.code.cmp(b.code))
    });
    out
}
