use cranelift::jit::Jit;
use diagnostics::Span;
use frontend::ast as A;
use std::collections::{BTreeMap, BTreeSet};

const SPARE_SLOTS: usize = 1024;
const EVAL_FN: &str = "__repl_eval";
const TYPE_FN: &str = "__repl_type";

pub enum ReplOut {
    Exit,
    Lines(Vec<String>),
}

struct FnRecord {
    sig: String,
    body: String,
}

#[derive(Clone)]
struct GlobalInfo {
    getter: String,
    init: Option<A::Expr>,
}

pub struct ReplSession {
    jit: Option<Jit>,
    defs: String,
    def_decls: usize,
    fns: BTreeMap<String, FnRecord>,
    globals: BTreeMap<String, GlobalInfo>,
    snap_idx: usize,
    file_idx: usize,
    session_id: u64,
}

fn zero_span() -> Span {
    Span { start: 0, end: 0 }
}

fn spanned<T>(node: T) -> A::Spanned<T> {
    A::Spanned { node, span: zero_span() }
}

fn err(text: String) -> diagnostics::Diagnostic {
    diagnostics::Diagnostic::new(diagnostics::Code::E108, text)
}

pub fn needs_more(input: &str) -> bool {
    let mut depth = 0i32;
    let mut in_str = false;
    let mut esc = false;
    let mut line_comment = false;
    let mut prev_slash = false;
    for c in input.chars() {
        if line_comment {
            if c == '\n' {
                line_comment = false;
            }
            continue;
        }
        if in_str {
            if esc {
                esc = false;
            } else if c == '\\' {
                esc = true;
            } else if c == '"' {
                in_str = false;
            }
            continue;
        }
        if prev_slash {
            prev_slash = false;
            if c == '/' {
                line_comment = true;
                continue;
            }
        } else if c == '/' {
            prev_slash = true;
            continue;
        }
        match c {
            '"' => in_str = true,
            '{' | '(' | '[' => depth += 1,
            '}' | ')' | ']' => depth -= 1,
            _ => {}
        }
    }
    if depth > 0 || in_str {
        return true;
    }
    let t = input.trim_end();
    if t.is_empty() || prev_slash {
        return false;
    }
    let last = t.chars().last().unwrap_or(' ');
    matches!(last, '+' | '-' | '*' | '/' | '%' | ',' | '=' | '&' | '|' | '<' | '>' | '^' | '?' | ':')
}

fn sig_key_of(f: &A::FnDecl) -> String {
    format!("{:?}|{:?}|{}", f.params.iter().map(|p| &p.ty).collect::<Vec<_>>(), f.ret, f.throws)
}

struct Rewriter<'x> {
    globals: &'x BTreeMap<String, GlobalInfo>,
    bound: Vec<BTreeSet<String>>,
}

