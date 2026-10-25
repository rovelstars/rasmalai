use crate::ast::{
    AssignOp, BinOp, Block, CallArg, ClassMember, Decl, Else, Expr, FnBody,
    IfCond, ImportSource, InterpPart, Module, Spanned, Stmt, SwitchExprBody,
    UnOp,
};
use crate::modules::native_import_locals;
use diagnostics::{Code, Diagnostic, Span};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone)]
pub struct Boundary {
    pub name: String,
    pub span: Span,
    pub direct_seeds: usize,
    pub contained: bool,
}

struct FnEntry<'a> {
    span: Span,
    is_unsafe: bool,
    body: FnBodyKind<'a>,
}

enum FnBodyKind<'a> {
    Body(&'a FnBody),
    Block(&'a Block),
}

struct BlockUse {
    span: Span,
    ffi: usize,
    raw: usize,
    unsafe_calls: usize,
    arith: bool,
    unknown: bool,
    calls: BTreeSet<String>,
}

struct FnUse {
    span: Span,
    in_unsafe_fn: bool,
    ffi: usize,
    raw: usize,
    unsafe_calls: usize,
    arith: bool,
    calls: BTreeSet<String>,
    uncovered_ffi: Vec<(String, Span)>,
    blocks: Vec<BlockUse>,
}

impl FnUse {
    fn blank(span: Span, in_unsafe_fn: bool) -> FnUse {
        FnUse {
            span,
            in_unsafe_fn,
            ffi: 0,
            raw: 0,
            unsafe_calls: 0,
            arith: false,
            calls: BTreeSet::new(),
            uncovered_ffi: Vec::new(),
            blocks: Vec::new(),
        }
    }

    fn direct_seeds(&self) -> usize {
        self.ffi + self.raw
    }
}

struct Facts<'a> {
    fns: BTreeMap<String, FnEntry<'a>>,
    native: BTreeSet<String>,
    unsafe_fns: BTreeSet<String>,
}

fn dotted(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(n) => Some(n.clone()),
        Expr::This => Some("this".to_string()),
        Expr::Super => Some("super".to_string()),
        Expr::Member { base, field } => {
            let b = dotted(&base.node)?;
            Some(format!("{b}.{field}"))
        }
        _ => None,
    }
}

fn is_pointer_base(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(n) => n == "Pointer" || n == "Address",
        Expr::Member { base, .. } => is_pointer_base(&base.node),
        _ => false,
    }
}

fn is_pointer_callee(expr: &Expr) -> bool {
    match expr {
        Expr::Ident(n) => n == "Pointer" || n == "Address",
        Expr::Member { base, .. } => is_pointer_base(&base.node),
        _ => false,
    }
}

impl Facts<'_> {
    fn resolve_loose(&self, name: &str) -> Option<String> {
        if self.fns.contains_key(name) {
            return Some(name.to_string());
        }
        if !name.contains('.') {
            return None;
        }
        let tail = name.rsplit('.').next().unwrap_or(name);
        let mut hit: Option<String> = None;
        for key in self.fns.keys() {
            if key == tail || key.ends_with(&format!(".{tail}")) {
                if hit.is_some() {
                    return None;
                }
                hit = Some(key.clone());
            }
        }
        hit
    }
}

fn uses_mut<'b>(
    uses: &'b mut BTreeMap<String, FnUse>,
    owner: &str,
) -> &'b mut FnUse {
    uses.get_mut(owner).expect("use entry registered")
}

fn record_call(
    facts: &Facts,
    uses: &mut BTreeMap<String, FnUse>,
    owner: &str,
    depth: &[usize],
    name: &str,
    span: Span,
) {
    let target = facts.resolve_loose(name);
    let exact = facts.fns.contains_key(name).then(|| name.to_string());
    let is_native = target
        .as_ref()
        .is_some_and(|t| facts.native.contains(t))
        || facts.native.contains(name);
    let is_unsafe_target = target
        .as_ref()
        .is_some_and(|t| facts.unsafe_fns.contains(t));
    {
        let u = uses_mut(uses, owner);
        if is_native {
            u.ffi += 1;
        }
        if is_unsafe_target {
            u.unsafe_calls += 1;
        }
        match &target {
            Some(t) => {
                u.calls.insert(t.clone());
            }
            None => {
                u.calls.insert(format!("~{name}"));
            }
        }
    }
    if let Some(&i) = depth.last() {
        let u = uses_mut(uses, owner);
        let b = &mut u.blocks[i];
        if is_native {
            b.ffi += 1;
        }
        if is_unsafe_target {
            b.unsafe_calls += 1;
        }
        match &exact {
            Some(t) => {
                b.calls.insert(t.clone());
            }
            None => {
                b.unknown = true;
            }
        }
    }
    if is_native && depth.is_empty() && !uses[owner].in_unsafe_fn {
        uses_mut(uses, owner)
            .uncovered_ffi
            .push((name.to_string(), span));
    }
}

