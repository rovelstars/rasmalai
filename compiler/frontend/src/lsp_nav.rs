use crate::ast as A;
use diagnostics::Span;

pub struct SymbolTarget {
    pub span: Span,
    pub signature: String,
    pub docs: String,
}

pub fn position_to_offset(source: &str, line: usize, character: usize) -> Option<usize> {
    let mut start = 0usize;
    for (idx, text) in source.split('\n').enumerate() {
        if idx == line {
            if character > text.chars().count() {
                return None;
            }
            let mut offset = start;
            for (count, ch) in text.chars().enumerate() {
                if count >= character {
                    break;
                }
                offset += ch.len_utf8();
            }
            return Some(offset);
        }
        start += text.len() + 1;
    }
    None
}

pub fn offset_to_position(source: &str, offset: usize) -> (usize, usize) {
    let offset = offset.min(source.len());
    let head = &source[..offset];
    let line = head.bytes().filter(|&b| b == b'\n').count();
    let col = head.rfind('\n').map(|i| offset - i - 1).unwrap_or(offset);
    (line, col)
}

pub(crate) fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn find_ident(src: &str, from: usize, name: &str, skip: &[&str]) -> Option<Span> {
    let bytes = src.as_bytes();
    let mut i = from.min(bytes.len());
    while i < bytes.len() {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let s = i;
            while i < bytes.len() && is_word_char(bytes[i]) {
                i += 1;
            }
            let word = &src[s..i];
            if word == name {
                return Span::new(s as u32, i as u32);
            }
            if !skip.contains(&word) {
                return None;
            }
        } else {
            i += 1;
        }
    }
    None
}

pub(crate) fn decl_name_span(src: &str, decl_start: usize, name: &str) -> Option<Span> {
    find_ident(src, decl_start, name, &["pub", "public", "export", "private", "static", "fn", "async", "unsafe", "class", "struct", "trait", "interface", "enum", "record", "const", "let", "test", "bench"])
}

pub(crate) fn render_type(ty: &A::Type) -> String {
    let mut out = ty.path.join(".");
    if !ty.args.is_empty() {
        let args: Vec<String> = ty.args.iter().map(render_type).collect();
        out.push('<');
        out.push_str(&args.join(", "));
        out.push('>');
    }
    if ty.nullable {
        out.push('?');
    }
    out
}

pub(crate) fn render_params(params: &[A::Param]) -> String {
    let parts: Vec<String> = params
        .iter()
        .map(|p| match &p.ty {
            Some(ty) => format!("{}: {}", p.name, render_type(ty)),
            None => p.name.clone(),
        })
        .collect();
    parts.join(", ")
}

pub(crate) fn fn_signature(func: &A::FnDecl) -> String {
    let mut out = format!("fn {}({})", func.name, render_params(&func.params));
    if let Some(ret) = &func.ret {
        out.push_str(&format!(": {}", render_type(ret)));
    }
    out
}

fn infer_literal(expr: &A::Expr) -> Option<&'static str> {
    match expr {
        A::Expr::Int(_) => Some("Int"),
        A::Expr::Float(_) => Some("Float"),
        A::Expr::Bool(_) => Some("Bool"),
        A::Expr::Null => Some("null"),
        A::Expr::Array(_) => Some("Array"),
        A::Expr::Interp(_) => Some("String"),
        _ => None,
    }
}

struct Def {
    name: String,
    span: Span,
    signature: String,
    docs: String,
    scope: usize,
    kind: DefKind,
    class_idx: Option<usize>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum DefKind {
    Var,
    Param,
    Fn,
    Method,
    Field,
    Type,
    Const,
}

struct Use {
    name: String,
    span: Span,
    scope: usize,
}

struct MemberUse {
    field: String,
    span: Span,
    class_ctx: Option<usize>,
}

struct ThisUse {
    span: Span,
    class_ctx: Option<usize>,
}

struct ClassInfo {
    name: String,
    docs: String,
}

struct Collector<'a> {
    src: &'a str,
    defs: Vec<Def>,
    uses: Vec<Use>,
    member_uses: Vec<MemberUse>,
    this_uses: Vec<ThisUse>,
    classes: Vec<ClassInfo>,
    class_stack: Vec<usize>,
    parents: Vec<Option<usize>>,
    stack: Vec<usize>,
    cursor: usize,
}