impl Rewriter<'_> {
    fn is_bound(&self, name: &str) -> bool {
        self.bound.iter().any(|s| s.contains(name))
    }

    fn bind_here(&mut self, name: &str) {
        if let Some(top) = self.bound.last_mut() {
            top.insert(name.to_string());
        }
    }

    fn rewrite_block(&mut self, b: &mut A::Block) {
        self.bound.push(BTreeSet::new());
        for s in &mut b.stmts {
            self.rewrite_stmt(s);
        }
        self.bound.pop();
    }

    fn rewrite_stmt(&mut self, s: &mut A::Spanned<A::Stmt>) {
        match &mut s.node {
            A::Stmt::Var { value, name, .. } => {
                self.rewrite_expr(value);
                self.bind_here(name);
            }
            A::Stmt::DestructureTuple { names, value, .. } => {
                self.rewrite_expr(value);
                for n in names {
                    self.bind_here(n);
                }
            }
            A::Stmt::DestructureRecord { fields, rest, value } => {
                self.rewrite_expr(value);
                for (_, n) in fields {
                    self.bind_here(n);
                }
                if let Some(r) = rest {
                    self.bind_here(r);
                }
            }
            A::Stmt::DestructureArray { names, rest, value } => {
                self.rewrite_expr(value);
                for n in names {
                    self.bind_here(n);
                }
                if let Some(r) = rest {
                    self.bind_here(r);
                }
            }
            A::Stmt::Assign { target, value, .. } => {
                self.rewrite_expr(target);
                self.rewrite_expr(value);
            }
            A::Stmt::Expr(e) | A::Stmt::Return(Some(e)) | A::Stmt::Assert(e) | A::Stmt::Throw(Some(e)) => {
                self.rewrite_expr(e);
            }
            A::Stmt::If { cond, then, otherwise } => {
                let is_let = matches!(cond, A::IfCond::Let { .. });
                match cond {
                    A::IfCond::Expr(e) => self.rewrite_expr(e),
                    A::IfCond::Let { name, value } => {
                        self.rewrite_expr(value);
                        self.bound.push(BTreeSet::from([name.clone()]));
                        self.rewrite_block(then);
                        self.bound.pop();
                    }
                }
                if !is_let {
                    self.rewrite_block(then);
                }
                if let Some(e) = otherwise {
                    match e {
                        A::Else::Block(b) => self.rewrite_block(b),
                        A::Else::If(s) => self.rewrite_stmt(s),
                    }
                }
            }
            A::Stmt::For { binding, iter, body } => {
                self.rewrite_expr(iter);
                let mut names = BTreeSet::new();
                match binding {
                    A::ForBinding::One(n) => {
                        names.insert(n.clone());
                    }
                    A::ForBinding::Many(ns) => {
                        names.extend(ns.iter().cloned());
                    }
                }
                self.bound.push(names);
                self.rewrite_block(body);
                self.bound.pop();
            }
            A::Stmt::While { cond, body } | A::Stmt::DoWhile { cond, body } => {
                self.rewrite_expr(cond);
                self.rewrite_block(body);
            }
            A::Stmt::Switch { scrutinee, cases, default } => {
                self.rewrite_expr(scrutinee);
                for c in cases {
                    self.rewrite_pattern(&mut c.pattern);
                    if let Some(g) = &mut c.guard {
                        self.rewrite_expr(g);
                    }
                    self.bound.push(BTreeSet::new());
                    for st in &mut c.body {
                        self.rewrite_stmt(st);
                    }
                    self.bound.pop();
                }
                if let Some(d) = default {
                    self.bound.push(BTreeSet::new());
                    for st in d {
                        self.rewrite_stmt(st);
                    }
                    self.bound.pop();
                }
            }
            A::Stmt::Guard { name, value, otherwise } => {
                self.rewrite_expr(value);
                self.bind_here(name);
                self.rewrite_block(otherwise);
            }
            A::Stmt::Try { body, catch, finally } => {
                self.rewrite_block(body);
                if let Some((n, b)) = catch {
                    self.bound.push(BTreeSet::from([n.clone()]));
                    self.rewrite_block(b);
                    self.bound.pop();
                }
                if let Some(f) = finally {
                    self.rewrite_block(f);
                }
            }
            A::Stmt::Defer(b) | A::Stmt::UnsafeBlock(b) => self.rewrite_block(b),
            _ => {}
        }
    }

    fn rewrite_pattern(&mut self, p: &mut A::Pattern) {
        match p {
            A::Pattern::Literal(e) => self.rewrite_expr(e),
            A::Pattern::Range { lo, hi, .. } => {
                self.rewrite_expr(lo);
                self.rewrite_expr(hi);
            }
            A::Pattern::Enum { args, .. } => {
                for a in args {
                    self.rewrite_pattern(a);
                }
            }
            _ => {}
        }
    }

    fn rewrite_fn_body(&mut self, params: &[A::Param], body: &mut A::FnBody) {
        let names: BTreeSet<String> = params.iter().map(|p| p.name.clone()).collect();
        self.bound.push(names);
        match body {
            A::FnBody::Block(b) => self.rewrite_block(b),
            A::FnBody::Expr(e) => self.rewrite_expr(e),
        }
        self.bound.pop();
    }

    fn rewrite_args(&mut self, args: &mut [A::CallArg]) {
        for a in args {
            self.rewrite_expr(&mut a.value);
        }
    }

    fn rewrite_expr(&mut self, e: &mut A::Spanned<A::Expr>) {
        if let A::Expr::Ident(name) = &e.node {
            if let Some(info) = self.globals.get(name) {
                if !self.is_bound(name) {
                    let getter = info.getter.clone();
                    let span = e.span;
                    e.node = A::Expr::Call {
                        callee: Box::new(A::Spanned { node: A::Expr::Ident(getter), span }),
                        type_args: Vec::new(),
                        args: Vec::new(),
                        trailing: None,
                    };
                    return;
                }
            } else {
                return;
            }
        }
        match &mut e.node {
            A::Expr::Binary { lhs, rhs, .. } => {
                self.rewrite_expr(lhs);
                self.rewrite_expr(rhs);
            }
            A::Expr::Unary { rhs, .. } | A::Expr::Await(rhs) | A::Expr::Propagate(rhs) => {
                self.rewrite_expr(rhs);
            }
            A::Expr::Ternary { cond, then, otherwise } => {
                self.rewrite_expr(cond);
                self.rewrite_expr(then);
                self.rewrite_expr(otherwise);
            }
            A::Expr::Coalesce { lhs, rhs } => {
                self.rewrite_expr(lhs);
                self.rewrite_expr(rhs);
            }
            A::Expr::OptChain { base, .. } => self.rewrite_expr(base),
            A::Expr::OptCall { base, args, .. } => {
                self.rewrite_expr(base);
                self.rewrite_args(args);
            }
            A::Expr::Range { lo, hi, .. } => {
                self.rewrite_expr(lo);
                self.rewrite_expr(hi);
            }
            A::Expr::Call { callee, args, trailing, .. } => {
                self.rewrite_expr(callee);
                self.rewrite_args(args);
                if let Some(b) = trailing {
                    self.rewrite_block(b);
                }
            }
            A::Expr::New { args, .. } => self.rewrite_args(args),
            A::Expr::Index { base, index } => {
                self.rewrite_expr(base);
                self.rewrite_expr(index);
            }
            A::Expr::Member { base, .. } | A::Expr::TupleGet { base, .. } => self.rewrite_expr(base),
            A::Expr::Cast { expr, .. } | A::Expr::Is { base: expr, .. } => self.rewrite_expr(expr),
            A::Expr::Macro { args, .. } => {
                for a in args {
                    self.rewrite_expr(a);
                }
            }
            A::Expr::Closure { params, body, .. } => self.rewrite_fn_body(params, body),
            A::Expr::UnsafeBlock(b) => self.rewrite_block(b),
            A::Expr::Tuple(es) => {
                for x in es {
                    self.rewrite_expr(x);
                }
            }
            A::Expr::Array(elems) => {
                for el in elems {
                    self.rewrite_expr(&mut el.expr);
                }
            }
            A::Expr::Record(fs) => {
                for e in fs {
                    self.rewrite_expr(e.value_mut());
                }
            }
            A::Expr::MapLiteral(fs) => {
                for e in fs {
                    self.rewrite_expr(e.value_mut());
                }
            }
            A::Expr::Interp(parts) => {
                for p in parts {
                    if let A::InterpPart::Expr(x) = p {
                        self.rewrite_expr(x);
                    }
                }
            }
            A::Expr::Switch { scrutinee, cases, default } => {
                self.rewrite_expr(scrutinee);
                for c in cases {
                    self.rewrite_pattern(&mut c.pattern);
                    if let Some(g) = &mut c.guard {
                        self.rewrite_expr(g);
                    }
                    match &mut c.body {
                        A::SwitchExprBody::Expr(x) => self.rewrite_expr(x),
                        A::SwitchExprBody::Block(b) => self.rewrite_block(b),
                    }
                }
                if let Some(d) = default {
                    match d {
                        A::SwitchExprBody::Expr(x) => self.rewrite_expr(x),
                        A::SwitchExprBody::Block(b) => self.rewrite_block(b),
                    }
                }
            }
            _ => {}
        }
    }
}

