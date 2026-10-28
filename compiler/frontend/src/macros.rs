use crate::ast as A;
use crate::parser::Parser;
use diagnostics::{Code, Diagnostic, Span};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};

pub const FUEL_STEPS: u64 = 1_000_000;
pub const CALL_DEPTH_MAX: u32 = 64;
pub const NODE_CAP: usize = 10_000;
pub const ITER_CAP: usize = 8;

pub fn toolchain() -> String {
    format!("rasmalai-macros/{}", env!("CARGO_PKG_VERSION"))
}

static COMPTIME_RUNS: AtomicU64 = AtomicU64::new(0);

pub fn comptime_runs() -> u64 {
    COMPTIME_RUNS.load(Ordering::SeqCst)
}

pub fn note_comptime_run() {
    COMPTIME_RUNS.fetch_add(1, Ordering::SeqCst);
}

pub fn reset_comptime_runs() {
    COMPTIME_RUNS.store(0, Ordering::SeqCst);
}

pub fn content_hash(bytes: &[u8]) -> String {
    let mut h = Sha256::new();
    h.update(bytes);
    hex_bytes(&h.finalize())
}

fn hex_bytes(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(hex_digit(*b >> 4));
        out.push(hex_digit(*b & 15));
    }
    out
}

fn hex_digit(n: u8) -> char {
    (b'0' + n + if n > 9 { 39 } else { 0 }) as char
}

fn hash_str(parts: &[&str]) -> String {
    let mut h = Sha256::new();
    for p in parts {
        h.update(p.as_bytes());
        h.update([0u8]);
    }
    hex_bytes(&h.finalize())
}

pub fn str_lit(s: &str, span: Span) -> A::Spanned<A::Expr> {
    A::sp(
        A::Expr::Interp(vec![A::InterpPart::Text(s.to_string())]),
        span,
    )
}

pub fn as_str_lit(expr: &A::Expr) -> Option<String> {
    if let A::Expr::Interp(parts) = expr {
        if parts.len() == 1 {
            if let A::InterpPart::Text(s) = &parts[0] {
                return Some(s.clone());
            }
        }
    }
    None
}

#[derive(Clone, Debug)]
pub struct CachedExpansion {
    pub expr: A::Expr,
    pub span: Span,
}

#[derive(Default)]
pub struct ExpansionCache {
    entries: BTreeMap<String, CachedExpansion>,
    methods: BTreeMap<String, Vec<String>>,
}

impl ExpansionCache {
    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty() && self.methods.is_empty()
    }
}

pub struct MacroOptions {
    pub base_dir: PathBuf,
    pub dump_dir: Option<PathBuf>,
    pub cache: ExpansionCache,
}

impl MacroOptions {
    pub fn new(base_dir: PathBuf) -> MacroOptions {
        MacroOptions {
            base_dir,
            dump_dir: None,
            cache: ExpansionCache::default(),
        }
    }

    pub fn cache_key(&self, macro_name: &str, args_debug: &str, file_hashes: &[String]) -> String {
        let tc = toolchain();
        let mut parts: Vec<&str> = vec![macro_name, args_debug, tc.as_str()];
        for f in file_hashes {
            parts.push(f.as_str());
        }
        hash_str(&parts)
    }

    pub fn cache_get(&self, key: &str) -> Option<&CachedExpansion> {
        self.cache.entries.get(key)
    }

    pub fn cache_insert(&mut self, key: String, value: CachedExpansion) {
        self.cache.entries.insert(key, value);
    }

    pub fn methods_get(&self, key: &str) -> Option<&Vec<String>> {
        self.cache.methods.get(key)
    }

    pub fn methods_insert(&mut self, key: String, value: Vec<String>) {
        self.cache.methods.insert(key, value);
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ComptimeValue {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
    Array(Vec<ComptimeValue>),
}

impl ComptimeValue {
    pub fn to_expr(&self, span: Span) -> A::Expr {
        match self {
            ComptimeValue::Int(v) => A::Expr::Int(*v),
            ComptimeValue::Float(v) => A::Expr::Float(*v),
            ComptimeValue::Bool(v) => A::Expr::Bool(*v),
            ComptimeValue::Str(s) => str_lit(s, span).node,
            ComptimeValue::Array(items) => A::Expr::Array(
                items
                    .iter()
                    .map(|i| A::ArrayElem {
                        spread: false,
                        expr: A::sp(i.to_expr(span), span),
                    })
                    .collect(),
            ),
        }
    }
}

pub trait ComptimeRunner {
    fn call(
        &self,
        fn_name: &str,
        fn_span: Span,
        args: Vec<ComptimeValue>,
        use_span: Span,
    ) -> Result<ComptimeValue, Diagnostic>;
}

const KNOWN_ATTRS: &[&str] = &[
    "Derive",
    "CompTime",
    "Inline",
    "Layout",
    "Section",
    "Allow",
    "Snapshot",
    "Persist",
    "HotReload",
    "FastMath",
    "AllowUnsafe",
    "PgoScenario",
];

pub fn is_phase_a_macro(name: &str) -> bool {
    matches!(name, "vec" | "EmbedBytes")
}

fn macro_key_args(args: &[A::Spanned<A::Expr>]) -> String {
    format!("{args:?}")
}

struct PassCtx<'a> {
    opts: &'a mut MacroOptions,
    diags: Vec<Diagnostic>,
    changed: bool,
    node_count: usize,
    capped: bool,
}

impl PassCtx<'_> {
    fn bump(&mut self, span: Span) {
        self.node_count += 1;
        if self.node_count > NODE_CAP && !self.capped {
            self.capped = true;
            self.diags.push(
                Diagnostic::new(
                    Code::E119,
                    format!("macro output exceeds {NODE_CAP} nodes"),
                )
                .with_span(span)
                .with_hint("split the expansion across smaller macros"),
            );
        }
    }
}

pub fn expand_phase_a(
    module: &mut A::Module,
    opts: &mut MacroOptions,
) -> Vec<Diagnostic> {
    if let Some(dir) = opts.dump_dir.clone() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("phase-a.before"), format!("{module:#?}"));
    }
    let mut diags = Vec::new();
    for iter in 0..ITER_CAP {
        let mut ctx = PassCtx {
            opts,
            diags: Vec::new(),
            changed: false,
            node_count: 0,
            capped: false,
        };
        for decl in &mut module.decls {
            rewrite_decl_a(&mut decl.node, &mut ctx);
            if ctx.capped {
                break;
            }
        }
        let PassCtx {
            opts: _,
            diags: mut d,
            changed,
            ..
        } = ctx;
        diags.append(&mut d);
        if diags.iter().any(|x| x.code == Code::E119) {
            break;
        }
        if !changed {
            break;
        }
        if iter + 1 >= ITER_CAP {
            diags.push(
                Diagnostic::new(
                    Code::E120,
                    format!("macro expansion did not reach a fixed point after {ITER_CAP} iterations"),
                )
                .with_hint("a macro is emitting macros that expand forever"),
            );
        }
    }
    if let Some(dir) = opts.dump_dir.clone() {
        let _ = std::fs::write(dir.join("phase-a.after"), format!("{module:#?}"));
    }
    diags
}

fn rewrite_decl_a(node: &mut A::Decl, ctx: &mut PassCtx) {
    match node {
        A::Decl::Class { members, .. }
        | A::Decl::Struct { members, .. }
        | A::Decl::Trait { members, .. }
        | A::Decl::Interface { members, .. }
        | A::Decl::Extension { members, .. } => {
            for m in members {
                rewrite_member_a(&mut m.node, ctx);
            }
        }
        A::Decl::Fn(f) => rewrite_fn_a(f, ctx),
        A::Decl::Const { value, .. } => rewrite_expr_a(&mut value.node, value.span, ctx),
        A::Decl::Stmt(s) => rewrite_stmt_a(&mut s.node, s.span, ctx),
        _ => {}
    }
}