impl<'a> Collector<'a> {
    fn push_scope(&mut self) -> usize {
        let parent = self.stack.last().copied();
        let id = self.parents.len();
        self.parents.push(parent);
        self.stack.push(id);
        id
    }

    fn pop_scope(&mut self) {
        self.stack.pop();
    }

    fn current(&self) -> usize {
        self.stack.last().copied().unwrap_or(0)
    }

    fn add_def(&mut self, name: &str, span: Span, signature: String, docs: &str) {
        self.add_def_full(name, span, signature, docs, DefKind::Var, None);
    }

    fn add_def_full(
        &mut self,
        name: &str,
        span: Span,
        signature: String,
        docs: &str,
        kind: DefKind,
        class_idx: Option<usize>,
    ) {
        self.defs.push(Def {
            name: name.to_string(),
            span,
            signature,
            docs: docs.to_string(),
            scope: self.current(),
            kind,
            class_idx,
        });
    }

    fn add_use(&mut self, name: &str) {
        let mut from = self.cursor;
        let mut found = None;
        while let Some((word, span)) = next_ident(self.src, from) {
            from = span.end as usize;
            if word != name {
                continue;
            }
            let inside_def = self
                .defs
                .iter()
                .any(|d| d.span.start <= span.start && span.end <= d.span.end);
            let preceded_by_dot = match span.start as usize {
                0 => false,
                s => {
                    let bytes = self.src.as_bytes();
                    let mut j = s - 1;
                    while j > 0 && (bytes[j] == b' ' || bytes[j] == b'\t') {
                        j -= 1;
                    }
                    bytes[j] == b'.'
                }
            };
            if inside_def || preceded_by_dot {
                continue;
            }
            found = Some(span);
            break;
        }
        let span = found.unwrap_or(Span { start: 0, end: 0 });
        if span.end > 0 {
            self.cursor = span.end as usize;
        }
        self.uses.push(Use {
            name: name.to_string(),
            span,
            scope: self.current(),
        });
    }

    fn visible(&self, name: &str, scope: usize) -> Option<&Def> {
        let mut scope = Some(scope);
        while let Some(id) = scope {
            if let Some(def) = self.defs.iter().rev().find(|d| {
                d.name == name
                    && d.scope == id
                    && !matches!(d.kind, DefKind::Field | DefKind::Method)
            }) {
                return Some(def);
            }
            scope = self.parents.get(id).copied().flatten();
        }
        None
    }

    fn find_member(&self, field: &str, class_ctx: Option<usize>) -> Option<&Def> {
        match class_ctx {
            Some(class) => self.defs.iter().rev().find(|d| {
                d.name == field
                    && d.class_idx == Some(class)
                    && matches!(d.kind, DefKind::Field | DefKind::Method)
            }),
            None => {
                let mut found = None;
                for def in self.defs.iter().rev() {
                    if def.name == field
                        && matches!(def.kind, DefKind::Field | DefKind::Method)
                        && def.class_idx.is_some()
                    {
                        match found {
                            Some(_) => return None,
                            None => found = Some(def),
                        }
                    }
                }
                found
            }
        }
    }

    fn scan_dotted(&mut self, name: &str) -> Span {
        let mut from = self.cursor;
        let mut found = Span { start: 0, end: 0 };
        while let Some((word, span)) = next_ident(self.src, from) {
            from = span.end as usize;
            if word != name {
                continue;
            }
            if self.preceded_by_dot(span) {
                found = span;
                break;
            }
        }
        if found.end > 0 {
            self.cursor = found.end as usize;
        }
        found
    }

    fn preceded_by_dot(&self, span: Span) -> bool {
        match span.start as usize {
            0 => false,
            s => {
                let bytes = self.src.as_bytes();
                let mut j = s - 1;
                while j > 0 && (bytes[j] == b' ' || bytes[j] == b'\t') {
                    j -= 1;
                }
                bytes[j] == b'.'
            }
        }
    }