struct Prepared {
    has_eval: bool,
    eval_off: Option<usize>,
    ast_fns: Vec<String>,
}

impl ReplSession {
    pub fn new() -> Self {
        static SESSION_CTR: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
        ReplSession {
            jit: None,
            defs: String::new(),
            def_decls: 0,
            fns: BTreeMap::new(),
            globals: BTreeMap::new(),
            snap_idx: 0,
            file_idx: 0,
            session_id: SESSION_CTR.fetch_add(1, std::sync::atomic::Ordering::SeqCst),
        }
    }

    fn reset_state(&mut self) {
        self.jit = None;
        self.defs.clear();
        self.def_decls = 0;
        self.fns.clear();
        self.globals.clear();
    }

    fn parse_pre_desugar(&mut self, source: &str) -> Result<(A::Module, usize), Vec<diagnostics::Diagnostic>> {
        let dir = std::env::temp_dir().join(format!("rnx-repl-{}-{}-{}", std::process::id(), self.session_id, self.file_idx));
        self.file_idx += 1;
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).map_err(|e| vec![err(format!("repl tempdir: {e}"))])?;
        let file = dir.join("repl.rnx");
        std::fs::write(&file, source).map_err(|e| vec![err(format!("repl write: {e}"))])?;
        let g = frontend::modules::ModuleGraph::build(&file).map_err(|e| vec![e])?;
        let root_count = g.files.iter().find(|f| f.path == g.root).map(|f| f.module.decls.len()).unwrap_or(0);
        let mut m = g.resolve().map_err(|e| vec![e])?;
        frontend::harness::strip_tests(&mut m);
        frontend::harness::strip_benches(&mut m);
        let _ = std::fs::remove_dir_all(&dir);
        Ok((m, root_count))
    }

    fn parse_raw(&mut self, source: &str) -> Result<A::Module, Vec<diagnostics::Diagnostic>> {
        let (mut m, _) = self.parse_pre_desugar(source)?;
        let desugar_errs = frontend::desugar::desugar(&mut m);
        if desugar_errs.is_empty() {
            Ok(m)
        } else {
            Err(desugar_errs)
        }
    }

    fn render_errors(&self, errs: &[diagnostics::Diagnostic]) -> Vec<String> {
        let theme = diagnostics::theme::AuraTheme::active();
        let path = std::path::Path::new("repl.rnx");
        crate::report::render_compile_errors(&theme, path, errs)
            .to_string()
            .lines()
            .map(|l| l.to_string())
            .collect()
    }

    pub fn eval(&mut self, block: &str) -> ReplOut {
        let t = block.trim();
        if t.is_empty() {
            return ReplOut::Lines(Vec::new());
        }
        if let Some(out) = self.directive(t) {
            return out;
        }
        if let Some(rest) = t.strip_prefix(":type ") {
            return ReplOut::Lines(self.type_of(rest.trim()));
        }
        let candidate = if self.defs.is_empty() {
            t.to_string()
        } else {
            format!("{}\n{t}", self.defs)
        };
        let mut module = match self.parse_pre_desugar(&candidate) {
            Ok((m, _)) => m,
            Err(errs) => return ReplOut::Lines(self.render_errors(&errs)),
        };
        if module.decls.len() < self.def_decls {
            self.def_decls = 0;
        }
        let new_start = self.def_decls.min(module.decls.len());
        let prepared = match self.prepare(&mut module, new_start, t.len(), candidate.len(), EVAL_FN) {
            Ok(p) => p,
            Err(e) => return ReplOut::Lines(self.render_errors(&[e])),
        };
        let desugar_errs = frontend::desugar::desugar(&mut module);
        if !desugar_errs.is_empty() {
            return ReplOut::Lines(self.render_errors(&desugar_errs));
        }
        let mut sem_errs = Vec::new();
        for d in frontend::semantic::check(&module) {
            if !d.code.is_warning() {
                sem_errs.push(d);
            }
        }
        if !sem_errs.is_empty() {
            return ReplOut::Lines(self.render_errors(&sem_errs));
        }
        let lowered = match lir::lower::lower(&module) {
            Ok(l) => l,
            Err(e) => return ReplOut::Lines(self.render_errors(&[e])),
        };
        if let Some(e) = lir::verify::verify(&lowered).into_iter().next() {
            return ReplOut::Lines(self.render_errors(&[e]));
        }
        let mut out = match self.sync(&lowered, &prepared.ast_fns, &module) {
            Ok(v) => v,
            Err(e) => return ReplOut::Lines(self.render_errors(&[e])),
        };
        self.commit_defs(t, &prepared);
        self.refresh_records(&module, &prepared.ast_fns);
        if prepared.has_eval {
            match self.run_eval(&lowered) {
                Ok(Some(line)) => out.push(line),
                Ok(None) => {}
                Err(e) => out.extend(self.render_errors(&[e])),
            }
        }
        ReplOut::Lines(out)
    }

    fn commit_defs(&mut self, block: &str, prepared: &Prepared) {
        let part = match prepared.eval_off {
            Some(off) => block.get(..off).unwrap_or("").trim_end(),
            None => block.trim_end(),
        };
        if part.is_empty() {
            return;
        }
        if self.defs.is_empty() {
            self.defs = part.to_string();
        } else {
            self.defs.push('\n');
            self.defs.push_str(part);
        }
        let defs = std::mem::take(&mut self.defs);
        match self.parse_pre_desugar(&defs) {
            Ok((m, root_count)) => {
                let _ = m;
                self.def_decls = root_count;
                self.defs = defs;
            }
            Err(_) => {
                self.defs = defs;
            }
        }
    }

    fn directive(&mut self, t: &str) -> Option<ReplOut> {
        match t {
            ":exit" | ":quit" => Some(ReplOut::Exit),
            ":reset" => {
                self.reset_state();
                Some(ReplOut::Lines(vec!["session reset".to_string()]))
            }
            ":clear" => {
                self.reset_state();
                Some(ReplOut::Lines(vec!["\x1b[2J\x1b[Hsession reset".to_string()]))
            }
            ":help" => Some(ReplOut::Lines(vec![
                ":exit, :quit  exit the repl".to_string(),
                ":reset       drop all definitions".to_string(),
                ":clear       clear screen and reset".to_string(),
                ":type <expr> print the static type (Any when unannotated)".to_string(),
            ])),
            _ => None,
        }
    }

    fn type_of(&mut self, expr: &str) -> Vec<String> {
        let candidate = if self.defs.is_empty() {
            format!("({expr});")
        } else {
            format!("{}\n({expr});", self.defs)
        };
        let mut module = match self.parse_pre_desugar(&candidate) {
            Ok((m, _)) => m,
            Err(errs) => return self.render_errors(&errs),
        };
        let new_start = self.def_decls.min(module.decls.len());
        let block_len = format!("({expr});").len();
        match self.prepare(&mut module, new_start, block_len, candidate.len(), TYPE_FN) {
            Ok(_) => {}
            Err(e) => return self.render_errors(&[e]),
        }
        let desugar_errs = frontend::desugar::desugar(&mut module);
        if !desugar_errs.is_empty() {
            return self.render_errors(&desugar_errs);
        }
        let mut sem_errs = Vec::new();
        for d in frontend::semantic::check(&module) {
            if !d.code.is_warning() {
                sem_errs.push(d);
            }
        }
        if !sem_errs.is_empty() {
            return self.render_errors(&sem_errs);
        }
        let lowered = match lir::lower::lower(&module) {
            Ok(l) => l,
            Err(e) => return self.render_errors(&[e]),
        };
        match lowered.functions.iter().find(|f| f.name == TYPE_FN) {
            Some(f) => vec![lir_type_name(&f.ret)],
            None => vec!["unknown".to_string()],
        }
    }

    fn getter_decl(&self, getter: &str, init: A::Expr) -> A::Spanned<A::Decl> {
        let body = A::Block {
            stmts: vec![spanned(A::Stmt::Return(Some(spanned(init))))],
        };
        spanned(A::Decl::Fn(A::FnDecl {
            access: A::Access::Internal,
            name: getter.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            ret: None,
            throws: false,
            is_unsafe: false,
            is_async: false,
            is_static: false,
            is_test: false,
            is_bench: false,
            body: A::FnBody::Block(body),
            attrs: Vec::new(),
            docs: String::new(),
        }))
    }

    fn eval_decl(&self, body: A::Block, name: &str) -> A::Spanned<A::Decl> {
        spanned(A::Decl::Fn(A::FnDecl {
            access: A::Access::Internal,
            name: name.to_string(),
            type_params: Vec::new(),
            params: Vec::new(),
            ret: None,
            throws: false,
            is_unsafe: false,
            is_async: false,
            is_static: false,
            is_test: false,
            is_bench: false,
            body: A::FnBody::Block(body),
            attrs: Vec::new(),
            docs: String::new(),
        }))
    }

    fn prepare(
        &mut self,
        module: &mut A::Module,
        new_start: usize,
        block_len: usize,
        candidate_len: usize,
        eval_name: &str,
    ) -> Result<Prepared, diagnostics::Diagnostic> {
        let mut eval_taken = false;
        let mut eval_off = None;
        let mut synthesized: BTreeMap<String, A::Spanned<A::Decl>> = BTreeMap::new();
        let mut keep: Vec<A::Spanned<A::Decl>> = Vec::new();
        let mut user_fns: Vec<String> = Vec::new();
        let mut user_classes: Vec<String> = Vec::new();
        let mut snap_fns: Vec<String> = Vec::new();
        let block_off = candidate_len - block_len;
        for (idx, decl) in module.decls.drain(..).enumerate() {
            let is_new = idx >= new_start;
            match decl.node {
                A::Decl::Stmt(s) => {
                    match s.node {
                        A::Stmt::Var { name, value, .. } => {
                            if is_new && self.fns.contains_key(&name) {
                                return Err(err(format!("`{name}` is already a function; rename or :reset")));
                            }
                            let getter = format!("__repl_g_{name}");
                            synthesized.insert(getter.clone(), self.getter_decl(&getter, value.node.clone()));
                            self.globals
                                .entry(name)
                                .or_insert(GlobalInfo { getter, init: None })
                                .init = Some(value.node);
                        }
                        A::Stmt::Assign { target, op, value } => {
                            let tspan = target.span;
                            if let A::Expr::Ident(name) = target.node {
                                let getter = match self.globals.get(&name) {
                                    Some(info) => info.getter.clone(),
                                    None => {
                                        let g = format!("__repl_g_{name}");
                                        match synthesized.get(&g) {
                                            Some(_) => g,
                                            None => {
                                                return Err(err(format!(
                                                    "unknown name `{name}`; bind it with `let` first"
                                                )));
                                            }
                                        }
                                    }
                                };
                                let old_init = match self.globals.get(&name) {
                                    Some(info) => info.init.clone(),
                                    None => None,
                                }
                                .or_else(|| snap_init_from(&synthesized, &getter));
                                let new_init = match op {
                                    A::AssignOp::Eq => value.node,
                                    _ => {
                                        let bin = match op {
                                            A::AssignOp::PlusEq => A::BinOp::Add,
                                            A::AssignOp::MinusEq => A::BinOp::Sub,
                                            A::AssignOp::StarEq => A::BinOp::Mul,
                                            A::AssignOp::SlashEq => A::BinOp::Div,
                                            _ => A::BinOp::Mod,
                                        };
                                        let old = match old_init {
                                            Some(e) => e,
                                            None => {
                                                return Err(err(format!(
                                                    "cannot compound-assign `{name}` before its value is known"
                                                )));
                                            }
                                        };
                                        let snap = format!("{getter}_snap_{}", self.snap_idx);
                                        self.snap_idx += 1;
                                        synthesized.insert(snap.clone(), self.getter_decl(&snap, old));
                                        snap_fns.push(snap.clone());
                                        compound_assign(&snap, tspan, bin, value)
                                    }
                                };
                                synthesized.insert(getter.clone(), self.getter_decl(&getter, new_init.clone()));
                                if let Some(info) = self.globals.get_mut(&name) {
                                    info.init = Some(new_init);
                                } else {
                                    self.globals.insert(name, GlobalInfo { getter, init: Some(new_init) });
                                }
                            } else {
                                return Err(err("only plain variables can be assigned at the top level".to_string()));
                            }
                        }
                        A::Stmt::Expr(e) => {
                            if !is_new {
                                continue;
                            }
                            if eval_taken {
                                return Err(err("only the trailing expression evaluates; split inputs across lines".to_string()));
                            }
                            eval_taken = true;
                            eval_off = e.span.start.checked_sub(block_off as u32).map(|o| o as usize);
                            let body = A::Block {
                                stmts: vec![spanned(A::Stmt::Return(Some(spanned(e.node))))],
                            };
                            keep.push(self.eval_decl(body, eval_name));
                        }
                        other => {
                            let kind = stmt_kind_name(&other);
                            return Err(err(format!(
                                "`{kind}` is not allowed at the top level; use let, assignment, or an expression"
                            )));
                        }
                    }
                }
                A::Decl::Fn(f) => {
                    if is_new && self.globals.contains_key(&f.name) {
                        return Err(err(format!("`{}` is already a variable; rename or :reset", f.name)));
                    }
                    if is_new {
                        user_fns.push(f.name.clone());
                    }
                    keep.push(spanned(A::Decl::Fn(f)));
                }
                other => {
                    if is_new {
                        match &other {
                            A::Decl::Class { name, .. } | A::Decl::Struct { name, .. } => {
                                user_classes.push(name.clone());
                            }
                            _ => {}
                        }
                    }
                    keep.push(spanned(other));
                }
            }
        }
        let mut ast_fns = user_fns;
        ast_fns.extend(snap_fns);
        for (_, d) in &synthesized {
            if let A::Decl::Fn(f) = &d.node {
                if !ast_fns.contains(&f.name) {
                    ast_fns.push(f.name.clone());
                }
            }
        }
        for (_, d) in synthesized {
            keep.push(d);
        }
        module.decls = keep;
        self.rewrite_all(module);
        for c in user_classes {
            ast_fns.push(format!("{c}.*"));
        }
        Ok(Prepared { has_eval: eval_taken, eval_off, ast_fns })
    }

    fn rewrite_all(&mut self, module: &mut A::Module) {
        let mut rw = Rewriter { globals: &self.globals, bound: vec![BTreeSet::new()] };
        for decl in &mut module.decls {
            match &mut decl.node {
                A::Decl::Fn(f) => {
                    rw.rewrite_fn_body(&f.params, &mut f.body);
                }
                A::Decl::Class { members, .. }
                | A::Decl::Struct { members, .. }
                | A::Decl::Trait { members, .. }
                | A::Decl::Interface { members, .. } => {
                    for m in members {
                        if let A::ClassMember::Method(f) = &mut m.node {
                            rw.rewrite_fn_body(&f.params, &mut f.body);
                        }
                    }
                }
                _ => {}
            }
        }
    }

    fn resolve_lir(lowered: &lir::instr::Module, ast: &str) -> Option<String> {
        if ast.ends_with(".*") {
            return None;
        }
        if lowered.functions.iter().any(|f| f.name == ast) {
            return Some(ast.to_string());
        }
        let suffix = format!(".{ast}");
        let mut hit = None;
        for f in &lowered.functions {
            if f.name.ends_with(&suffix) {
                if hit.is_some() {
                    return None;
                }
                hit = Some(f.name.clone());
            }
        }
        hit
    }

    fn sync(
        &mut self,
        lowered: &lir::instr::Module,
        ast_fns: &[String],
        module: &A::Module,
    ) -> Result<Vec<String>, diagnostics::Diagnostic> {
        let mut lines = Vec::new();
        if self.jit.is_none() {
            let jit = Jit::compile_hot_reserve(lowered, SPARE_SLOTS).map_err(|e| e)?;
            self.jit = Some(jit);
        }
        let jit = match self.jit.as_mut() {
            Some(j) => j,
            None => return Err(err("no jit session".to_string())),
        };
        for ast in ast_fns {
            if ast.ends_with(".*") {
                continue;
            }
            let lir = match Self::resolve_lir(lowered, ast) {
                Some(n) => n,
                None => return Err(err(format!("cannot map `{ast}` to compiled code; :reset and retry"))),
            };
            let (sig, body) = fn_keys(module, ast);
            match jit.slot_of(&lir) {
                None => {
                    jit.define_new_function(&lir, lowered).map_err(|e| e)?;
                    lines.push(describe_define(ast));
                }
                Some(slot) => {
                    let changed = match self.fns.get(ast) {
                        Some(rec) => rec.body != body,
                        None => true,
                    };
                    if changed {
                        match jit.hot_swap_function(slot, lowered) {
                            Ok(()) => lines.push(describe_update(ast)),
                            Err(e) => {
                                return Err(err(format!(
                                    "cannot redefine `{ast}` ({e:?}); :reset to change its signature"
                                )));
                            }
                        }
                    }
                    let _ = sig;
                }
            }
        }
        Ok(lines)
    }

    fn refresh_records(&mut self, module: &A::Module, ast_fns: &[String]) {
        for ast in ast_fns {
            if ast.ends_with(".*") {
                for decl in &module.decls {
                    let (name, members) = match &decl.node {
                        A::Decl::Class { name, members, .. } | A::Decl::Struct { name, members, .. } => (name, members),
                        _ => continue,
                    };
                    if format!("{name}.*") != *ast {
                        continue;
                    }
                    for m in members {
                        if let A::ClassMember::Method(f) = &m.node {
                            let key = format!("{name}.{}", f.name);
                            self.fns.insert(
                                key,
                                FnRecord { sig: sig_key_of(f), body: format!("{:?}", f.body) },
                            );
                        }
                    }
                }
                continue;
            }
            for decl in &module.decls {
                if let A::Decl::Fn(f) = &decl.node {
                    if f.name == *ast {
                        let rec = self
                            .fns
                            .entry(ast.clone())
                            .or_insert(FnRecord { sig: String::new(), body: String::new() });
                        rec.sig = sig_key_of(f);
                        rec.body = format!("{:?}", f.body);
                    }
                }
            }
        }
    }

    fn run_eval(&mut self, lowered: &lir::instr::Module) -> Result<Option<String>, diagnostics::Diagnostic> {
        let jit = self.jit.as_mut().ok_or_else(|| err("no jit session".to_string()))?;
        match jit.slot_of(EVAL_FN) {
            None => {
                jit.define_new_function(EVAL_FN, lowered).map_err(|e| e)?;
            }
            Some(slot) => {
                jit.hot_swap_function(slot, lowered).map_err(|e| e)?;
            }
        }
        let ret = lowered
            .functions
            .iter()
            .find(|f| f.name == EVAL_FN)
            .map(|f| f.ret.clone())
            .unwrap_or(lir::instr::LirType::Void);
        let bits = jit.call(EVAL_FN, &[]).map_err(|e| e)? as u64;
        Ok(format_bits(&ret, bits))
    }
}