fn rewrite_member_a(node: &mut A::ClassMember, ctx: &mut PassCtx) {
    match node {
        A::ClassMember::Field(f) => {
            if let Some(v) = f.value.as_mut() {
                rewrite_expr_a(&mut v.node, v.span, ctx);
            }
        }
        A::ClassMember::Method(f) => rewrite_fn_a(f, ctx),
        A::ClassMember::Init { body, .. }
        | A::ClassMember::Deinit(body)
        | A::ClassMember::OnReload { body, .. } => rewrite_block_a(body, ctx),
    }
}

fn rewrite_fn_a(f: &mut A::FnDecl, ctx: &mut PassCtx) {
    match &mut f.body {
        A::FnBody::Block(b) => rewrite_block_a(b, ctx),
        A::FnBody::Expr(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
    }
}

fn rewrite_block_a(block: &mut A::Block, ctx: &mut PassCtx) {
    for s in &mut block.stmts {
        rewrite_stmt_a(&mut s.node, s.span, ctx);
        if ctx.capped {
            return;
        }
    }
}

fn rewrite_stmt_a(stmt: &mut A::Stmt, span: Span, ctx: &mut PassCtx) {
    ctx.bump(span);
    if ctx.capped {
        return;
    }
    match stmt {
        A::Stmt::Var { value, .. } => rewrite_expr_a(&mut value.node, value.span, ctx),
        A::Stmt::DestructureTuple { value, .. }
        | A::Stmt::DestructureRecord { value, .. }
        | A::Stmt::DestructureArray { value, .. } => {
            rewrite_expr_a(&mut value.node, value.span, ctx)
        }
        A::Stmt::Assign { target, value, .. } => {
            rewrite_expr_a(&mut target.node, target.span, ctx);
            rewrite_expr_a(&mut value.node, value.span, ctx);
        }
        A::Stmt::Expr(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
        A::Stmt::If { cond, then, otherwise } => {
            match cond {
                A::IfCond::Expr(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
                A::IfCond::Let { value, .. } => rewrite_expr_a(&mut value.node, value.span, ctx),
            }
            rewrite_block_a(then, ctx);
            if let Some(A::Else::Block(b)) = otherwise {
                rewrite_block_a(b, ctx);
            }
        }
        A::Stmt::For { iter, body, .. } => {
            rewrite_expr_a(&mut iter.node, iter.span, ctx);
            rewrite_block_a(body, ctx);
        }
        A::Stmt::While { cond, body } | A::Stmt::DoWhile { cond, body } => {
            rewrite_expr_a(&mut cond.node, cond.span, ctx);
            rewrite_block_a(body, ctx);
        }
        A::Stmt::Switch {
            scrutinee,
            cases,
            default,
        } => {
            rewrite_expr_a(&mut scrutinee.node, scrutinee.span, ctx);
            for c in cases {
                if let Some(g) = c.guard.as_mut() {
                    rewrite_expr_a(&mut g.node, g.span, ctx);
                }
                for s in &mut c.body {
                    rewrite_stmt_a(&mut s.node, s.span, ctx);
                }
            }
            if let Some(stmts) = default {
                for s in stmts {
                    rewrite_stmt_a(&mut s.node, s.span, ctx);
                }
            }
        }
        A::Stmt::Return(e) => {
            if let Some(e) = e {
                rewrite_expr_a(&mut e.node, e.span, ctx);
            }
        }
        A::Stmt::Assert(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
        A::Stmt::Defer(b) | A::Stmt::UnsafeBlock(b) => rewrite_block_a(b, ctx),
        A::Stmt::Guard { value, otherwise, .. } => {
            rewrite_expr_a(&mut value.node, value.span, ctx);
            rewrite_block_a(otherwise, ctx);
        }
        A::Stmt::Try { body, catch, finally } => {
            rewrite_block_a(body, ctx);
            if let Some((_, b)) = catch {
                rewrite_block_a(b, ctx);
            }
            if let Some(b) = finally {
                rewrite_block_a(b, ctx);
            }
        }
        A::Stmt::Throw(e) => {
            if let Some(e) = e {
                rewrite_expr_a(&mut e.node, e.span, ctx);
            }
        }
        A::Stmt::Break | A::Stmt::Continue | A::Stmt::Fallthrough | A::Stmt::Pass | A::Stmt::Empty => {}
    }
}

fn rewrite_expr_a(expr: &mut A::Expr, span: Span, ctx: &mut PassCtx) {
    ctx.bump(span);
    if ctx.capped {
        return;
    }
    recurse_expr_a(expr, ctx);
    if ctx.capped {
        return;
    }
    let (name, args) = match expr {
        A::Expr::Macro { name, args, .. } if is_phase_a_macro(name) => {
            (name.clone(), args.clone())
        }
        _ => return,
    };
    match expand_builtin_a(&name, &args, span, ctx.opts) {
        Ok(Some(new_expr)) => {
            *expr = new_expr;
            ctx.changed = true;
        }
        Ok(None) => {}
        Err(d) => ctx.diags.push(d),
    }
}

fn recurse_expr_a(expr: &mut A::Expr, ctx: &mut PassCtx) {
    match expr {
        A::Expr::Array(items) => {
            for i in items {
                rewrite_expr_a(&mut i.expr.node, i.expr.span, ctx);
            }
        }
        A::Expr::Call { callee, args, trailing, .. } => {
            rewrite_expr_a(&mut callee.node, callee.span, ctx);
            for a in args {
                rewrite_expr_a(&mut a.value.node, a.value.span, ctx);
            }
            if let Some(b) = trailing {
                rewrite_block_a(b, ctx);
            }
        }
        A::Expr::Binary { lhs, rhs, .. } => {
            rewrite_expr_a(&mut lhs.node, lhs.span, ctx);
            rewrite_expr_a(&mut rhs.node, rhs.span, ctx);
        }
        A::Expr::Unary { rhs, .. } => rewrite_expr_a(&mut rhs.node, rhs.span, ctx),
        A::Expr::Postfix { expr: e, .. } => rewrite_expr_a(&mut e.node, e.span, ctx),
        A::Expr::Ternary { cond, then, otherwise } => {
            rewrite_expr_a(&mut cond.node, cond.span, ctx);
            rewrite_expr_a(&mut then.node, then.span, ctx);
            rewrite_expr_a(&mut otherwise.node, otherwise.span, ctx);
        }
        A::Expr::Range { lo, hi, .. } => {
            rewrite_expr_a(&mut lo.node, lo.span, ctx);
            rewrite_expr_a(&mut hi.node, hi.span, ctx);
        }
        A::Expr::Index { base, index } => {
            rewrite_expr_a(&mut base.node, base.span, ctx);
            rewrite_expr_a(&mut index.node, index.span, ctx);
        }
        A::Expr::Member { base, .. } => rewrite_expr_a(&mut base.node, base.span, ctx),
        A::Expr::Cast { expr: e, .. } | A::Expr::Is { base: e, .. } => {
            rewrite_expr_a(&mut e.node, e.span, ctx)
        }
        A::Expr::New { args, .. } => {
            for a in args {
                rewrite_expr_a(&mut a.value.node, a.value.span, ctx);
            }
        }
        A::Expr::Macro { args, .. } => {
            for a in args {
                rewrite_expr_a(&mut a.node, a.span, ctx);
            }
        }
        A::Expr::Closure { body, .. } => match body {
            A::FnBody::Block(b) => rewrite_block_a(b, ctx),
            A::FnBody::Expr(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
        },
        A::Expr::Interp(parts) => {
            for p in parts {
                if let A::InterpPart::Expr(e) = p {
                    rewrite_expr_a(&mut e.node, e.span, ctx);
                }
            }
        }
        A::Expr::Record(fields) => {
            for f in fields {
                let v = f.value_mut();
                rewrite_expr_a(&mut v.node, v.span, ctx);
            }
        }
        A::Expr::MapLiteral(entries) => {
            for e in entries {
                let v = e.value_mut();
                rewrite_expr_a(&mut v.node, v.span, ctx);
            }
        }
        A::Expr::Tuple(items) => {
            for i in items {
                rewrite_expr_a(&mut i.node, i.span, ctx);
            }
        }
        A::Expr::TupleGet { base, .. } => rewrite_expr_a(&mut base.node, base.span, ctx),
        A::Expr::Switch { scrutinee, cases, default } => {
            rewrite_expr_a(&mut scrutinee.node, scrutinee.span, ctx);
            for c in cases {
                if let Some(g) = c.guard.as_mut() {
                    rewrite_expr_a(&mut g.node, g.span, ctx);
                }
                match &mut c.body {
                    A::SwitchExprBody::Expr(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
                    A::SwitchExprBody::Block(b) => rewrite_block_a(b, ctx),
                }
            }
            if let Some(d) = default {
                match d {
                    A::SwitchExprBody::Expr(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
                    A::SwitchExprBody::Block(b) => rewrite_block_a(b, ctx),
                }
            }
        }
        A::Expr::Coalesce { lhs, rhs } => {
            rewrite_expr_a(&mut lhs.node, lhs.span, ctx);
            rewrite_expr_a(&mut rhs.node, rhs.span, ctx);
        }
        A::Expr::OptChain { base, .. } => {
            rewrite_expr_a(&mut base.node, base.span, ctx);
        }
        A::Expr::OptCall { base, args, .. } => {
            rewrite_expr_a(&mut base.node, base.span, ctx);
            for a in args {
                rewrite_expr_a(&mut a.value.node, a.value.span, ctx);
            }
        }
        A::Expr::Await(e) | A::Expr::Propagate(e) => rewrite_expr_a(&mut e.node, e.span, ctx),
        A::Expr::UnsafeBlock(b) => rewrite_block_a(b, ctx),
        _ => {}
    }
}

fn array_of(items: Vec<A::Spanned<A::Expr>>) -> A::Expr {
    A::Expr::Array(
        items
            .into_iter()
            .map(|e| A::ArrayElem { spread: false, expr: e })
            .collect(),
    )
}

fn expand_builtin_a(
    name: &str,
    args: &[A::Spanned<A::Expr>],
    use_span: Span,
    opts: &mut MacroOptions,
) -> Result<Option<A::Expr>, Diagnostic> {
    match name {
        "vec" => {
            let key = opts.cache_key(name, &macro_key_args(args), &[]);
            if let Some(hit) = opts.cache_get(&key).cloned() {
                let _ = hit.span;
                return Ok(Some(hit.expr));
            }
            let expanded = array_of(args.to_vec());
            opts.cache_insert(
                key,
                CachedExpansion { expr: expanded.clone(), span: use_span },
            );
            Ok(Some(expanded))
        }
        "EmbedBytes" => {
            if args.len() != 1 {
                return Err(Diagnostic::new(
                    Code::E122,
                    "`EmbedBytes!` takes exactly one path literal",
                )
                .with_span(use_span)
                .with_hint("example: EmbedBytes!(\"assets/icon.bin\")"));
            }
            let path_text = match as_str_lit(&args[0].node) {
                Some(s) => s,
                None => {
                    return Err(Diagnostic::new(
                        Code::E122,
                        "`EmbedBytes!` path must be a string literal",
                    )
                    .with_span(args[0].span)
                    .with_hint("file bytes enter only through a literal path"));
                }
            };
            let rel = std::path::Path::new(&path_text);
            if rel.is_absolute() {
                return Err(Diagnostic::new(
                    Code::E122,
                    "`EmbedBytes!` rejects absolute paths",
                )
                .with_span(args[0].span)
                .with_hint("use a crate-relative path"));
            }
            if path_text.contains("..") {
                return Err(Diagnostic::new(
                    Code::E122,
                    "`EmbedBytes!` rejects parent escapes",
                )
                .with_span(args[0].span)
                .with_hint("keep embedded files inside the crate directory"));
            }
            let joined = opts.base_dir.join(rel);
            let bytes = std::fs::read(&joined).map_err(|_| {
                Diagnostic::new(
                    Code::E122,
                    format!("`EmbedBytes!` cannot read `{path_text}`"),
                )
                .with_span(args[0].span)
                .with_hint("the file must exist at build time")
            })?;
            let file_hash = content_hash(&bytes);
            let key = opts.cache_key(name, &macro_key_args(args), &[file_hash]);
            if let Some(hit) = opts.cache_get(&key).cloned() {
                return Ok(Some(hit.expr));
            }
            let items: Vec<A::Spanned<A::Expr>> = bytes
                .iter()
                .map(|b| A::sp(A::Expr::Int(*b as i64), use_span))
                .collect();
            let expanded = array_of(items);
            opts.cache_insert(
                key,
                CachedExpansion { expr: expanded.clone(), span: use_span },
            );
            Ok(Some(expanded))
        }
        _ => Ok(None),
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum ConstVal {
    Int(i64),
    Float(f64),
    Bool(bool),
    Str(String),
}

impl ConstVal {
    pub fn to_expr(&self, span: Span) -> A::Expr {
        match self {
            ConstVal::Int(v) => A::Expr::Int(*v),
            ConstVal::Float(v) => A::Expr::Float(*v),
            ConstVal::Bool(v) => A::Expr::Bool(*v),
            ConstVal::Str(s) => str_lit(s, span).node,
        }
    }

    pub fn to_comptime(&self) -> ComptimeValue {
        match self {
            ConstVal::Int(v) => ComptimeValue::Int(*v),
            ConstVal::Float(v) => ComptimeValue::Float(*v),
            ConstVal::Bool(v) => ComptimeValue::Bool(*v),
            ConstVal::Str(s) => ComptimeValue::Str(s.clone()),
        }
    }

    pub fn truthy(&self) -> Option<bool> {
        match self {
            ConstVal::Bool(b) => Some(*b),
            ConstVal::Int(i) => Some(*i != 0),
            ConstVal::Float(f) => Some(*f != 0.0),
            ConstVal::Str(s) => Some(!s.is_empty()),
        }
    }
}

pub fn eval_const(expr: &A::Expr) -> Result<ConstVal, ()> {
    match expr {
        A::Expr::Bool(b) => Ok(ConstVal::Bool(*b)),
        A::Expr::Int(i) => Ok(ConstVal::Int(*i)),
        A::Expr::Float(f) => Ok(ConstVal::Float(*f)),
        e if as_str_lit(e).is_some() => Ok(ConstVal::Str(as_str_lit(e).unwrap_or_default())),
        A::Expr::Unary { op, rhs } => {
            let v = eval_const(&rhs.node)?;
            match (op, v) {
                (A::UnOp::Neg, ConstVal::Int(i)) => Ok(ConstVal::Int(i.wrapping_neg())),
                (A::UnOp::Neg, ConstVal::Float(f)) => Ok(ConstVal::Float(-f)),
                (A::UnOp::Not, ConstVal::Bool(b)) => Ok(ConstVal::Bool(!b)),
                _ => Err(()),
            }
        }
        A::Expr::Binary { op, lhs, rhs } => {
            let l = eval_const(&lhs.node)?;
            let r = eval_const(&rhs.node)?;
            eval_binary(*op, l, r)
        }
        _ => Err(()),
    }
}

fn eval_binary(op: A::BinOp, l: ConstVal, r: ConstVal) -> Result<ConstVal, ()> {
    match (op, l, r) {
        (A::BinOp::Add, ConstVal::Int(a), ConstVal::Int(b)) => Ok(ConstVal::Int(a.wrapping_add(b))),
        (A::BinOp::Sub, ConstVal::Int(a), ConstVal::Int(b)) => Ok(ConstVal::Int(a.wrapping_sub(b))),
        (A::BinOp::Mul, ConstVal::Int(a), ConstVal::Int(b)) => Ok(ConstVal::Int(a.wrapping_mul(b))),
        (A::BinOp::Div, ConstVal::Int(a), ConstVal::Int(b)) => {
            if b == 0 {
                return Err(());
            }
            Ok(ConstVal::Int(a.wrapping_div(b)))
        }
        (A::BinOp::Mod, ConstVal::Int(a), ConstVal::Int(b)) => {
            if b == 0 {
                return Err(());
            }
            Ok(ConstVal::Int(a.wrapping_rem(b)))
        }
        (A::BinOp::Add, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Float(a + b)),
        (A::BinOp::Sub, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Float(a - b)),
        (A::BinOp::Mul, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Float(a * b)),
        (A::BinOp::Div, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Float(a / b)),
        (A::BinOp::Add, ConstVal::Str(a), ConstVal::Str(b)) => Ok(ConstVal::Str(a + &b)),
        (A::BinOp::Eq, a, b) => Ok(ConstVal::Bool(a == b)),
        (A::BinOp::NotEq, a, b) => Ok(ConstVal::Bool(a != b)),
        (A::BinOp::Lt, ConstVal::Int(a), ConstVal::Int(b)) => Ok(ConstVal::Bool(a < b)),
        (A::BinOp::LtEq, ConstVal::Int(a), ConstVal::Int(b)) => Ok(ConstVal::Bool(a <= b)),
        (A::BinOp::Gt, ConstVal::Int(a), ConstVal::Int(b)) => Ok(ConstVal::Bool(a > b)),
        (A::BinOp::GtEq, ConstVal::Int(a), ConstVal::Int(b)) => Ok(ConstVal::Bool(a >= b)),
        (A::BinOp::Lt, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Bool(a < b)),
        (A::BinOp::LtEq, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Bool(a <= b)),
        (A::BinOp::Gt, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Bool(a > b)),
        (A::BinOp::GtEq, ConstVal::Float(a), ConstVal::Float(b)) => Ok(ConstVal::Bool(a >= b)),
        (A::BinOp::And, ConstVal::Bool(a), ConstVal::Bool(b)) => Ok(ConstVal::Bool(a && b)),
        (A::BinOp::Or, ConstVal::Bool(a), ConstVal::Bool(b)) => Ok(ConstVal::Bool(a || b)),
        _ => Err(()),
    }
}

fn type_to_string(ty: &A::Type) -> String {
    let mut out = ty.path.join(".");
    if !ty.args.is_empty() {
        let inner: Vec<String> = ty.args.iter().map(type_to_string).collect();
        out.push('<');
        out.push_str(&inner.join(", "));
        out.push('>');
    }
    if ty.nullable {
        out.push('?');
    }
    out
}

struct TypeShape {
    name: String,
    fields: Vec<(String, String, Span)>,
}

fn shape_of(decl: &A::Decl) -> Option<TypeShape> {
    match decl {
        A::Decl::Struct { name, members, .. } => {
            let mut fields = Vec::new();
            for m in members {
                if let A::ClassMember::Field(f) = &m.node {
                    let ty = f.ty.as_ref().map(type_to_string).unwrap_or_else(|| "Any".to_string());
                    fields.push((f.name.clone(), ty, m.span));
                }
            }
            Some(TypeShape { name: name.clone(), fields })
        }
        A::Decl::Class { name, members, .. } => {
            let mut fields = Vec::new();
            for m in members {
                if let A::ClassMember::Field(f) = &m.node {
                    let ty = f.ty.as_ref().map(type_to_string).unwrap_or_else(|| "Any".to_string());
                    fields.push((f.name.clone(), ty, m.span));
                }
            }
            Some(TypeShape { name: name.clone(), fields })
        }
        _ => None,
    }
}

fn member_names(decl: &A::Decl) -> Vec<(String, Span)> {
    match decl {
        A::Decl::Class { members, .. } | A::Decl::Struct { members, .. } => members
            .iter()
            .filter_map(|m| match &m.node {
                A::ClassMember::Field(f) => Some((f.name.clone(), m.span)),
                A::ClassMember::Method(f) => Some((f.name.clone(), m.span)),
                A::ClassMember::Init { .. } => Some(("init".to_string(), m.span)),
                A::ClassMember::Deinit(_) => Some(("deinit".to_string(), m.span)),
                A::ClassMember::OnReload { .. } => Some(("onReload".to_string(), m.span)),
            })
            .collect(),
        _ => Vec::new(),
    }
}

fn derive_names(attr: &A::Attr) -> Vec<(String, Span)> {
    attr.args
        .iter()
        .filter_map(|a| match &a.node {
            A::Expr::Ident(n) => Some((n.clone(), a.span)),
            _ => None,
        })
        .collect()
}

fn debug_method(shape: &TypeShape, use_span: Span) -> A::FnDecl {
    let mut parts: Vec<A::InterpPart> = Vec::new();
    parts.push(A::InterpPart::Text(format!("{}(", shape.name)));
    for (i, (fname, _, _)) in shape.fields.iter().enumerate() {
        if i > 0 {
            parts.push(A::InterpPart::Text(", ".to_string()));
        }
        parts.push(A::InterpPart::Text(format!("{fname}=")));
        parts.push(A::InterpPart::Expr(A::sp(
            A::Expr::Member {
                base: Box::new(A::sp(A::Expr::This, use_span)),
                field: fname.clone(),
            },
            use_span,
        )));
    }
    parts.push(A::InterpPart::Text(")".to_string()));
    A::FnDecl {
        access: A::Access::Internal,
        name: "debug".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        ret: Some(A::Type::named("String")),
        throws: false,
        is_unsafe: false,
        is_async: false,
        is_static: false,
        is_test: false,
        is_bench: false,
        body: A::FnBody::Block(A::Block {
            stmts: vec![A::sp(
                A::Stmt::Return(Some(A::sp(A::Expr::Interp(parts), use_span))),
                use_span,
            )],
        }),
        attrs: Vec::new(),
        docs: String::new(),
    }
}

fn call_arg(value: A::Spanned<A::Expr>) -> A::CallArg {
    A::CallArg { name: None, value }
}

fn this_field_exprs(shape: &TypeShape, use_span: Span) -> Vec<A::Spanned<A::Expr>> {
    shape
        .fields
        .iter()
        .map(|(fname, _, _)| {
            A::sp(
                A::Expr::Member {
                    base: Box::new(A::sp(A::Expr::This, use_span)),
                    field: fname.clone(),
                },
                use_span,
            )
        })
        .collect()
}

fn clone_method(shape: &TypeShape, use_span: Span, is_class: bool) -> A::FnDecl {
    let field_exprs = this_field_exprs(shape, use_span);
    let ctor = if is_class {
        A::Expr::New {
            target: shape.name.clone(),
            type_args: Vec::new(),
            args: field_exprs.into_iter().map(call_arg).collect(),
        }
    } else {
        A::Expr::Call {
            callee: Box::new(A::sp(A::Expr::Ident(shape.name.clone()), use_span)),
            type_args: Vec::new(),
            args: field_exprs.into_iter().map(call_arg).collect(),
            trailing: None,
        }
    };
    A::FnDecl {
        access: A::Access::Internal,
        name: "clone".to_string(),
        type_params: Vec::new(),
        params: Vec::new(),
        ret: Some(A::Type::named(&shape.name)),
        throws: false,
        is_unsafe: false,
        is_async: false,
        is_static: false,
        is_test: false,
        is_bench: false,
        body: A::FnBody::Block(A::Block {
            stmts: vec![A::sp(
                A::Stmt::Return(Some(A::sp(ctor, use_span))),
                use_span,
            )],
        }),
        attrs: Vec::new(),
        docs: String::new(),
    }
}

fn retag_expr(expr: &mut A::Expr, span: Span) {
    match expr {
        A::Expr::Array(items) => {
            for i in items {
                i.expr.span = span;
                retag_expr(&mut i.expr.node, span);
            }
        }
        A::Expr::Call { callee, args, trailing, .. } => {
            callee.span = span;
            retag_expr(&mut callee.node, span);
            for a in args {
                a.value.span = span;
                retag_expr(&mut a.value.node, span);
            }
            if let Some(b) = trailing {
                retag_block(b, span);
            }
        }
        A::Expr::Binary { lhs, rhs, .. } => {
            lhs.span = span;
            rhs.span = span;
            retag_expr(&mut lhs.node, span);
            retag_expr(&mut rhs.node, span);
        }
        A::Expr::Unary { rhs, .. } => {
            rhs.span = span;
            retag_expr(&mut rhs.node, span);
        }
        A::Expr::Member { base, .. } => {
            base.span = span;
            retag_expr(&mut base.node, span);
        }
        A::Expr::Interp(parts) => {
            for p in parts {
                if let A::InterpPart::Expr(e) = p {
                    e.span = span;
                    retag_expr(&mut e.node, span);
                }
            }
        }
        A::Expr::Closure { params, body, .. } => {
            for p in params {
                p.span = span;
            }
            match body {
                A::FnBody::Block(b) => retag_block(b, span),
                A::FnBody::Expr(e) => {
                    e.span = span;
                    retag_expr(&mut e.node, span);
                }
            }
        }
        _ => {}
    }
}

fn retag_block(block: &mut A::Block, span: Span) {
    for s in &mut block.stmts {
        s.span = span;
        retag_stmt(&mut s.node, span);
    }
}

fn retag_stmt(stmt: &mut A::Stmt, span: Span) {
    match stmt {
        A::Stmt::Var { value, .. } => {
            value.span = span;
            retag_expr(&mut value.node, span);
        }
        A::Stmt::Expr(e) | A::Stmt::Assert(e) => {
            e.span = span;
            retag_expr(&mut e.node, span);
        }
        A::Stmt::Return(e) => {
            if let Some(e) = e {
                e.span = span;
                retag_expr(&mut e.node, span);
            }
        }
        A::Stmt::If { then, otherwise, .. } => {
            retag_block(then, span);
            if let Some(A::Else::Block(b)) = otherwise {
                retag_block(b, span);
            }
        }
        _ => {}
    }
}

fn retag_fn(f: &mut A::FnDecl, span: Span) {
    for p in &mut f.params {
        p.span = span;
    }
    match &mut f.body {
        A::FnBody::Block(b) => retag_block(b, span),
        A::FnBody::Expr(e) => {
            e.span = span;
            retag_expr(&mut e.node, span);
        }
    }
}

struct Gensym {
    counter: u64,
}

impl Gensym {
    fn fresh(&mut self, name: &str) -> String {
        self.counter += 1;
        format!("__m{}_{}", self.counter, name)
    }
}

fn hygienize_fn(f: &mut A::FnDecl, gensym: &mut Gensym) {
    let mut scopes: Vec<Vec<(String, String)>> = vec![Vec::new()];
    for p in &mut f.params {
        let fresh = gensym.fresh(&p.name);
        scopes[0].push((p.name.clone(), fresh.clone()));
        p.name = fresh;
    }
    match &mut f.body {
        A::FnBody::Block(b) => hygienize_block(b, &mut scopes, gensym),
        A::FnBody::Expr(e) => hygienize_expr(&mut e.node, &mut scopes, gensym),
    }
}

fn lookup(scopes: &[Vec<(String, String)>], name: &str) -> Option<String> {
    for scope in scopes.iter().rev() {
        for (old, new) in scope.iter().rev() {
            if old == name {
                return Some(new.clone());
            }
        }
    }
    None
}

fn hygienize_block(
    block: &mut A::Block,
    scopes: &mut Vec<Vec<(String, String)>>,
    gensym: &mut Gensym,
) {
    scopes.push(Vec::new());
    for s in &mut block.stmts {
        hygienize_stmt(&mut s.node, scopes, gensym);
    }
    scopes.pop();
}

fn hygienize_stmt(
    stmt: &mut A::Stmt,
    scopes: &mut Vec<Vec<(String, String)>>,
    gensym: &mut Gensym,
) {
    match stmt {
        A::Stmt::Var { name, value, .. } => {
            hygienize_expr(&mut value.node, scopes, gensym);
            let fresh = gensym.fresh(name);
            if let Some(top) = scopes.last_mut() {
                top.push((name.clone(), fresh.clone()));
            }
            *name = fresh;
        }
        A::Stmt::Expr(e) => hygienize_expr(&mut e.node, scopes, gensym),
        A::Stmt::Return(e) => {
            if let Some(e) = e {
                hygienize_expr(&mut e.node, scopes, gensym);
            }
        }
        A::Stmt::If { cond, then, otherwise } => {
            match cond {
                A::IfCond::Expr(e) => hygienize_expr(&mut e.node, scopes, gensym),
                A::IfCond::Let { name, value } => {
                    hygienize_expr(&mut value.node, scopes, gensym);
                    let fresh = gensym.fresh(name);
                    scopes.push(vec![(name.clone(), fresh.clone())]);
                    *name = fresh;
                    hygienize_block(then, scopes, gensym);
                    scopes.pop();
                    if let Some(A::Else::Block(b)) = otherwise {
                        hygienize_block(b, scopes, gensym);
                    }
                    return;
                }
            }
            hygienize_block(then, scopes, gensym);
            if let Some(A::Else::Block(b)) = otherwise {
                hygienize_block(b, scopes, gensym);
            }
        }
        A::Stmt::For { binding, iter, body } => {
            hygienize_expr(&mut iter.node, scopes, gensym);
            let rename = |names: &mut Vec<String>, gensym: &mut Gensym| {
                let fresh: Vec<String> = names.iter().map(|n| gensym.fresh(n)).collect();
                let pairs: Vec<(String, String)> =
                    names.iter().cloned().zip(fresh.clone()).collect();
                *names = fresh;
                pairs
            };
            let pairs = match binding {
                A::ForBinding::One(n) => {
                    let mut v = vec![n.clone()];
                    let p = rename(&mut v, gensym);
                    *n = v.pop().unwrap_or_default();
                    p
                }
                A::ForBinding::Many(v) => rename(v, gensym),
            };
            scopes.push(pairs);
            hygienize_block(body, scopes, gensym);
            scopes.pop();
        }
        _ => {}
    }
}

fn hygienize_expr(
    expr: &mut A::Expr,
    scopes: &mut Vec<Vec<(String, String)>>,
    gensym: &mut Gensym,
) {
    match expr {
        A::Expr::Ident(n) => {
            if let Some(fresh) = lookup(scopes, n) {
                *n = fresh;
            }
        }
        A::Expr::Array(items) => {
            for i in items {
                hygienize_expr(&mut i.expr.node, scopes, gensym);
            }
        }
        A::Expr::Call { callee, args, .. } => {
            hygienize_expr(&mut callee.node, scopes, gensym);
            for a in args {
                hygienize_expr(&mut a.value.node, scopes, gensym);
            }
        }
        A::Expr::Binary { lhs, rhs, .. } => {
            hygienize_expr(&mut lhs.node, scopes, gensym);
            hygienize_expr(&mut rhs.node, scopes, gensym);
        }
        A::Expr::Member { base, .. } => hygienize_expr(&mut base.node, scopes, gensym),
        A::Expr::Closure { params, body, .. } => {
            let mut pairs = Vec::new();
            for p in params.iter_mut() {
                let fresh = gensym.fresh(&p.name);
                pairs.push((p.name.clone(), fresh.clone()));
                p.name = fresh;
            }
            scopes.push(pairs);
            match body {
                A::FnBody::Block(b) => hygienize_block(b, scopes, gensym),
                A::FnBody::Expr(e) => hygienize_expr(&mut e.node, scopes, gensym),
            }
            scopes.pop();
        }
        _ => {}
    }
}

fn check_attrs_closed(decl_kind: &str, attrs: &[A::Attr], diags: &mut Vec<Diagnostic>) {
    for a in attrs {
        if a.inner {
            continue;
        }
        if !KNOWN_ATTRS.contains(&a.name.as_str()) {
            diags.push(
                Diagnostic::new(
                    Code::E108,
                    format!("unknown attribute `#[{}]` on {decl_kind}", a.name),
                )
                .with_span(a.span)
                .with_hint("the attribute namespace is closed; user extensions use #[Derive(Name)]"),
            );
        }
    }
}

fn strip_consumed_derives(node: &mut A::Decl, consumed: &[String]) {
    if consumed.is_empty() {
        return;
    }
    let attrs: Option<&mut Vec<A::Attr>> = match node {
        A::Decl::Class { attrs, .. }
        | A::Decl::Struct { attrs, .. }
        | A::Decl::Record { attrs, .. }
        | A::Decl::Enum { attrs, .. } => Some(attrs),
        _ => None,
    };
    if let Some(attrs) = attrs {
        for a in attrs.iter_mut() {
            if a.name != "Derive" {
                continue;
            }
            a.args.retain(|x| match &x.node {
                A::Expr::Ident(n) => !consumed.contains(n),
                _ => true,
            });
        }
        attrs.retain(|a| a.name != "Derive" || !a.args.is_empty());
    }
}

pub fn expand_phase_b(
    module: &mut A::Module,
    opts: &mut MacroOptions,
    runner: Option<&dyn ComptimeRunner>,
) -> Vec<Diagnostic> {
    if let Some(dir) = opts.dump_dir.clone() {
        let _ = std::fs::create_dir_all(&dir);
        let _ = std::fs::write(dir.join("phase-b.before"), format!("{module:#?}"));
    }
    let mut diags = Vec::new();
    let mut comptime_fns: BTreeMap<String, Span> = BTreeMap::new();
    let mut short_counts: BTreeMap<String, usize> = BTreeMap::new();
    for decl in &module.decls {
        if let A::Decl::Fn(f) = &decl.node {
            if f.attrs.iter().any(|a| a.name == "CompTime") {
                comptime_fns.insert(f.name.clone(), decl.span);
                if let Some(short) = f.name.rsplit('.').next() {
                    *short_counts.entry(short.to_string()).or_insert(0) += 1;
                }
            }
        }
    }
    for (full, span) in comptime_fns.clone() {
        if let Some(short) = full.rsplit('.').next() {
            if short != full && short_counts.get(short) == Some(&1) {
                comptime_fns.entry(short.to_string()).or_insert(span);
            }
        }
    }
    for decl in &module.decls {
        match &decl.node {
            A::Decl::Class { attrs, .. } => check_attrs_closed("class", attrs, &mut diags),
            A::Decl::Struct { attrs, .. } => check_attrs_closed("struct", attrs, &mut diags),
            A::Decl::Record { attrs, .. } => check_attrs_closed("record", attrs, &mut diags),
            A::Decl::Enum { attrs, .. } => check_attrs_closed("enum", attrs, &mut diags),
            A::Decl::Fn(f) => check_attrs_closed("fn", &f.attrs, &mut diags),
            _ => {}
        }
    }
    if diags.iter().any(|d| !d.code.is_warning()) {
        return diags;
    }
    let mut gensym = Gensym { counter: 0 };
    for iter in 0..ITER_CAP {
        let mut changed = false;
        for idx in 0..module.decls.len() {
            let derives: Vec<(String, Span)> = match &module.decls[idx].node {
                A::Decl::Class { attrs, .. }
                | A::Decl::Struct { attrs, .. }
                | A::Decl::Record { attrs, .. }
                | A::Decl::Enum { attrs, .. } => attrs
                    .iter()
                    .filter(|a| a.name == "Derive")
                    .flat_map(derive_names)
                    .collect(),
                _ => Vec::new(),
            };
            if derives.is_empty() {
                continue;
            }
            let mut consumed: Vec<String> = Vec::new();
            for (dname, dspan) in derives {
                if dname == "Clone" || dname == "Debug" {
                    let shape = shape_of(&module.decls[idx].node);
                    match apply_builtin_derive(
                        &mut module.decls[idx].node,
                        &dname,
                        shape.as_ref(),
                        dspan,
                        &mut gensym,
                    ) {
                        Ok(did) => {
                            changed = changed || did;
                            if did {
                                consumed.push(dname.clone());
                            }
                        }
                        Err(d) => diags.push(d),
                    }
                    continue;
                }
                if dname == "Serde" {
                    diags.push(
                        Diagnostic::new(
                            Code::E108,
                            "`#[Derive(Serde)]` is out of scope for v1; derive Clone or Debug, or a #[CompTime] fn",
                        )
                        .with_span(dspan)
                        .with_hint("Serde is deferred past v1"),
                    );
                    continue;
                }
                let shape = shape_of(&module.decls[idx].node);
                let Some(shape) = shape.as_ref() else {
                    diags.push(
                        Diagnostic::new(
                            Code::E108,
                            format!("`#[Derive({dname})]` needs a struct, class, or record"),
                        )
                        .with_span(dspan),
                    );
                    continue;
                };
                let Some(cspan) = comptime_fns.get(&dname).cloned() else {
                    diags.push(
                        Diagnostic::new(
                            Code::E108,
                            format!("`#[Derive({dname})]` resolves to no #[CompTime] fn (crate-local only in v1)"),
                        )
                        .with_span(dspan)
                        .with_hint("define a crate-local #[CompTime] fn with that name"),
                    );
                    continue;
                };
                let Some(r) = runner else {
                    continue;
                };
                match apply_user_derive(
                    &mut module.decls[idx].node,
                    shape,
                    &dname,
                    cspan,
                    dspan,
                    r,
                    &mut gensym,
                    opts,
                ) {
                    Ok(did) => {
                        changed = changed || did;
                        if did {
                            consumed.push(dname.clone());
                        }
                    }
                    Err(d) => diags.push(d),
                }
            }
            strip_consumed_derives(&mut module.decls[idx].node, &consumed);
        }
        if !changed {
            let _ = iter;
            break;
        }
        if iter + 1 >= ITER_CAP {
            diags.push(
                Diagnostic::new(
                    Code::E120,
                    format!("derive expansion did not settle after {ITER_CAP} iterations"),
                )
                .with_hint("a derive is emitting derives that expand forever"),
            );
        }
    }
    let mut bctx = PhaseBCtx {
        opts,
        runner,
        comptime_fns: comptime_fns.clone(),
        diags: Vec::new(),
        changed: false,
        node_count: 0,
        capped: false,
    };
    for decl in &mut module.decls {
        bctx.rewrite_decl(&mut decl.node, decl.span);
        if bctx.capped {
            break;
        }
    }
    diags.append(&mut bctx.diags);
    if let Some(dir) = opts.dump_dir.clone() {
        let _ = std::fs::write(dir.join("phase-b.after"), format!("{module:#?}"));
    }
    diags
}

fn apply_builtin_derive(
    node: &mut A::Decl,
    dname: &str,
    shape: Option<&TypeShape>,
    use_span: Span,
    gensym: &mut Gensym,
) -> Result<bool, Diagnostic> {
    let Some(shape) = shape else {
        return Err(Diagnostic::new(
            Code::E108,
            format!("`#[Derive({dname})]` needs a struct or class"),
        )
        .with_span(use_span));
    };
    let existing = member_names(node);
    let is_class = matches!(node, A::Decl::Class { .. });
    let mut method = match dname {
        "Debug" => debug_method(shape, use_span),
        "Clone" => {
            if is_class {
                match class_init_arity(node) {
                    Some(n) if n == shape.fields.len() => {}
                    Some(n) => {
                        return Err(Diagnostic::new(
                            Code::E108,
                            format!(
                                "`#[Derive(Clone)]` needs a positional init matching {} fields, found {n}",
                                shape.fields.len()
                            ),
                        )
                        .with_span(use_span));
                    }
                    None => {
                        return Err(Diagnostic::new(
                            Code::E108,
                            "`#[Derive(Clone)]` needs an explicit init with positional params",
                        )
                        .with_span(use_span));
                    }
                }
            }
            clone_method(shape, use_span, is_class)
        }
        _ => {
            return Err(Diagnostic::new(Code::E108, "unknown builtin derive").with_span(use_span));
        }
    };
    if let Some((_, prev)) = existing.iter().find(|(n, _)| *n == method.name) {
        return Err(Diagnostic::new(
            Code::E121,
            format!(
                "derive would collide with existing member `{}` (derive at {}..{})",
                method.name, use_span.start, use_span.end
            ),
        )
        .with_span(*prev)
        .with_hint("rename the existing member or drop the derive"));
    }
    hygienize_fn(&mut method, gensym);
    match node {
        A::Decl::Class { members, .. } | A::Decl::Struct { members, .. } => {
            members.push(A::sp(A::ClassMember::Method(method), use_span));
            Ok(true)
        }
        _ => Err(Diagnostic::new(
            Code::E108,
            format!("`#[Derive({dname})]` needs a struct or class"),
        )
        .with_span(use_span)),
    }
}

fn class_init_arity(node: &A::Decl) -> Option<usize> {
    match node {
        A::Decl::Class { members, .. } => members.iter().find_map(|m| match &m.node {
            A::ClassMember::Init { params, .. } => Some(params.len()),
            _ => None,
        }),
        _ => None,
    }
}

fn apply_user_derive(
    node: &mut A::Decl,
    shape: &TypeShape,
    dname: &str,
    cspan: Span,
    use_span: Span,
    runner: &dyn ComptimeRunner,
    gensym: &mut Gensym,
    opts: &mut MacroOptions,
) -> Result<bool, Diagnostic> {
    let mut sorted = shape.fields.clone();
    sorted.sort_by(|a, b| a.0.cmp(&b.0).then(a.1.cmp(&b.1)));
    let mut info_items = vec![ComptimeValue::Str(shape.name.clone())];
    for (fname, fty, _) in &sorted {
        info_items.push(ComptimeValue::Str(format!("{fname}:{fty}")));
    }
    let info_debug = format!("{info_items:?}");
    let key = opts.cache_key(dname, &info_debug, &[]);
    let sources: Vec<String> = if let Some(hit) = opts.methods_get(&key).cloned() {
        hit
    } else {
        let out = runner.call(dname, cspan, vec![ComptimeValue::Array(info_items)], use_span)?;
        let items = match out {
            ComptimeValue::Array(items) => items,
            _ => {
                return Err(Diagnostic::new(
                    Code::E122,
                    format!("derive fn `{dname}` must return an Array of method sources"),
                )
                .with_span(use_span)
                .with_hint(format!("defined at {}..{}", cspan.start, cspan.end)));
            }
        };
        let mut sources = Vec::new();
        for item in items {
            match item {
                ComptimeValue::Str(s) => sources.push(s),
                _ => {
                    return Err(Diagnostic::new(
                        Code::E122,
                        format!("derive fn `{dname}` must return method source strings"),
                    )
                    .with_span(use_span));
                }
            }
        }
        opts.methods_insert(key.clone(), sources.clone());
        sources
    };
    let existing = member_names(node);
    let mut added = false;
    for src in sources {
        let mut m = Parser::parse_module(&src).map_err(|e| {
            Diagnostic::new(
                Code::E122,
                format!("derive fn `{dname}` returned unparseable code: {e}"),
            )
            .with_span(use_span)
            .with_hint(format!("defined at {}..{}", cspan.start, cspan.end))
        })?;
        if m.decls.len() != 1 {
            return Err(Diagnostic::new(
                Code::E122,
                format!("derive fn `{dname}` must return one method per item"),
            )
            .with_span(use_span));
        }
        let first = m.decls.pop().unwrap_or_else(|| unreachable!());
        match first.node {
            A::Decl::Fn(mut f) => {
                if let Some((_, prev)) = existing.iter().find(|(n, _)| *n == f.name) {
                    return Err(Diagnostic::new(
                        Code::E121,
                        format!(
                            "derive would collide with existing member `{}` (derive at {}..{})",
                            f.name, use_span.start, use_span.end
                        ),
                    )
                    .with_span(*prev)
                    .with_hint(format!("derive fn defined at {}..{}", cspan.start, cspan.end)));
                }
                retag_fn(&mut f, use_span);
                hygienize_fn(&mut f, gensym);
                match node {
                    A::Decl::Class { members, .. } | A::Decl::Struct { members, .. } => {
                        members.push(A::sp(A::ClassMember::Method(f), use_span));
                        added = true;
                    }
                    _ => {
                        return Err(Diagnostic::new(
                            Code::E108,
                            format!("`#[Derive({dname})]` needs a struct or class"),
                        )
                        .with_span(use_span));
                    }
                }
            }
            _ => {
                return Err(Diagnostic::new(
                    Code::E122,
                    format!("derive fn `{dname}` must return method sources"),
                )
                .with_span(use_span));
            }
        }
    }
    Ok(added)
}

struct PhaseBCtx<'a> {
    opts: &'a mut MacroOptions,
    runner: Option<&'a dyn ComptimeRunner>,
    comptime_fns: BTreeMap<String, Span>,
    diags: Vec<Diagnostic>,
    changed: bool,
    node_count: usize,
    capped: bool,
}

impl PhaseBCtx<'_> {
    fn bump(&mut self, span: Span) {
        self.node_count += 1;
        if self.node_count > NODE_CAP && !self.capped {
            self.capped = true;
            self.diags.push(
                Diagnostic::new(
                    Code::E119,
                    format!("macro output exceeds {NODE_CAP} nodes"),
                )
                .with_span(span),
            );
        }
    }

    fn rewrite_decl(&mut self, node: &mut A::Decl, span: Span) {
        match node {
            A::Decl::Fn(f) => self.rewrite_fn(f),
            A::Decl::Const { value, .. } => {
                let vs = value.span;
                if self.rewrite_comptime_value(&mut value.node, vs) {
                    self.changed = true;
                }
                self.rewrite_expr(&mut value.node, vs);
            }
            A::Decl::Class { members, .. } | A::Decl::Struct { members, .. } => {
                for m in members {
                    match &mut m.node {
                        A::ClassMember::Field(f) => {
                            if let Some(v) = f.value.as_mut() {
                                let vs = v.span;
                                self.rewrite_expr(&mut v.node, vs);
                            }
                        }
                        A::ClassMember::Method(f) => self.rewrite_fn(f),
                        A::ClassMember::Init { body, .. }
                        | A::ClassMember::Deinit(body)
                        | A::ClassMember::OnReload { body, .. } => self.rewrite_block(body),
                    }
                }
            }
            A::Decl::Stmt(s) => {
                let ss = s.span;
                self.rewrite_stmt(&mut s.node, ss);
            }
            _ => {}
        }
        let _ = span;
    }

    fn rewrite_fn(&mut self, f: &mut A::FnDecl) {
        match &mut f.body {
            A::FnBody::Block(b) => self.rewrite_block(b),
            A::FnBody::Expr(e) => {
                let s = e.span;
                self.rewrite_expr(&mut e.node, s);
            }
        }
    }

    fn rewrite_block(&mut self, block: &mut A::Block) {
        for s in &mut block.stmts {
            if self.capped {
                return;
            }
            let ss = s.span;
            if let A::Stmt::Var { is_static: true, value, .. } = &mut s.node {
                let vs = value.span;
                if self.rewrite_comptime_value(&mut value.node, vs) {
                    self.changed = true;
                }
            }
            self.rewrite_stmt(&mut s.node, ss);
        }
    }

    fn rewrite_stmt(&mut self, stmt: &mut A::Stmt, span: Span) {
        self.bump(span);
        if self.capped {
            return;
        }
        match stmt {
            A::Stmt::Var { value, .. } => {
                let s = value.span;
                self.rewrite_expr(&mut value.node, s);
            }
            A::Stmt::Expr(e) => {
                let s = e.span;
                self.rewrite_expr(&mut e.node, s);
                if !matches!(&e.node, A::Expr::Macro { .. }) {
                    return;
                }
                if let A::Expr::Macro { name, args, .. } = &e.node {
                    if name == "StaticAssert" {
                        let owned = args.clone();
                        self.apply_static_assert(&owned, s);
                    }
                }
            }
            A::Stmt::Return(e) => {
                if let Some(e) = e {
                    let s = e.span;
                    self.rewrite_expr(&mut e.node, s);
                }
            }
            A::Stmt::If { cond, then, otherwise } => {
                match cond {
                    A::IfCond::Expr(e) => {
                        let s = e.span;
                        self.rewrite_expr(&mut e.node, s);
                    }
                    A::IfCond::Let { value, .. } => {
                        let s = value.span;
                        self.rewrite_expr(&mut value.node, s);
                    }
                }
                self.rewrite_block(then);
                if let Some(A::Else::Block(b)) = otherwise {
                    self.rewrite_block(b);
                }
            }
            A::Stmt::For { iter, body, .. } => {
                let s = iter.span;
                self.rewrite_expr(&mut iter.node, s);
                self.rewrite_block(body);
            }
            A::Stmt::While { cond, body } | A::Stmt::DoWhile { cond, body } => {
                let s = cond.span;
                self.rewrite_expr(&mut cond.node, s);
                self.rewrite_block(body);
            }
            _ => {}
        }
    }

    fn rewrite_expr(&mut self, expr: &mut A::Expr, span: Span) {
        self.bump(span);
        if self.capped {
            return;
        }
        let is_sa = matches!(expr, A::Expr::Macro { name, .. } if name == "StaticAssert");
        let is_ct = matches!(expr, A::Expr::Macro { name, .. } if name == "CompTime");
        if is_sa {
            if let A::Expr::Macro { args, .. } = expr {
                for a in args.iter_mut() {
                    let s = a.span;
                    self.rewrite_expr(&mut a.node, s);
                }
                let owned: Vec<A::Spanned<A::Expr>> = args.clone();
                self.apply_static_assert(&owned, span);
            }
            if !self.diags.iter().any(|d| d.span == Some(span)) {
                *expr = A::Expr::Bool(true);
                self.changed = true;
            }
            return;
        }
        if is_ct {
            if let A::Expr::Macro { args, .. } = expr {
                if let Some(first) = args.first().cloned() {
                    let mut owned = std::mem::replace(expr, A::Expr::Bool(true));
                    if self.rewrite_comptime_call(&mut owned, &first, span) {
                        *expr = owned;
                        self.changed = true;
                        return;
                    }
                    *expr = owned;
                    return;
                }
            }
        }
        match expr {
            A::Expr::Array(items) => {
                for i in items {
                    let s = i.expr.span;
                    self.rewrite_expr(&mut i.expr.node, s);
                }
            }
            A::Expr::Call { callee, args, .. } => {
                let s = callee.span;
                self.rewrite_expr(&mut callee.node, s);
                for a in args {
                    let s = a.value.span;
                    self.rewrite_expr(&mut a.value.node, s);
                }
            }            A::Expr::Binary { lhs, rhs, .. } => {
                let (ls, rs) = (lhs.span, rhs.span);
                self.rewrite_expr(&mut lhs.node, ls);
                self.rewrite_expr(&mut rhs.node, rs);
            }
            A::Expr::Macro { args, .. } => {
                for a in args.iter_mut() {
                    let s = a.span;
                    self.rewrite_expr(&mut a.node, s);
                }
            }
            _ => {}
        }
    }

    fn apply_static_assert(&mut self, args: &[A::Spanned<A::Expr>], span: Span) {
        let msg = args
            .get(1)
            .and_then(|a| as_str_lit(&a.node))
            .unwrap_or_else(|| "static assertion failed".to_string());
        let Some(first) = args.first() else {
            self.diags.push(
                Diagnostic::new(Code::E123, "StaticAssert! needs a condition").with_span(span),
            );
            return;
        };
        match eval_const(&first.node) {
            Ok(v) => match v.truthy() {
                Some(true) => {}
                _ => {
                    self.diags.push(
                        Diagnostic::new(Code::E123, format!("static assertion failed: {msg}"))
                            .with_span(first.span)
                            .with_hint("StaticAssert! evaluates at build time"),
                    );
                }
            },
            Err(()) => {
                self.diags.push(
                    Diagnostic::new(
                        Code::E123,
                        format!("StaticAssert! condition is not constant: {msg}"),
                    )
                    .with_span(first.span)
                    .with_hint("use literals and constant arithmetic"),
                );
            }
        }
    }

    fn rewrite_comptime_value(&mut self, expr: &mut A::Expr, span: Span) -> bool {
        let inner = match expr {
            A::Expr::Macro { name, args, .. } if name == "CompTime" => {
                args.first().cloned()
            }
            _ => None,
        };
        if let Some(first) = inner {
            let mut owned = std::mem::replace(expr, A::Expr::Bool(true));
            if self.rewrite_comptime_call(&mut owned, &first, span) {
                *expr = owned;
                return true;
            }
            *expr = owned;
        }
        false
    }

    fn rewrite_comptime_call(
        &mut self,
        slot: &mut A::Expr,
        inner: &A::Spanned<A::Expr>,
        span: Span,
    ) -> bool {
        if let Ok(v) = eval_const(&inner.node) {
            *slot = v.to_expr(span);
            return true;
        }
        if let A::Expr::Call { callee, args, .. } = &inner.node {
            if let A::Expr::Ident(fname) = &callee.node {
                let fname = fname.clone();
                let Some(cspan) = self.comptime_fns.get(&fname).cloned() else {
                    self.diags.push(
                        Diagnostic::new(
                            Code::E122,
                            format!("CompTime! calls unknown comptime fn `{fname}`"),
                        )
                        .with_span(inner.span)
                        .with_hint("only #[CompTime] fns run at build time"),
                    );
                    return false;
                };
                let mut cargs = Vec::new();
                for a in args {
                    match eval_const(&a.value.node) {
                        Ok(v) => cargs.push(v.to_comptime()),
                        Err(()) => {
                            self.diags.push(
                                Diagnostic::new(
                                    Code::E122,
                                    "CompTime! arguments must be constant",
                                )
                                .with_span(a.value.span),
                            );
                            return false;
                        }
                    }
                }
                let key = self.opts.cache_key(&fname, &format!("{cargs:?}"), &[]);
                if let Some(hit) = self.opts.cache_get(&key).cloned() {
                    *slot = hit.expr;
                    return true;
                }
                let Some(runner) = self.runner else {
                    return false;
                };
                match runner.call(&fname, cspan, cargs, span) {
                    Ok(v) => {
                        let e = v.to_expr(span);
                        self.opts
                            .cache_insert(key, CachedExpansion { expr: e.clone(), span });
                        *slot = e;
                        return true;
                    }
                    Err(d) => {
                        self.diags.push(d);
                        return false;
                    }
                }
            }
        }
        self.diags.push(
            Diagnostic::new(Code::E122, "CompTime! takes a constant or #[CompTime] call")
                .with_span(inner.span),
        );
        false
    }
}