    fn scan_this(&mut self) -> Span {
        let mut from = self.cursor;
        let mut found = Span { start: 0, end: 0 };
        while let Some((word, span)) = next_ident(self.src, from) {
            from = span.end as usize;
            if word != "this" {
                continue;
            }
            let inside_def = self
                .defs
                .iter()
                .any(|d| d.span.start <= span.start && span.end <= d.span.end);
            if inside_def {
                continue;
            }
            found = span;
            break;
        }
        if found.end > 0 {
            self.cursor = found.end as usize;
        }
        found
    }

    fn scan_names_in_order(&self, names: &[String], region_start: usize, region_end: usize) -> Vec<Span> {
        let mut spans = Vec::new();
        let mut pending: Vec<&String> = names.iter().collect();
        let mut from = region_start.min(self.src.len());
        let end = region_end.min(self.src.len());
        while !pending.is_empty() {
            match next_ident(self.src, from) {
                Some((word, span)) if (span.end as usize) <= end => {
                    from = span.end as usize;
                    if pending.first().is_some_and(|n| n.as_str() == word) {
                        pending.remove(0);
                        spans.push(span);
                    }
                }
                _ => break,
            }
        }
        spans
    }

    fn collect_expr(&mut self, expr: &A::Spanned<A::Expr>) {
        match &expr.node {
            A::Expr::Ident(name) => self.add_use(name),
            A::Expr::Interp(parts) => {
                for part in parts {
                    if let A::InterpPart::Expr(inner) = part {
                        self.collect_expr(inner);
                    }
                }
            }
            A::Expr::Array(items) => {
                for item in items {
                    self.collect_expr(&item.expr);
                }
            }
            A::Expr::Record(fields) => {
                let names: Vec<Option<String>> = fields
                    .iter()
                    .map(|e| match e {
                        A::RecordEntry::Field(name, _) => Some(name.clone()),
                        A::RecordEntry::Spread(_) => None,
                    })
                    .collect();
                let present: Vec<String> = names.iter().filter_map(|n| n.clone()).collect();
                let spans = self.scan_names_in_order(
                    &present,
                    expr.span.start as usize,
                    expr.span.end as usize,
                );
                let mut si = 0usize;
                for (i, e) in fields.iter().enumerate() {
                    let (field, value) = match e {
                        A::RecordEntry::Field(n, v) => (n, v),
                        A::RecordEntry::Spread(v) => {
                            self.collect_expr(v);
                            continue;
                        }
                    };
                    if let Some(span) = spans.get(si) {
                        let signature = match infer_literal(&fields[i].value().node) {
                            Some(inferred) => format!("{field}: {inferred}"),
                            None => field.clone(),
                        };
                        self.defs.push(Def {
                            name: field.clone(),
                            span: *span,
                            signature,
                            docs: String::new(),
                            scope: self.current(),
                            kind: DefKind::Field,
                            class_idx: None,
                        });
                    }
                    si += 1;
                    self.collect_expr(value);
                }
            }
            A::Expr::MapLiteral(entries) => {
                for e in entries {
                    self.collect_expr(e.value());
                }
            }
            A::Expr::Binary { lhs, rhs, .. } | A::Expr::Range { lo: lhs, hi: rhs, .. } => {
                self.collect_expr(lhs);
                self.collect_expr(rhs);
            }
            A::Expr::Unary { rhs, .. } | A::Expr::Postfix { expr: rhs, .. } | A::Expr::Await(rhs) | A::Expr::Cast { expr: rhs, .. } => {
                self.collect_expr(rhs);
            }
            A::Expr::Is { base, .. } => {
                self.collect_expr(base);
            }
            A::Expr::Ternary { cond, then, otherwise } => {
                self.collect_expr(cond);
                self.collect_expr(then);
                self.collect_expr(otherwise);
            }
            A::Expr::Coalesce { lhs, rhs } => {
                self.collect_expr(lhs);
                self.collect_expr(rhs);
            }
            A::Expr::OptChain { base, .. } => {
                self.collect_expr(base);
            }
            A::Expr::OptCall { base, args, .. } => {
                self.collect_expr(base);
                for arg in args {
                    self.collect_expr(&arg.value);
                }
            }
            A::Expr::Call { callee, args, trailing, .. } => {
                self.collect_expr(callee);
                for arg in args {
                    self.collect_expr(&arg.value);
                }
                if let Some(block) = trailing {
                    self.collect_block(block);
                }
            }
            A::Expr::Index { base, index } => {
                self.collect_expr(base);
                self.collect_expr(index);
            }
            A::Expr::Member { base, field } => {
                if matches!(base.node, A::Expr::This) {
                    let span = self.scan_this();
                    if span.end > 0 {
                        self.this_uses.push(ThisUse {
                            span,
                            class_ctx: self.class_stack.last().copied(),
                        });
                        let field_span = self.scan_dotted(field);
                        if field_span.end > 0 {
                            self.member_uses.push(MemberUse {
                                field: field.clone(),
                                span: field_span,
                                class_ctx: self.class_stack.last().copied(),
                            });
                        }
                    }
                } else {
                    self.collect_expr(base);
                    let field_span = self.scan_dotted(field);
                    if field_span.end > 0 {
                        self.member_uses.push(MemberUse {
                            field: field.clone(),
                            span: field_span,
                            class_ctx: None,
                        });
                    }
                }
            }
            A::Expr::Macro { args, .. } => {
                for arg in args {
                    self.collect_expr(arg);
                }
            }
            A::Expr::Closure { params, body, .. } => {
                let scope = self.push_scope();
                self.collect_params(params, expr.span, scope);
                match body {
                    A::FnBody::Block(block) => self.collect_block(block),
                    A::FnBody::Expr(inner) => self.collect_expr(inner),
                }
                self.pop_scope();
            }
            A::Expr::UnsafeBlock(block) => self.collect_block(block),
            A::Expr::This => {
                let span = self.scan_this();
                if span.end > 0 {
                    self.this_uses.push(ThisUse {
                        span,
                        class_ctx: self.class_stack.last().copied(),
                    });
                }
            }
            _ => {}
        }
    }