fn describe_define(ast: &str) -> String {
    match ast.strip_prefix("__repl_g_") {
        Some(v) if !v.contains("_snap_") => format!("defined variable {v}"),
        _ => format!("defined function {ast}"),
    }
}

fn describe_update(ast: &str) -> String {
    match ast.strip_prefix("__repl_g_") {
        Some(v) if !v.contains("_snap_") => format!("updated variable {v}"),
        _ => format!("updated function {ast}"),
    }
}

fn fn_keys(module: &A::Module, ast: &str) -> (String, String) {
    for decl in &module.decls {
        if let A::Decl::Fn(f) = &decl.node {
            if f.name == ast {
                return (sig_key_of(f), format!("{:?}", f.body));
            }
        }
    }
    (String::new(), String::new())
}

fn format_bits(ret: &lir::instr::LirType, bits: u64) -> Option<String> {
    match ret {
        lir::instr::LirType::I64 => Some((bits as i64).to_string()),
        lir::instr::LirType::F64(_) => Some(format!("{}", f64::from_bits(bits))),
        lir::instr::LirType::Bool => Some(if bits == 0 { "false".to_string() } else { "true".to_string() }),
        lir::instr::LirType::Str => {
            let s = unsafe { runtime::native::native_str(bits as *const u8) };
            Some(format!("{s:?}"))
        }
        lir::instr::LirType::Any => format_any(bits),
        lir::instr::LirType::Null | lir::instr::LirType::Void => None,
        lir::instr::LirType::Array(_) => Some("<array>".to_string()),
        lir::instr::LirType::Obj(n) => Some(format!("<{n}>")),
        lir::instr::LirType::Closure => Some("<closure>".to_string()),
        lir::instr::LirType::Tuple(_) => Some("<tuple>".to_string()),
        lir::instr::LirType::Enum(_) => Some("<enum>".to_string()),
        _ => Some("<value>".to_string()),
    }
}