fn record_raw(uses: &mut BTreeMap<String, FnUse>, owner: &str, depth: &[usize]) {
    let u = uses_mut(uses, owner);
    u.raw += 1;
    if let Some(&i) = depth.last() {
        u.blocks[i].raw += 1;
    }
}

fn record_arith(uses: &mut BTreeMap<String, FnUse>, owner: &str, depth: &[usize]) {
    let u = uses_mut(uses, owner);
    u.arith = true;
    if let Some(&i) = depth.last() {
        u.blocks[i].arith = true;
    }
}

fn walk_body(
    facts: &Facts,
    uses: &mut BTreeMap<String, FnUse>,
    owner: &str,
    body: &FnBody,
    depth: &mut Vec<usize>,
) {
    match body {
        FnBody::Block(b) => walk_block(facts, uses, owner, b, depth),
        FnBody::Expr(e) => walk_expr(facts, uses, owner, e, depth),
    }
}

fn walk_block(
    facts: &Facts,
    uses: &mut BTreeMap<String, FnUse>,
    owner: &str,
    block: &Block,
    depth: &mut Vec<usize>,
) {
    for stmt in &block.stmts {
        walk_stmt(facts, uses, owner, stmt, depth);
    }
}

fn walk_stmt(
    facts: &Facts,
    uses: &mut BTreeMap<String, FnUse>,
    owner: &str,
    stmt: &Spanned<Stmt>,
    depth: &mut Vec<usize>,
) {
    match &stmt.node {
        Stmt::Var { value, .. } => walk_expr(facts, uses, owner, value, depth),
        Stmt::DestructureTuple { value, .. }
        | Stmt::DestructureRecord { value, .. }
        | Stmt::DestructureArray { value, .. } => {
            walk_expr(facts, uses, owner, value, depth);
        }
        Stmt::Assign { target, value, op } => {
            walk_expr(facts, uses, owner, target, depth);
            walk_expr(facts, uses, owner, value, depth);
            if *op != AssignOp::Eq {
                record_arith(uses, owner, depth);
            }
        }
        Stmt::Expr(e) => walk_expr(facts, uses, owner, e, depth),
        Stmt::If { cond, then, otherwise } => {
            match cond {
                IfCond::Expr(e) | IfCond::Let { value: e, .. } => {
                    walk_expr(facts, uses, owner, e, depth);
                }
            }
            walk_block(facts, uses, owner, then, depth);
            if let Some(e) = otherwise {
                match e {
                    Else::Block(b) => walk_block(facts, uses, owner, b, depth),
                    Else::If(s) => walk_stmt(facts, uses, owner, s, depth),
                }
            }
        }
        Stmt::For { iter, body, .. } => {
            walk_expr(facts, uses, owner, iter, depth);
            walk_block(facts, uses, owner, body, depth);
        }
        Stmt::While { cond, body } | Stmt::DoWhile { cond, body } => {
            walk_expr(facts, uses, owner, cond, depth);
            walk_block(facts, uses, owner, body, depth);
        }
        Stmt::Switch { scrutinee, cases, default } => {
            walk_expr(facts, uses, owner, scrutinee, depth);
            for c in cases {
                if let Some(g) = &c.guard {
                    walk_expr(facts, uses, owner, g, depth);
                }
                for s in &c.body {
                    walk_stmt(facts, uses, owner, s, depth);
                }
            }
            if let Some(d) = default {
                for s in d {
                    walk_stmt(facts, uses, owner, s, depth);
                }
            }
        }
        Stmt::Return(e) => {
            if let Some(e) = e {
                walk_expr(facts, uses, owner, e, depth);
            }
        }
        Stmt::Assert(e) => walk_expr(facts, uses, owner, e, depth),
        Stmt::Defer(b) => walk_block(facts, uses, owner, b, depth),
        Stmt::Guard { value, otherwise, .. } => {
            walk_expr(facts, uses, owner, value, depth);
            walk_block(facts, uses, owner, otherwise, depth);
        }
        Stmt::Try { body, catch, finally } => {
            walk_block(facts, uses, owner, body, depth);
            if let Some((_, b)) = catch {
                walk_block(facts, uses, owner, b, depth);
            }
            if let Some(b) = finally {
                walk_block(facts, uses, owner, b, depth);
            }
        }
        Stmt::Throw(e) => {
            if let Some(e) = e {
                walk_expr(facts, uses, owner, e, depth);
            }
        }
        Stmt::UnsafeBlock(b) => {
            push_block(uses, owner, stmt.span);
            let idx = uses[owner].blocks.len() - 1;
            depth.push(idx);
            walk_block(facts, uses, owner, b, depth);
            depth.pop();
        }
        Stmt::Break | Stmt::Continue | Stmt::Fallthrough | Stmt::Pass | Stmt::Empty => {}
    }
}