    fn collect_params(&mut self, params: &[A::Param], fn_span: Span, scope: usize) {
        let param_spans = split_params(self.src, fn_span);
        for (i, param) in params.iter().enumerate() {
            let span = param_spans.get(i).copied().unwrap_or(fn_span);
            let signature = match &param.ty {
                Some(ty) => format!("{}: {}", param.name, render_type(ty)),
                None => param.name.clone(),
            };
            self.defs.push(Def {
                name: param.name.clone(),
                span,
                signature,
                docs: String::new(),
                scope,
                kind: DefKind::Param,
                class_idx: None,
            });
            if let Some(default) = &param.default {
                self.collect_expr(default);
            }
        }
    }

    fn collect_block(&mut self, block: &A::Block) {
        for stmt in &block.stmts {
            self.collect_stmt(stmt);
        }
    }

    fn collect_stmt(&mut self, stmt: &A::Spanned<A::Stmt>) {
        match &stmt.node {
            A::Stmt::Var { name, value, ty, .. } => {
                self.collect_expr(value);
                if let Some(span) = decl_name_span(self.src, stmt.span.start as usize, name) {
                    let signature = match ty {
                        Some(ty) => format!("let {name}: {}", render_type(ty)),
                        None => match infer_literal(&value.node) {
                            Some(inferred) => format!("let {name}: {inferred}"),
                            None => format!("let {name}"),
                        },
                    };
                    self.add_def(name, span, signature, "");
                }
            }
            A::Stmt::DestructureTuple { names, value, .. } => {
                self.collect_expr(value);
                for n in names {
                    if let Some(span) = decl_name_span(self.src, stmt.span.start as usize, n) {
                        self.add_def(n, span, format!("let {n}"), "");
                    }
                }
            }
            A::Stmt::DestructureRecord { fields, rest, value, .. } => {
                self.collect_expr(value);
                for local in fields.iter().map(|(_, l)| l).chain(rest.iter()) {
                    if let Some(span) = decl_name_span(self.src, stmt.span.start as usize, local) {
                        self.add_def(local, span, format!("let {local}"), "");
                    }
                }
            }
            A::Stmt::DestructureArray { names, rest, value, .. } => {
                self.collect_expr(value);
                for n in names.iter().chain(rest.iter()) {
                    if let Some(span) = decl_name_span(self.src, stmt.span.start as usize, n) {
                        self.add_def(n, span, format!("let {n}"), "");
                    }
                }
            }
            A::Stmt::Assign { target, value, .. } => {
                self.collect_expr(target);
                self.collect_expr(value);
            }
            A::Stmt::Expr(expr) | A::Stmt::Assert(expr) => self.collect_expr(expr),
            A::Stmt::If { cond, then, otherwise } => {
                match cond {
                    A::IfCond::Expr(expr) => self.collect_expr(expr),
                    A::IfCond::Let { value, .. } => self.collect_expr(value),
                }
                self.collect_block(then);
                match otherwise {
                    Some(A::Else::Block(block)) => self.collect_block(block),
                    Some(A::Else::If(inner)) => self.collect_stmt(inner),
                    None => {}
                }
            }
            A::Stmt::For { iter, body, .. } => {
                self.collect_expr(iter);
                self.collect_block(body);
            }
            A::Stmt::While { cond, body } | A::Stmt::DoWhile { cond, body } => {
                self.collect_expr(cond);
                self.collect_block(body);
            }
            A::Stmt::Switch { scrutinee, cases, default } => {
                self.collect_expr(scrutinee);
                for case in cases {
                    match &case.pattern {
                        A::Pattern::Literal(expr) => self.collect_expr(expr),
                        A::Pattern::Range { lo, hi, .. } => {
                            self.collect_expr(lo);
                            self.collect_expr(hi);
                        }
                        _ => {}
                    }
                    if let Some(guard) = &case.guard {
                        self.collect_expr(guard);
                    }
                    for inner in &case.body {
                        self.collect_stmt(inner);
                    }
                }
                if let Some(stmts) = default {
                    for inner in stmts {
                        self.collect_stmt(inner);
                    }
                }
            }
            A::Stmt::Return(expr) | A::Stmt::Throw(expr) => {
                if let Some(expr) = expr {
                    self.collect_expr(expr);
                }
            }
            A::Stmt::Defer(block) | A::Stmt::UnsafeBlock(block) => self.collect_block(block),
            A::Stmt::Guard { value, otherwise, .. } => {
                self.collect_expr(value);
                self.collect_block(otherwise);
            }
            A::Stmt::Try { body, catch, finally } => {
                self.collect_block(body);
                if let Some((_, block)) = catch {
                    self.collect_block(block);
                }
                if let Some(block) = finally {
                    self.collect_block(block);
                }
            }
            A::Stmt::Break | A::Stmt::Continue | A::Stmt::Fallthrough | A::Stmt::Pass | A::Stmt::Empty => {}
        }
    }