fn format_any(bits: u64) -> Option<String> {
    unsafe {
        match runtime::native::rnx_any_tag(bits) as u32 {
            runtime::native::TAG_INT => Some((runtime::native::rnx_any_unbox(bits) as i64).to_string()),
            runtime::native::TAG_BOOL => Some(if runtime::native::rnx_any_unbox(bits) == 0 {
                "false".to_string()
            } else {
                "true".to_string()
            }),
            runtime::native::TAG_FLOAT => Some(format!("{}", f64::from_bits(runtime::native::rnx_any_unbox(bits)))),
            runtime::native::TAG_STR => {
                let s = runtime::native::native_str(runtime::native::rnx_any_unbox(bits) as *const u8);
                Some(format!("{s:?}"))
            }
            runtime::native::TAG_NULL => None,
            _ => Some("<value>".to_string()),
        }
    }
}

fn snap_init_from(synthesized: &BTreeMap<String, A::Spanned<A::Decl>>, getter: &str) -> Option<A::Expr> {
    match synthesized.get(getter)?.node {
        A::Decl::Fn(ref f) => match &f.body {
            A::FnBody::Block(b) => match b.stmts.first().map(|s| &s.node) {
                Some(A::Stmt::Return(Some(e))) => Some(e.node.clone()),
                _ => None,
            },
            A::FnBody::Expr(e) => Some(e.node.clone()),
        },
        _ => None,
    }
}