fn push_block(uses: &mut BTreeMap<String, FnUse>, owner: &str, span: Span) {
    uses_mut(uses, owner).blocks.push(BlockUse {
        span,
        ffi: 0,
        raw: 0,
        unsafe_calls: 0,
        arith: false,
        unknown: false,
        calls: BTreeSet::new(),
    });
}

fn walk_expr(
    facts: &Facts,
    uses: &mut BTreeMap<String, FnUse>,
    owner: &str,
    expr: &Spanned<Expr>,
    depth: &mut Vec<usize>,
) {
    match &expr.node {
        Expr::Ident(n) if n == "Pointer" || n == "Address" => {
            record_raw(uses, owner, depth);
        }
        Expr::Member { base, .. } if is_pointer_base(&base.node) => {
            record_raw(uses, owner, depth);
            walk_expr(facts, uses, owner, base, depth);
        }
        Expr::Call { callee, args, trailing, .. } => {
            if is_pointer_callee(&callee.node) {
                record_raw(uses, owner, depth);
            }
            if let Some(name) = dotted(&callee.node) {
                record_call(facts, uses, owner, depth, &name, expr.span);
            }
            walk_expr(facts, uses, owner, callee, depth);
            for a in args {
                walk_call_arg(facts, uses, owner, a, depth);
            }
            if let Some(b) = trailing {
                walk_block(facts, uses, owner, b, depth);
            }
        }
        Expr::New { args, .. } => {
            for a in args {
                walk_call_arg(facts, uses, owner, a, depth);
            }
        }
        Expr::OptCall { base, args, field } => {
            let _ = field;
            walk_expr(facts, uses, owner, base, depth);
            for a in args {
                walk_call_arg(facts, uses, owner, a, depth);
            }
        }
        Expr::Unary { op, rhs } => {
            if matches!(op, UnOp::AddrOf | UnOp::Deref) {
                record_raw(uses, owner, depth);
            }
            walk_expr(facts, uses, owner, rhs, depth);
        }
        Expr::Postfix { expr: inner, .. } => {
            record_arith(uses, owner, depth);
            walk_expr(facts, uses, owner, inner, depth);
        }
        Expr::Binary { op, lhs, rhs } => {
            if matches!(op, BinOp::Add | BinOp::Sub) {
                record_arith(uses, owner, depth);
            }
            walk_expr(facts, uses, owner, lhs, depth);
            walk_expr(facts, uses, owner, rhs, depth);
        }
        Expr::Interp(parts) => {
            for p in parts {
                if let InterpPart::Expr(e) = p {
                    walk_expr(facts, uses, owner, e, depth);
                }
            }
        }
        Expr::Array(elems) => {
            for e in elems {
                walk_expr(facts, uses, owner, &e.expr, depth);
            }
        }
        Expr::Record(fields) => {
            for e in fields {
                walk_expr(facts, uses, owner, e.value(), depth);
            }
        }
        Expr::MapLiteral(entries) => {
            for e in entries {
                walk_expr(facts, uses, owner, e.value(), depth);
            }
        }
        Expr::Ternary { cond, then, otherwise } => {
            walk_expr(facts, uses, owner, cond, depth);
            walk_expr(facts, uses, owner, then, depth);
            walk_expr(facts, uses, owner, otherwise, depth);
        }
        Expr::Coalesce { lhs, rhs } => {
            walk_expr(facts, uses, owner, lhs, depth);
            walk_expr(facts, uses, owner, rhs, depth);
        }
        Expr::OptChain { base, .. } => walk_expr(facts, uses, owner, base, depth),
        Expr::Range { lo, hi, .. } => {
            walk_expr(facts, uses, owner, lo, depth);
            walk_expr(facts, uses, owner, hi, depth);
        }
        Expr::Index { base, index } => {
            walk_expr(facts, uses, owner, base, depth);
            walk_expr(facts, uses, owner, index, depth);
        }
        Expr::Cast { expr: inner, .. } | Expr::Is { base: inner, .. } => {
            walk_expr(facts, uses, owner, inner, depth);
        }
        Expr::Macro { args, .. } => {
            for a in args {
                walk_expr(facts, uses, owner, a, depth);
            }
        }
        Expr::Closure { body, .. } => match body {
            FnBody::Block(b) => walk_block(facts, uses, owner, b, depth),
            FnBody::Expr(e) => walk_expr(facts, uses, owner, e, depth),
        },
        Expr::UnsafeBlock(b) => {
            push_block(uses, owner, expr.span);
            let idx = uses[owner].blocks.len() - 1;
            depth.push(idx);
            walk_block(facts, uses, owner, b, depth);
            depth.pop();
        }
        Expr::Await(inner) | Expr::Propagate(inner) => {
            walk_expr(facts, uses, owner, inner, depth);
        }
        Expr::Tuple(elems) => {
            for e in elems {
                walk_expr(facts, uses, owner, e, depth);
            }
        }
        Expr::TupleGet { base, .. } => walk_expr(facts, uses, owner, base, depth),
        Expr::Switch { scrutinee, cases, default } => {
            walk_expr(facts, uses, owner, scrutinee, depth);
            for c in cases {
                if let Some(g) = &c.guard {
                    walk_expr(facts, uses, owner, g, depth);
                }
                match &c.body {
                    SwitchExprBody::Expr(e) => walk_expr(facts, uses, owner, e, depth),
                    SwitchExprBody::Block(b) => walk_block(facts, uses, owner, b, depth),
                }
            }
            if let Some(d) = default {
                match d {
                    SwitchExprBody::Expr(e) => walk_expr(facts, uses, owner, e, depth),
                    SwitchExprBody::Block(b) => walk_block(facts, uses, owner, b, depth),
                }
            }
        }
        Expr::Member { base, .. } => walk_expr(facts, uses, owner, base, depth),
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

fn walk_call_arg(
    facts: &Facts,
    uses: &mut BTreeMap<String, FnUse>,
    owner: &str,
    arg: &CallArg,
    depth: &mut Vec<usize>,
) {
    walk_expr(facts, uses, owner, &arg.value, depth);
}

fn collect(module: &Module) -> Facts<'_> {
    let mut facts = Facts {
        fns: BTreeMap::new(),
        native: BTreeSet::new(),
        unsafe_fns: BTreeSet::new(),
    };
    for decl in &module.decls {
        match &decl.node {
            Decl::Fn(f) => {
                if f.is_unsafe {
                    facts.unsafe_fns.insert(f.name.clone());
                }
                facts.fns.insert(
                    f.name.clone(),
                    FnEntry { span: decl.span, is_unsafe: f.is_unsafe, body: FnBodyKind::Body(&f.body) },
                );
            }
            Decl::Class { name, members, .. } => {
                for m in members {
                    match &m.node {
                        ClassMember::Method(f) => {
                            let full = format!("{name}.{}", f.name);
                            if f.is_unsafe {
                                facts.unsafe_fns.insert(full.clone());
                            }
                            facts.fns.insert(
                                full,
                                FnEntry { span: m.span, is_unsafe: f.is_unsafe, body: FnBodyKind::Body(&f.body) },
                            );
                        }
                        ClassMember::Init { body, .. } => {
                            facts.fns.insert(
                                format!("{name}.init"),
                                FnEntry { span: m.span, is_unsafe: false, body: FnBodyKind::Block(body) },
                            );
                        }
                        ClassMember::Deinit(b) => {
                            facts.fns.insert(
                                format!("{name}.deinit"),
                                FnEntry { span: m.span, is_unsafe: false, body: FnBodyKind::Block(b) },
                            );
                        }
                        ClassMember::OnReload { body, .. } => {
                            facts.fns.insert(
                                format!("{name}.onReload"),
                                FnEntry { span: m.span, is_unsafe: false, body: FnBodyKind::Block(body) },
                            );
                        }
                        _ => {}
                    }
                }
            }
            Decl::Extension { target, members, .. } => {
                let owner = target.path.last().cloned().unwrap_or_default();
                for m in members {
                    if let ClassMember::Method(f) = &m.node {
                        let full = format!("{owner}.{}", f.name);
                        if f.is_unsafe {
                            facts.unsafe_fns.insert(full.clone());
                        }
                        facts.fns.insert(
                            full,
                            FnEntry { span: m.span, is_unsafe: f.is_unsafe, body: FnBodyKind::Body(&f.body) },
                        );
                    }
                }
            }
            Decl::Import(d) => {
                if matches!(d.source, ImportSource::Native(_)) {
                    for local in native_import_locals(&d.clause) {
                        facts.native.insert(local);
                    }
                }
            }
            Decl::ExportFrom(d) => {
                if matches!(d.source, ImportSource::Native(_)) {
                    if let crate::ast::ExportClause::Native(sigs) = &d.clause {
                        for sig in sigs {
                            let local = sig.alias.clone().unwrap_or_else(|| sig.name.clone());
                            facts.native.insert(local);
                        }
                    }
                }
            }
            _ => {}
        }
    }
    facts
}

fn walk_all(facts: &Facts, uses: &mut BTreeMap<String, FnUse>, module: &Module) {
    for (name, entry) in &facts.fns {
        uses.insert(name.clone(), FnUse::blank(entry.span, entry.is_unsafe));
    }
    for (name, entry) in &facts.fns {
        let mut depth = Vec::new();
        match entry.body {
            FnBodyKind::Body(b) => walk_body(facts, uses, name, b, &mut depth),
            FnBodyKind::Block(b) => walk_block(facts, uses, name, b, &mut depth),
        }
    }
    for decl in &module.decls {
        match &decl.node {
            Decl::Const { value, .. } => {
                uses.insert("<const>".to_string(), FnUse::blank(decl.span, false));
                let mut depth = Vec::new();
                walk_expr(facts, uses, "<const>", value, &mut depth);
            }
            Decl::Stmt(s) => {
                uses.insert("<top-level>".to_string(), FnUse::blank(decl.span, false));
                let mut depth = Vec::new();
                walk_stmt(facts, uses, "<top-level>", s, &mut depth);
            }
            _ => {}
        }
    }
}

fn tainted_set(facts: &Facts, uses: &BTreeMap<String, FnUse>) -> BTreeMap<String, bool> {
    let mut tainted: BTreeMap<String, bool> = BTreeMap::new();
    for name in facts.fns.keys() {
        tainted.insert(name.clone(), uses[name].direct_seeds() > 0);
    }
    loop {
        let mut changed = false;
        for name in facts.fns.keys() {
            if tainted[name] {
                continue;
            }
            if uses[name]
                .calls
                .iter()
                .any(|c| tainted.get(c).copied().unwrap_or(false))
            {
                tainted.insert(name.clone(), true);
                changed = true;
            }
        }
        if !changed {
            break;
        }
    }
    tainted
}

fn lint_blocks(
    facts: &Facts,
    uses: &BTreeMap<String, FnUse>,
    owner: &str,
    tainted: &BTreeMap<String, bool>,
    diags: &mut Vec<Diagnostic>,
) {
    let Some(u) = uses.get(owner) else {
        return;
    };
    for b in &u.blocks {
        if block_lintable(facts, b, tainted) {
            diags.push(
                Diagnostic::new(Code::W202, "unneeded `unsafe` block".to_string())
                    .with_span(b.span)
                    .with_hint("remove the `unsafe` block or move a foreign call into it"),
            );
        }
    }
}

fn block_lintable(
    facts: &Facts,
    block: &BlockUse,
    tainted: &BTreeMap<String, bool>,
) -> bool {
    if block.ffi > 0 || block.raw > 0 || block.unsafe_calls > 0 || block.arith || block.unknown {
        return false;
    }
    for c in &block.calls {
        if !facts.fns.contains_key(c) {
            return false;
        }
        if tainted.get(c).copied().unwrap_or(true) {
            return false;
        }
    }
    true
}

pub fn check_module(module: &Module) -> Vec<Diagnostic> {
    let facts = collect(module);
    let mut uses: BTreeMap<String, FnUse> = BTreeMap::new();
    walk_all(&facts, &mut uses, module);
    let tainted = tainted_set(&facts, &uses);
    let mut diags = Vec::new();
    let mut owners: Vec<String> = facts.fns.keys().cloned().collect();
    if uses.contains_key("<const>") {
        owners.push("<const>".to_string());
    }
    if uses.contains_key("<top-level>") {
        owners.push("<top-level>".to_string());
    }
    for owner in &owners {
        let Some(u) = uses.get(owner) else {
            continue;
        };
        for (callee, span) in &u.uncovered_ffi {
            diags.push(
                Diagnostic::new(
                    Code::E201,
                    format!("foreign call to `{callee}` needs `unsafe`"),
                )
                .with_span(*span)
                .with_hint("wrap the call in `unsafe { ... }`"),
            );
        }
        lint_blocks(&facts, &uses, owner, &tainted, &mut diags);
    }
    diags.sort_by(|a, b| {
        a.code
            .as_str()
            .cmp(b.code.as_str())
            .then(a.message.cmp(&b.message))
    });
    diags
}

pub fn boundary_map(module: &Module) -> Vec<Boundary> {
    let facts = collect(module);
    let mut uses: BTreeMap<String, FnUse> = BTreeMap::new();
    walk_all(&facts, &mut uses, module);
    let tainted = tainted_set(&facts, &uses);
    let mut out = Vec::new();
    for name in facts.fns.keys() {
        if !tainted[name] {
            continue;
        }
        let u = &uses[name];
        out.push(Boundary {
            name: name.clone(),
            span: u.span,
            direct_seeds: u.direct_seeds(),
            contained: u.uncovered_ffi.is_empty(),
        });
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::Parser;

    fn parse(src: &str) -> Module {
        Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
    }

    fn codes(module: &Module) -> Vec<String> {
        check_module(module)
            .iter()
            .map(|d| format!("{}:{}", d.code.as_str(), d.message))
            .collect()
    }

    #[test]
    fn ffi_outside_unsafe_is_e201() {
        let m = parse(
            "import { fn compressBound(sourceLen: Int): Int } from native \"z\"\nfn Main(): Int {\n    return compressBound(100);\n}\n",
        );
        let got = codes(&m);
        assert!(got.iter().any(|d| d.starts_with("E201")), "{got:?}");
    }

    #[test]
    fn contained_wrapper_passes() {
        let m = parse(
            "import { fn compressBound(sourceLen: Int): Int } from native \"z\"\nfn bound(n: Int): Int {\n    unsafe {\n        return compressBound(n);\n    }\n}\nfn Main(): Int {\n    return bound(100);\n}\n",
        );
        let got = codes(&m);
        assert!(got.is_empty(), "{got:?}");
        let map = boundary_map(&m);
        assert_eq!(map.len(), 2, "{map:?}");
        assert!(map.iter().any(|b| b.name == "bound" && b.contained));
        assert!(map.iter().any(|b| b.name == "Main" && b.direct_seeds == 0));
    }

    #[test]
    fn empty_unsafe_block_lints() {
        let m = parse("fn Main(): Int {\n    unsafe {\n        return 1;\n    }\n}\n");
        let got = codes(&m);
        assert!(got.iter().any(|d| d.starts_with("W202")), "{got:?}");
    }
}