    fn collect_fn(&mut self, func: &A::FnDecl, span: Span) {
        self.collect_fn_in(func, span, None);
    }

    fn collect_fn_in(&mut self, func: &A::FnDecl, span: Span, class_idx: Option<usize>) {
        if let Some(name_span) = decl_name_span(self.src, span.start as usize, &func.name) {
            self.defs.push(Def {
                name: func.name.clone(),
                span: name_span,
                signature: fn_signature(func),
                docs: func.docs.clone(),
                scope: 0,
                kind: if class_idx.is_some() {
                    DefKind::Method
                } else {
                    DefKind::Fn
                },
                class_idx,
            });
        }
        let body_scope = self.push_scope();
        self.collect_params(&func.params, span, body_scope);
        match &func.body {
            A::FnBody::Block(block) => self.collect_block(block),
            A::FnBody::Expr(expr) => self.collect_expr(expr),
        }
        self.pop_scope();
    }

    fn collect_member(&mut self, member: &A::Spanned<A::ClassMember>, class_idx: Option<usize>) {
        match &member.node {
            A::ClassMember::Method(func) => {
                self.collect_fn_in(func, member.span, class_idx);
            }
            A::ClassMember::Init { params, body } => {
                if let Some(name_span) = decl_name_span(self.src, member.span.start as usize, "init") {
                    self.defs.push(Def {
                        name: "init".to_string(),
                        span: name_span,
                        signature: format!("init({})", render_params(params)),
                        docs: String::new(),
                        scope: self.current(),
                        kind: DefKind::Method,
                        class_idx,
                    });
                }
                let scope = self.push_scope();
                self.collect_params(params, member.span, scope);
                self.collect_block(body);
                self.pop_scope();
            }
            A::ClassMember::OnReload { params, body } => {
                if let Some(name_span) = decl_name_span(self.src, member.span.start as usize, "onReload") {
                    self.defs.push(Def {
                        name: "onReload".to_string(),
                        span: name_span,
                        signature: format!("onReload({})", render_params(params)),
                        docs: String::new(),
                        scope: self.current(),
                        kind: DefKind::Method,
                        class_idx,
                    });
                }
                let scope = self.push_scope();
                self.collect_params(params, member.span, scope);
                self.collect_block(body);
                self.pop_scope();
            }
            A::ClassMember::Deinit(body) => {
                if let Some(name_span) = decl_name_span(self.src, member.span.start as usize, "deinit") {
                    self.defs.push(Def {
                        name: "deinit".to_string(),
                        span: name_span,
                        signature: "deinit".to_string(),
                        docs: String::new(),
                        scope: self.current(),
                        kind: DefKind::Method,
                        class_idx,
                    });
                }
                self.collect_block(body);
            }
            A::ClassMember::Field(field) => {
                if let Some(name_span) = decl_name_span(self.src, member.span.start as usize, &field.name) {
                    let signature = match &field.ty {
                        Some(ty) => format!("let {}: {}", field.name, render_type(ty)),
                        None => format!("let {}", field.name),
                    };
                    self.defs.push(Def {
                        name: field.name.clone(),
                        span: name_span,
                        signature,
                        docs: field.docs.clone(),
                        scope: self.current(),
                        kind: DefKind::Field,
                        class_idx,
                    });
                }
                if let Some(value) = &field.value {
                    self.collect_expr(value);
                }
            }
        }
    }