fn compound_assign(getter: &str, tspan: Span, op: A::BinOp, value: A::Spanned<A::Expr>) -> A::Expr {
    let vspan = value.span;
    A::Expr::Binary {
        op,
        lhs: Box::new(A::Spanned {
            node: A::Expr::Call {
                callee: Box::new(A::Spanned { node: A::Expr::Ident(getter.to_string()), span: tspan }),
                type_args: Vec::new(),
                args: Vec::new(),
                trailing: None,
            },
            span: tspan,
        }),
        rhs: Box::new(A::Spanned { node: value.node, span: vspan }),
    }
}

fn stmt_kind_name(s: &A::Stmt) -> &'static str {
    match s {
        A::Stmt::Var { .. } => "let",
        A::Stmt::Assign { .. } => "assignment",
        A::Stmt::Expr(_) => "expression",
        A::Stmt::If { .. } => "if",
        A::Stmt::For { .. } => "for",
        A::Stmt::While { .. } => "while",
        A::Stmt::DoWhile { .. } => "do-while",
        A::Stmt::Switch { .. } => "switch",
        A::Stmt::Return(_) => "return",
        _ => "statement",
    }
}

fn lir_type_name(t: &lir::instr::LirType) -> String {
    match t {
        lir::instr::LirType::I64 => "Int".to_string(),
        lir::instr::LirType::F64(_) => "Float".to_string(),
        lir::instr::LirType::Bool => "Bool".to_string(),
        lir::instr::LirType::Str => "String".to_string(),
        lir::instr::LirType::Null | lir::instr::LirType::Void => "void".to_string(),
        lir::instr::LirType::Any => "Any".to_string(),
        other => format!("{other:?}"),
    }
}