    fn collect_module(&mut self, module: &A::Module) {
        for decl in &module.decls {
            match &decl.node {
                A::Decl::Fn(func) => self.collect_fn(func, decl.span),
                A::Decl::Class { name, members, docs, .. } => {
                    self.add_type_def(name, "class", docs, decl.span);
                    let class_idx = self.classes.len();
                    self.classes.push(ClassInfo {
                        name: name.clone(),
                        docs: docs.clone(),
                    });
                    self.class_stack.push(class_idx);
                    for member in members {
                        self.collect_member(member, Some(class_idx));
                    }
                    self.class_stack.pop();
                }
                A::Decl::Struct { name, members, docs, .. } => {
                    self.add_type_def(name, "struct", docs, decl.span);
                    let class_idx = self.classes.len();
                    self.classes.push(ClassInfo {
                        name: name.clone(),
                        docs: docs.clone(),
                    });
                    self.class_stack.push(class_idx);
                    for member in members {
                        self.collect_member(member, Some(class_idx));
                    }
                    self.class_stack.pop();
                }
                A::Decl::Trait { name, members, docs, .. } => {
                    self.add_type_def(name, "trait", docs, decl.span);
                    for member in members {
                        self.collect_member(member, None);
                    }
                }
                A::Decl::Interface { name, members, docs, .. } => {
                    self.add_type_def(name, "interface", docs, decl.span);
                    for member in members {
                        self.collect_member(member, None);
                    }
                }
                A::Decl::Extension { members, .. } => {
                    for member in members {
                        self.collect_member(member, None);
                    }
                }
                A::Decl::Enum { name, docs, members, .. } => {
                    self.add_type_def(name, "enum", docs, decl.span);
                    let variant_names: Vec<String> =
                        members.iter().map(|m| m.name.clone()).collect();
                    let spans = self.scan_names_in_order(
                        &variant_names,
                        decl.span.start as usize,
                        decl.span.end as usize,
                    );
                    for (member, span) in members.iter().zip(spans.iter()) {
                        let signature = if member.payload.is_empty() {
                            member.name.clone()
                        } else {
                            let args: Vec<String> =
                                member.payload.iter().map(render_type).collect();
                            format!("{}({})", member.name, args.join(", "))
                        };
                        self.defs.push(Def {
                            name: member.name.clone(),
                            span: *span,
                            signature,
                            docs: member.docs.clone(),
                            scope: 0,
                            kind: DefKind::Field,
                            class_idx: None,
                        });
                    }
                }
                A::Decl::Const { name, ty, value, docs, .. } => {
                    self.collect_expr(value);
                    if let Some(span) = decl_name_span(self.src, decl.span.start as usize, name) {
                        let signature = match ty {
                            Some(ty) => format!("const {name}: {}", render_type(ty)),
                            None => format!("const {name}"),
                        };
                        self.defs.push(Def {
                            name: name.clone(),
                            span,
                            signature,
                            docs: docs.clone(),
                            scope: 0,
                            kind: DefKind::Const,
                            class_idx: None,
                        });
                    }
                }
                A::Decl::Record { .. }
                | A::Decl::Import(..)
                | A::Decl::ExportFrom(..)
                | A::Decl::ExportList(..)
                | A::Decl::ExportDefault(..) => {}
                A::Decl::Stmt(s) => {
                    self.collect_stmt(s);
                }
            }
        }
    }

    fn add_type_def(&mut self, name: &str, kind: &str, docs: &str, span: Span) {
        if let Some(name_span) = decl_name_span(self.src, span.start as usize, name) {
            self.defs.push(Def {
                name: name.to_string(),
                span: name_span,
                signature: format!("{kind} {name}"),
                docs: docs.to_string(),
                scope: 0,
                kind: DefKind::Type,
                class_idx: None,
            });
        }
    }
}

fn split_params(src: &str, fn_span: Span) -> Vec<Span> {
    let bytes = src.as_bytes();
    let mut i = fn_span.start as usize;
    while i < bytes.len() && bytes[i] != b'(' {
        i += 1;
    }
    if i >= bytes.len() {
        return Vec::new();
    }
    i += 1;
    let mut spans = Vec::new();
    let mut depth = 0usize;
    let mut seg_start = i;
    let mut closed = false;
    while i < bytes.len() {
        match bytes[i] {
            b'(' | b'[' | b'{' => depth += 1,
            b')' if depth == 0 => {
                push_param(src, seg_start, i, &mut spans);
                closed = true;
                break;
            }
            b')' | b']' | b'}' => depth = depth.saturating_sub(1),
            b',' if depth == 0 => {
                push_param(src, seg_start, i, &mut spans);
                seg_start = i + 1;
            }
            _ => {}
        }
        i += 1;
    }
    if !closed {
        return Vec::new();
    }
    spans
}

fn push_param(src: &str, start: usize, end: usize, spans: &mut Vec<Span>) {
    let bytes = src.as_bytes();
    let mut i = start.min(bytes.len());
    let end = end.min(bytes.len());
    while i < end {
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let s = i;
            while i < end && is_word_char(bytes[i]) {
                i += 1;
            }
            if let Some(span) = Span::new(s as u32, i as u32) {
                spans.push(span);
            }
            return;
        }
        i += 1;
    }
}

fn skip_trivia_text(bytes: &[u8], i: &mut usize) -> bool {
    if *i >= bytes.len() {
        return false;
    }
    if bytes[*i] == b'"' {
        *i += 1;
        while *i < bytes.len() {
            if bytes[*i] == b'\\' {
                *i += 2;
            } else if bytes[*i] == b'"' {
                *i += 1;
                return true;
            } else {
                *i += 1;
            }
        }
        return true;
    }
    if bytes[*i] == b'/' && bytes.get(*i + 1) == Some(&b'/') {
        while *i < bytes.len() && bytes[*i] != b'\n' {
            *i += 1;
        }
        return true;
    }
    if bytes[*i] == b'/' && bytes.get(*i + 1) == Some(&b'*') {
        *i += 2;
        let mut depth = 1usize;
        while *i < bytes.len() && depth > 0 {
            if bytes[*i] == b'/' && bytes.get(*i + 1) == Some(&b'*') {
                depth += 1;
                *i += 2;
            } else if bytes[*i] == b'*' && bytes.get(*i + 1) == Some(&b'/') {
                depth -= 1;
                *i += 2;
            } else {
                *i += 1;
            }
        }
        return true;
    }
    false
}