impl Default for ReplSession {
    fn default() -> Self {
        Self::new()
    }
}

pub fn run_interactive() {
    println!("Rasmalai v{} ({}, cranelift-jit)", env!("CARGO_PKG_VERSION"), std::env::consts::ARCH);
    println!("Type \":exit\" to quit, \":help\" for directives.");
    let mut editor = match rustyline::DefaultEditor::new() {
        Ok(e) => e,
        Err(e) => {
            eprintln!("rnx: line editor unavailable: {e}");
            std::process::exit(1);
        }
    };
    let mut session = ReplSession::new();
    let mut pending = String::new();
    loop {
        let prompt = if pending.is_empty() { "rnx> " } else { "... " };
        match editor.readline(prompt) {
            Ok(line) => {
                let _ = editor.add_history_entry(line.as_str());
                if !pending.is_empty() {
                    pending.push('\n');
                }
                pending.push_str(&line);
                if needs_more(&pending) {
                    continue;
                }
                let block = std::mem::take(&mut pending);
                match session.eval(&block) {
                    ReplOut::Exit => break,
                    ReplOut::Lines(lines) => {
                        for l in lines {
                            println!("{l}");
                        }
                    }
                }
            }
            Err(rustyline::error::ReadlineError::Interrupted) => {
                pending.clear();
                println!();
            }
            Err(rustyline::error::ReadlineError::Eof) => break,
            Err(e) => {
                eprintln!("rnx: input error: {e}");
                break;
            }
        }
    }
}