pub(crate) fn next_ident(src: &str, from: usize) -> Option<(String, Span)> {
    let bytes = src.as_bytes();
    let mut i = from.min(bytes.len());
    while i < bytes.len() {
        if skip_trivia_text(bytes, &mut i) {
            continue;
        }
        if bytes[i].is_ascii_alphabetic() || bytes[i] == b'_' {
            let s = i;
            while i < bytes.len() && is_word_char(bytes[i]) {
                i += 1;
            }
            let span = Span::new(s as u32, i as u32)?;
            return Some((src[s..i].to_string(), span));
        }
        i += 1;
    }
    None
}

fn word_at(src: &str, offset: usize) -> Option<(String, Span)> {    let bytes = src.as_bytes();
    if offset >= bytes.len() || !is_word_char(bytes[offset]) {
        return None;
    }
    let mut s = offset;
    while s > 0 && is_word_char(bytes[s - 1]) {
        s -= 1;
    }
    let mut e = offset;
    while e < bytes.len() && is_word_char(bytes[e]) {
        e += 1;
    }
    Span::new(s as u32, e as u32).map(|span| (src[s..e].to_string(), span))
}

pub fn find_symbol_at(module: &A::Module, src: &str, offset: usize) -> Option<SymbolTarget> {
    let mut collector = Collector {
        src,
        defs: Vec::new(),
        uses: Vec::new(),
        member_uses: Vec::new(),
        this_uses: Vec::new(),
        classes: Vec::new(),
        class_stack: Vec::new(),
        parents: vec![None],
        stack: vec![0],
        cursor: 0,
    };
    collector.collect_module(module);
    let offset = offset as u32;
    let mut best_def: Option<&Def> = None;
    for def in &collector.defs {
        if def.span.start <= offset && offset < def.span.end {
            let replace = match &best_def {
                Some(current) => {
                    let current_len = current.span.end - current.span.start;
                    let def_len = def.span.end - def.span.start;
                    def_len < current_len || (def_len == current_len && def.scope > current.scope)
                }
                None => true,
            };
            if replace {
                best_def = Some(def);
            }
        }
    }
    if let Some(def) = best_def {
        return Some(SymbolTarget {
            span: def.span,
            signature: def.signature.clone(),
            docs: def.docs.clone(),
        });
    }
    let mut best_use: Option<&Use> = None;
    for use_node in &collector.uses {
        if use_node.span.start <= offset && offset < use_node.span.end {
            best_use = Some(use_node);
            break;
        }
    }
    if let Some(use_node) = best_use
        && let Some(def) = collector.visible(&use_node.name, use_node.scope)
    {
        return Some(SymbolTarget {
            span: def.span,
            signature: def.signature.clone(),
            docs: def.docs.clone(),
        });
    }
    for mu in &collector.member_uses {
        if mu.span.start <= offset
            && offset < mu.span.end
            && let Some(def) = collector.find_member(&mu.field, mu.class_ctx)
        {
            return Some(SymbolTarget {
                span: def.span,
                signature: def.signature.clone(),
                docs: def.docs.clone(),
            });
        }
    }
    for tu in &collector.this_uses {
        if tu.span.start <= offset
            && offset < tu.span.end
            && let Some(ci) = tu.class_ctx
            && let Some(info) = collector.classes.get(ci)
        {
            let target = collector.defs.iter().rev().find(|d| {
                d.name == info.name && matches!(d.kind, DefKind::Type) && d.scope == 0
            });
            let (span, signature, docs) = match target {
                Some(def) => (def.span, def.signature.clone(), def.docs.clone()),
                None => (
                    Span { start: tu.span.start, end: tu.span.end },
                    format!("class {}", info.name),
                    info.docs.clone(),
                ),
            };
            return Some(SymbolTarget {
                span,
                signature,
                docs,
            });
        }
    }
    let (word, _) = word_at(src, offset as usize)?;
    collector
        .defs
        .iter()
        .rev()
        .find(|d| d.name == word && d.scope == 0 && !matches!(d.kind, DefKind::Field | DefKind::Method))
        .map(|def| SymbolTarget {
            span: def.span,
            signature: def.signature.clone(),
            docs: def.docs.clone(),
        })
}
