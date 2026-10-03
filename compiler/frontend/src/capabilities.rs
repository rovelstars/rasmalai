use crate::ast::{
    Access, BinOp, Block, CallArg, ClassMember, Decl, Else, Expr, FnBody, FnDecl,
    IfCond, ImportSource, InterpPart, Module, Param, Spanned, Stmt, SwitchExprBody, Type, UnOp,
};
use diagnostics::{Code, Diagnostic, Span};
use std::collections::{BTreeSet, HashMap};
use std::fmt;

pub const DEFAULT_FUEL: u64 = 500_000;
const SNIPPET_MAX: usize = 80;
const SIG_MAX: usize = 96;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum CapabilityTier {
    Pure,
    Delegated,
    Ambient,
    Hazard,
}

impl fmt::Display for CapabilityTier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let s = match self {
            CapabilityTier::Pure => "pure",
            CapabilityTier::Delegated => "delegated",
            CapabilityTier::Ambient => "ambient",
            CapabilityTier::Hazard => "hazard",
        };
        f.write_str(s)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Capability {
    FsRead(String),
    FsWrite(String),
    FsDelegated,
    NetHttp(String),
    NetWs(String),
    NetDelegated,
    SysExec(String),
    EnvRead(String),
    EnvDump,
    TermWrite,
    TermRead,
    TermRaw,
    UnsafeFfi,
    UnsafeRawMemory,
}

impl Capability {
    pub fn tier(&self) -> CapabilityTier {
        match self {
            Capability::FsDelegated | Capability::NetDelegated => CapabilityTier::Delegated,
            Capability::UnsafeFfi | Capability::UnsafeRawMemory | Capability::SysExec(_) | Capability::TermRaw => {
                CapabilityTier::Hazard
            }
            _ => CapabilityTier::Ambient,
        }
    }

    pub fn is_delegated(&self) -> bool {
        matches!(self, Capability::FsDelegated | Capability::NetDelegated)
    }
}

impl fmt::Display for Capability {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Capability::FsRead(p) => write!(f, "fs:read:{p}"),
            Capability::FsWrite(p) => write!(f, "fs:write:{p}"),
            Capability::FsDelegated => f.write_str("fs:delegated"),
            Capability::NetHttp(p) => write!(f, "net:http:{p}"),
            Capability::NetWs(p) => write!(f, "net:ws:{p}"),
            Capability::NetDelegated => f.write_str("net:delegated"),
            Capability::SysExec(b) => write!(f, "sys:exec:{b}"),
            Capability::EnvRead(v) => write!(f, "env:read:{v}"),
            Capability::EnvDump => f.write_str("env:read:*"),
            Capability::TermWrite => f.write_str("term:write"),
            Capability::TermRead => f.write_str("term:read"),
            Capability::TermRaw => f.write_str("term:raw"),
            Capability::UnsafeFfi => f.write_str("unsafe:ffi"),
            Capability::UnsafeRawMemory => f.write_str("unsafe:raw_memory"),
        }
    }
}

impl std::str::FromStr for Capability {
    type Err = String;

    fn from_str(s: &str) -> Result<Self, Self::Err> {
        if s == "fs:delegated" {
            return Ok(Capability::FsDelegated);
        }
        if s == "net:delegated" {
            return Ok(Capability::NetDelegated);
        }
        if s == "unsafe:ffi" {
            return Ok(Capability::UnsafeFfi);
        }
        if s == "unsafe:raw_memory" {
            return Ok(Capability::UnsafeRawMemory);
        }
        if s == "env:dump" || s == "env:read:*" {
            return Ok(Capability::EnvDump);
        }
        if s == "term:write" {
            return Ok(Capability::TermWrite);
        }
        if s == "term:read" {
            return Ok(Capability::TermRead);
        }
        if s == "term:raw" {
            return Ok(Capability::TermRaw);
        }
        let mut parts = s.splitn(3, ':');
        let (domain, action, scope) = match (parts.next(), parts.next(), parts.next()) {
            (Some(d), Some(a), Some(s)) => (d, a, s),
            _ => return Err(format!("bad capability `{s}`")),
        };
        match (domain, action) {
            ("fs", "read") => Ok(Capability::FsRead(scope.to_string())),
            ("fs", "write") => Ok(Capability::FsWrite(scope.to_string())),
            ("net", "http") => Ok(Capability::NetHttp(scope.to_string())),
            ("net", "ws") => Ok(Capability::NetWs(scope.to_string())),
            ("sys", "exec") => Ok(Capability::SysExec(scope.to_string())),
            ("env", "read") => Ok(Capability::EnvRead(scope.to_string())),
            _ => Err(format!("unknown capability `{s}`")),
        }
    }
}

#[derive(Debug, Clone)]
pub struct CallTraceNode {
    pub file: String,
    pub line: usize,
    pub col: usize,
    pub symbol: String,
    pub expression_snippet: String,
}

#[derive(Debug, Clone)]
pub struct CallChain {
    pub capability: Capability,
    pub is_delegated: bool,
    pub nodes: Vec<CallTraceNode>,
}

#[derive(Debug, Clone)]
pub struct CapabilityAnalysisReport {
    pub tier: CapabilityTier,
    pub capabilities: Vec<Capability>,
    pub traces: Vec<CallChain>,
}

#[derive(Debug, Clone)]
pub enum SecurityDiagnostic {
    S101 { message: String },
    S102 { message: String },
    S201 { message: String, span: Span },
    S301 { message: String },
    S401 { message: String },
    S501 { fuel_limit: u64 },
}

fn s201(domain: &str, symbol: &str, span: Span) -> SecurityDiagnostic {
    SecurityDiagnostic::S201 {
        message: format!("untrusted path mutation: delegated argument modified before reaching `{domain}` sink `{symbol}`; pass it through untouched or request an ambient grant"),
        span,
    }
}

impl SecurityDiagnostic {
    pub fn code(&self) -> Code {
        match self {
            SecurityDiagnostic::S101 { .. } => Code::S101,
            SecurityDiagnostic::S102 { .. } => Code::S102,
            SecurityDiagnostic::S201 { .. } => Code::S201,
            SecurityDiagnostic::S301 { .. } => Code::S301,
            SecurityDiagnostic::S401 { .. } => Code::S401,
            SecurityDiagnostic::S501 { .. } => Code::S501,
        }
    }

    pub fn message(&self) -> String {
        match self {
            SecurityDiagnostic::S101 { message }
            | SecurityDiagnostic::S102 { message }
            | SecurityDiagnostic::S201 { message, .. }
            | SecurityDiagnostic::S301 { message }
            | SecurityDiagnostic::S401 { message } => message.clone(),
            SecurityDiagnostic::S501 { fuel_limit } => {
                format!("capability analysis fuel exhausted (limit {fuel_limit})")
            }
        }
    }

    pub fn to_diagnostic(&self, span: Span) -> Diagnostic {
        match self {
            SecurityDiagnostic::S201 { span: inner, .. } => {
                Diagnostic::new(self.code(), self.message()).with_span(*inner)
            }
            _ => Diagnostic::new(self.code(), self.message()).with_span(span),
        }
    }
}

impl fmt::Display for SecurityDiagnostic {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}: {}", self.code(), self.message())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Taint {
    Param,
    Literal(String),
    Derived,
    Ambient,
}

fn is_delegated_taint(t: &Taint) -> bool {
    matches!(t, Taint::Param | Taint::Derived)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Sink {
    FsReadArg(usize),
    FsWriteArg(usize),
    FsOpen,
    NetHttpArg(usize),
    NetWsArg(usize),
    SysExecArg(usize),
    EnvReadArg(usize),
    EnvDump,
    TermWrite,
    TermRead,
    TermRaw,
    TermReadWrite,
}

fn sink_for(dotted: &str) -> Option<Sink> {
    Some(match dotted {
        "File.open" => Sink::FsOpen,
        "fs.mmap" => Sink::FsOpen,
        "File.read" | "File.readText" | "File.readBytes" | "File.lines" | "File.exists"
        | "File.seek" | "File.tell" | "File.len" | "Path.exists" | "Path.isFile"
        | "Path.isDir" => Sink::FsReadArg(0),
        "fs.stat" | "fs.exists" | "fs.isFile" | "fs.isDir" | "fs.readDir" | "fs.glob"
        | "fs.readLink" | "fs.readText" | "fs.readBytes" | "fs.readTextAsync"
        | "fs.readBytesAsync" | "fs.statAsync" | "fs.readDirAsync" | "fs.globAsync" => Sink::FsReadArg(0),
        "fs.mkdir" | "fs.mkdirAll" | "fs.remove" | "fs.removeAll" | "fs.copy"
        | "fs.move" | "fs.rename" | "fs.truncate" | "fs.chmod" | "fs.symlink"
        | "fs.fsync" | "fs.writeText" | "fs.writeBytes" | "fs.writeTextAsync"
        | "fs.writeBytesAsync" | "fs.copyAsync" | "fs.moveAsync" | "fs.renameAsync" => Sink::FsWriteArg(0),
        "File.write" | "File.writeText" | "File.writeBytes" | "File.append" | "File.create" | "File.remove"
        | "File.truncate" | "Path.remove" => Sink::FsWriteArg(0),
        "fetch" | "Request" => Sink::NetHttpArg(0),
        "WebSocket" => Sink::NetWsArg(0),
        "Process.spawn" | "Process.run" => Sink::SysExecArg(0),
        "Env.get" | "Env.has" | "env.get" | "env.has" => Sink::EnvReadArg(0),
        "Env.all" | "Env.allEnv" | "env.all" => Sink::EnvDump,
        "io.write" | "io.writeRaw" | "io.writeError" | "io.clear" | "io.stdout" | "io.stderr" | "print" | "__rnx_io_pretty" => Sink::TermWrite,
        "io.read" | "io.readLine" | "io.stdin" | "io.isTTY" | "io.width" | "io.height" | "io.colorProfile" => Sink::TermRead,
        "io.setRawMode" => Sink::TermRaw,
        "File.fromHandle" => Sink::TermReadWrite,
        _ => match dotted.rsplit_once('.').map(|(_, m)| m) {
            Some("writeText") | Some("writeBytes") => Sink::TermWrite,
            Some("readText") | Some("readBytes") => Sink::TermRead,
            _ => return None,
        },
    })
}

fn is_raw_memory_type(ty: &Type) -> bool {
    if ty.path.iter().any(|s| s == "Pointer" || s == "Address") {
        return true;
    }
    ty.args.iter().any(is_raw_memory_type)
        || ty.tuple.iter().any(is_raw_memory_type)
        || ty.fn_sig.as_ref().is_some_and(|s| {
            s.params.iter().any(is_raw_memory_type)
                || s.ret.as_ref().is_some_and(|r| is_raw_memory_type(r))
        })
}

fn line_col(source: &str, offset: u32) -> (usize, usize) {
    let offset = (offset as usize).min(source.len());
    let mut line = 1usize;
    let mut col = 1usize;
    for b in source.as_bytes()[..offset].iter() {
        if *b == b'\n' {
            line += 1;
            col = 1;
        } else {
            col += 1;
        }
    }
    (line, col)
}

fn snippet(source: &str, span: Span) -> String {
    let start = (span.start as usize).min(source.len());
    let end = (span.end as usize).min(source.len()).max(start);
    let raw: String = source[start..end].chars().take(SNIPPET_MAX).collect();
    raw.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn signature_of(source: &str, span: Span) -> String {
    let start = (span.start as usize).min(source.len());
    let end = (span.end as usize).min(source.len()).max(start);
    let text = &source[start..end];
    let open = match text.find('(') {
        Some(i) => i,
        None => {
            let first = text.lines().next().unwrap_or(text).trim();
            return cap_sig(first);
        }
    };
    let bytes = text.as_bytes();
    let mut depth = 0usize;
    let mut close = None;
    let mut i = open;
    let mut in_str = false;
    while i < bytes.len() {
        let b = bytes[i];
        if in_str {
            if b == b'\\' {
                i += 1;
            } else if b == b'"' {
                in_str = false;
            }
        } else if b == b'"' {
            in_str = true;
        } else if b == b'(' {
            depth += 1;
        } else if b == b')' {
            depth -= 1;
            if depth == 0 {
                close = Some(i);
                break;
            }
        }
        i += 1;
    }
    let mut sig_end = close.map(|c| c + 1).unwrap_or(bytes.len());
    let mut angle = 0usize;
    let mut square = 0usize;
    let mut paren = 0usize;
    let mut j = sig_end;
    let mut instr = false;
    while j < bytes.len() {
        let b = bytes[j];
        if instr {
            if b == b'\\' {
                j += 1;
            } else if b == b'"' {
                instr = false;
            }
        } else if b == b'"' {
            instr = true;
        } else if b == b'<' {
            angle += 1;
        } else if b == b'>' {
            angle = angle.saturating_sub(1);
        } else if b == b'[' {
            square += 1;
        } else if b == b']' {
            square = square.saturating_sub(1);
        } else if b == b'(' {
            paren += 1;
        } else if b == b')' {
            paren = paren.saturating_sub(1);
        } else if b == b'{' && angle == 0 && square == 0 && paren == 0 {
            sig_end = j;
            break;
        }
        j += 1;
    }
    cap_sig(text[..sig_end].trim_end().strip_suffix('{').unwrap_or(text[..sig_end].trim_end()).trim())
}

fn cap_sig(sig: &str) -> String {
    let flat = sig.split_whitespace().collect::<Vec<_>>().join(" ");
    if flat.chars().count() > SIG_MAX {
        let capped: String = flat.chars().take(SIG_MAX - 3).collect();
        format!("{capped}...")
    } else {
        flat
    }
}

fn url_prefix(literal: &str) -> String {
    let no_frag = literal.split('#').next().unwrap_or(literal);
    let no_query = no_frag.split('?').next().unwrap_or(no_frag);
    let (origin, path) = match no_query.split_once("://") {
        Some((scheme, rest)) => {
            let lower_scheme = scheme.to_ascii_lowercase();
            let (host, path) = match rest.split_once('/') {
                Some((host, path)) => (host.to_ascii_lowercase(), format!("/{path}")),
                None => (rest.to_ascii_lowercase(), String::new()),
            };
            (format!("{lower_scheme}://{host}"), path)
        }
        None => (no_query.to_string(), String::new()),
    };
    let mut segs: Vec<&str> = path.split('/').filter(|s| !s.is_empty()).collect();
    segs.pop();
    if segs.is_empty() {
        format!("{origin}/*")
    } else {
        format!("{origin}/{}/{}", segs.join("/"), "*")
    }
}

fn fs_pattern(literal: &str) -> String {
    match literal.rsplit_once('/') {
        Some((parent, _)) if !parent.is_empty() => format!("{parent}/**"),
        _ => "/**".to_string(),
    }
}

fn binary_name(literal: &str) -> String {
    literal
        .rsplit(['/', '\\'])
        .next()
        .unwrap_or(literal)
        .to_string()
}

struct FnInfo<'a> {
    name: String,
    params: Vec<String>,
    is_entry: bool,
    decl_span: Span,
    body: &'a FnBody,
}

struct Analyzer<'a> {
    source: &'a str,
    file: String,
    fns: HashMap<String, FnInfo<'a>>,
    order: Vec<String>,
    caps: BTreeSet<Capability>,
    traces: Vec<CallChain>,
    seen_trace: BTreeSet<(String, u32)>,
    fuel: u64,
    fuel_limit: u64,
}

impl<'a> Analyzer<'a> {
    fn burn(&mut self) -> Result<(), SecurityDiagnostic> {
        if self.fuel == 0 {
            return Err(SecurityDiagnostic::S501 {
                fuel_limit: self.fuel_limit,
            });
        }
        self.fuel -= 1;
        Ok(())
    }

    fn node(&self, symbol: &str, span: Span) -> CallTraceNode {
        let (line, col) = line_col(self.source, span.start);
        CallTraceNode {
            file: self.file.clone(),
            line,
            col,
            symbol: symbol.to_string(),
            expression_snippet: snippet(self.source, span),
        }
    }

    fn origin_node(&self, symbol: &str, span: Span) -> CallTraceNode {
        let (line, col) = line_col(self.source, span.start);
        CallTraceNode {
            file: self.file.clone(),
            line,
            col,
            symbol: symbol.to_string(),
            expression_snippet: signature_of(self.source, span),
        }
    }

    fn record(
        &mut self,
        capability: Capability,
        is_delegated: bool,
        nodes: Vec<CallTraceNode>,
        span: Span,
    ) {
        let key = (capability.to_string(), span.start);
        if !self.seen_trace.insert(key) {
            return;
        }
        self.caps.insert(capability.clone());
        self.traces.push(CallChain {
            capability,
            is_delegated,
            nodes,
        });
    }
}

fn dotted_name(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(n) => Some(n.clone()),
        Expr::This => Some("this".to_string()),
        Expr::Super => Some("super".to_string()),
        Expr::Member { base, field } => {
            let b = dotted_name(&base.node)?;
            Some(format!("{b}.{field}"))
        }
        _ => None,
    }
}

fn mode_tail(expr: &Expr) -> Option<String> {
    match expr {
        Expr::Ident(n) => n.rsplit('.').next().map(|s| s.to_string()),
        Expr::Member { field, .. } => Some(field.clone()),
        _ => None,
    }
}

pub fn analyze(
    module: &Module,
    source: &str,
    file: &str,
) -> Result<CapabilityAnalysisReport, SecurityDiagnostic> {
    analyze_with_fuel(module, source, file, DEFAULT_FUEL)
}

pub fn analyze_with_fuel(
    module: &Module,
    source: &str,
    file: &str,
    fuel: u64,
) -> Result<CapabilityAnalysisReport, SecurityDiagnostic> {
    let mut an = Analyzer {
        source,
        file: file.to_string(),
        fns: HashMap::new(),
        order: Vec::new(),
        caps: BTreeSet::new(),
        traces: Vec::new(),
        seen_trace: BTreeSet::new(),
        fuel,
        fuel_limit: fuel,
    };
    collect_fns(module, &mut an)?;
    let entries: Vec<String> = an
        .order
        .iter()
        .filter(|n| an.fns.get(*n).is_some_and(|f| f.is_entry))
        .cloned()
        .collect();
    for entry in entries {
        let (origin, nparams) = {
            let f = &an.fns[&entry];
            (an.origin_node(&entry, f.decl_span), f.params.len())
        };
        let mut stack = vec![entry.clone()];
        enter_fn(
            &mut an,
            &entry,
            vec![Taint::Param; nparams],
            vec![origin],
            &mut stack,
        )?;
    }
    let mut top_env: Vec<(String, Taint)> = Vec::new();
    let mut top_stack = vec!["<top-level>".to_string()];
    for decl in &module.decls {
        if let Decl::Stmt(s) = &decl.node {
            let origin = an.origin_node("<top-level>", decl.span);
            walk_stmt(&mut an, s, &[], &mut top_env, &[origin], &mut top_stack)?;
        }
    }
    let tier = an
        .caps
        .iter()
        .map(|c| c.tier())
        .max()
        .unwrap_or(CapabilityTier::Pure);
    Ok(CapabilityAnalysisReport {
        tier,
        capabilities: an.caps.into_iter().collect(),
        traces: an.traces,
    })
}

fn collect_fns<'a>(module: &'a Module, an: &mut Analyzer<'a>) -> Result<(), SecurityDiagnostic> {
    let decls: &'a [Spanned<Decl>] = &module.decls;
    for decl in decls {
        an.burn()?;
        match &decl.node {
            Decl::Fn(f) => {
                register_fn(an, f, f.name.clone(), decl.span)?;
            }
            Decl::Class { name, members, .. } => {
                collect_methods(an, name, members)?;
            }
            Decl::Extension { target, members, .. } => {
                let owner = target.path.last().cloned().unwrap_or_default();
                collect_methods(an, &owner, members)?;
            }
            Decl::Import(d) => {
                if let ImportSource::Native(lib) = &d.source {
                    let node = an.node(&format!("native:{lib}"), decl.span);
                    an.record(Capability::UnsafeFfi, false, vec![node], decl.span);
                }
            }
            Decl::ExportFrom(d) => {
                if let ImportSource::Native(lib) = &d.source {
                    let node = an.node(&format!("native:{lib}"), decl.span);
                    an.record(Capability::UnsafeFfi, false, vec![node], decl.span);
                }
            }
            Decl::Const { value, .. } => {
                let mut env = Vec::new();
                let mut stack = vec![String::new()];
                walk_expr(an, value, &[], &mut env, &[], &mut stack)?;
            }
            _ => {}
        }
    }
    Ok(())
}

fn register_fn<'a>(
    an: &mut Analyzer<'a>,
    f: &'a FnDecl,
    full: String,
    span: Span,
) -> Result<(), SecurityDiagnostic> {
    let entry = matches!(f.access, Access::Internal | Access::Export);
    if f.is_unsafe {
        let node = an.node(&full, span);
        an.record(Capability::UnsafeRawMemory, false, vec![node], span);
    }
    check_params(an, &full, &f.params)?;
    an.order.push(full.clone());
    an.fns.insert(
        full.clone(),
        FnInfo {
            name: full,
            params: f.params.iter().map(|p| p.name.clone()).collect(),
            is_entry: entry,
            decl_span: span,
            body: &f.body,
        },
    );
    Ok(())
}

fn collect_methods<'a>(
    an: &mut Analyzer<'a>,
    owner: &str,
    members: &'a [Spanned<ClassMember>],
) -> Result<(), SecurityDiagnostic> {
    for m in members {
        an.burn()?;
        if let ClassMember::Method(f) = &m.node {
            register_fn(an, f, format!("{owner}.{}", f.name), m.span)?;
        }
    }
    Ok(())
}

fn check_params(
    an: &mut Analyzer,
    owner: &str,
    params: &[Param],
) -> Result<(), SecurityDiagnostic> {
    for p in params {
        an.burn()?;
        if let Some(ty) = &p.ty
            && is_raw_memory_type(ty)
        {
            let node = an.node(owner, p.span);
            an.record(Capability::UnsafeRawMemory, false, vec![node], p.span);
        }
    }
    Ok(())
}

fn resolve_local<'a>(an: &'a Analyzer<'a>, dotted: &str) -> Option<&'a FnInfo<'a>> {
    if let Some(f) = an.fns.get(dotted) {
        return Some(f);
    }
    if !dotted.contains('.') {
        return None;
    }
    let tail = dotted.rsplit('.').next().unwrap_or(dotted);
    let mut hit: Option<&'a FnInfo<'a>> = None;
    for (name, f) in &an.fns {
        if name == tail || name.ends_with(&format!(".{tail}")) {
            if hit.is_some() {
                return None;
            }
            hit = Some(f);
        }
    }
    hit
}

fn taint_of(
    an: &mut Analyzer,
    expr: &Spanned<Expr>,
    env: &[(String, Taint)],
    params: &[String],
) -> Result<Taint, SecurityDiagnostic> {
    an.burn()?;
    Ok(match &expr.node {
        Expr::Ident(n) => {
            if let Some((_, t)) = env.iter().rev().find(|(k, _)| k == n) {
                t.clone()
            } else if params.contains(n) {
                Taint::Param
            } else {
                Taint::Ambient
            }
        }
        Expr::Interp(parts) => {
            let mut out = String::new();
            let mut derived = false;
            for p in parts {
                match p {
                    InterpPart::Text(t) => out.push_str(t),
                    InterpPart::Expr(e) => match taint_of(an, e, env, params)? {
                        Taint::Literal(s) => out.push_str(&s),
                        Taint::Param | Taint::Derived => derived = true,
                        Taint::Ambient => return Ok(Taint::Ambient),
                    },
                }
            }
            if derived {
                return Ok(Taint::Derived);
            }
            Taint::Literal(out)
        }
        Expr::Binary { op, lhs, rhs } => {
            if matches!(op, BinOp::Add) {
                match (
                    taint_of(an, lhs, env, params)?,
                    taint_of(an, rhs, env, params)?,
                ) {
                    (Taint::Literal(a), Taint::Literal(b)) => Taint::Literal(a + &b),
                    (a, b) if is_delegated_taint(&a) || is_delegated_taint(&b) => {
                        Taint::Derived
                    }
                    _ => Taint::Ambient,
                }
            } else {
                Taint::Ambient
            }
        }
        _ => Taint::Ambient,
    })
}

fn arg_taints(
    an: &mut Analyzer,
    args: &[CallArg],
    env: &[(String, Taint)],
    params: &[String],
) -> Result<Vec<Taint>, SecurityDiagnostic> {
    args.iter()
        .map(|a| taint_of(an, &a.value, env, params))
        .collect::<Result<Vec<_>, _>>()
}

fn classify_sink(
    an: &mut Analyzer,
    sink: Sink,
    call_span: Span,
    args: &[CallArg],
    taints: &[Taint],
    nodes: &[CallTraceNode],
    symbol: &str,
) -> Result<(), SecurityDiagnostic> {
    an.burn()?;
    let lit = |i: usize| match taints.get(i) {
        Some(Taint::Literal(s)) => Some(s.clone()),
        _ => None,
    };
    let delegated = |i: usize| matches!(taints.get(i), Some(Taint::Param));
    let mutated = |i: usize| matches!(taints.get(i), Some(Taint::Derived));
    let mut chain = |cap: Capability, is_delegated: bool| {
        let mut full = nodes.to_vec();
        full.push(an.node(symbol, call_span));
        an.record(cap, is_delegated, full, call_span);
    };
    match sink {
        Sink::FsReadArg(i) => {
            if delegated(i) {
                chain(Capability::FsDelegated, true);
            } else if mutated(i) {
                return Err(s201("fs:read", symbol, call_span));
            } else if let Some(s) = lit(i) {
                chain(Capability::FsRead(fs_pattern(&s)), false);
            } else {
                chain(Capability::FsRead("*".to_string()), false);
            }
        }
        Sink::FsWriteArg(i) => {
            if delegated(i) {
                chain(Capability::FsDelegated, true);
            } else if mutated(i) {
                return Err(s201("fs:write", symbol, call_span));
            } else if let Some(s) = lit(i) {
                chain(Capability::FsWrite(fs_pattern(&s)), false);
            } else {
                chain(Capability::FsWrite("*".to_string()), false);
            }
        }
        Sink::FsOpen => {
            let mode = args.get(1).and_then(|a| mode_tail(&a.value.node));
            let read = mode.as_deref() == Some("Read");
            let write = matches!(mode.as_deref(), Some("Write") | Some("Append"));
            let known = read || write;
            if delegated(0) {
                chain(Capability::FsDelegated, true);
            } else if mutated(0) {
                return Err(s201("fs", symbol, call_span));
            } else if let Some(s) = lit(0) {
                let pat = fs_pattern(&s);
                if read || !known {
                    chain(Capability::FsRead(pat.clone()), false);
                }
                if write || !known {
                    chain(Capability::FsWrite(pat), false);
                }
            } else {
                chain(Capability::FsRead("*".to_string()), false);
                chain(Capability::FsWrite("*".to_string()), false);
            }
        }
        Sink::NetHttpArg(i) => {
            if delegated(i) {
                chain(Capability::NetDelegated, true);
            } else if mutated(i) {
                return Err(s201("net:http", symbol, call_span));
            } else if let Some(s) = lit(i) {
                chain(Capability::NetHttp(url_prefix(&s)), false);
            } else {
                chain(Capability::NetHttp("*".to_string()), false);
            }
        }
        Sink::NetWsArg(i) => {
            if delegated(i) {
                chain(Capability::NetDelegated, true);
            } else if mutated(i) {
                return Err(s201("net:ws", symbol, call_span));
            } else if let Some(s) = lit(i) {
                chain(Capability::NetWs(url_prefix(&s)), false);
            } else {
                chain(Capability::NetWs("*".to_string()), false);
            }
        }
        Sink::SysExecArg(i) => {
            if mutated(i) {
                return Err(s201("sys:exec", symbol, call_span));
            } else if let Some(s) = lit(i) {
                chain(Capability::SysExec(binary_name(&s)), false);
            } else {
                chain(Capability::SysExec("*".to_string()), false);
            }
        }
        Sink::EnvReadArg(i) => {
            if mutated(i) {
                return Err(s201("env:read", symbol, call_span));
            } else if let Some(s) = lit(i) {
                chain(Capability::EnvRead(s), false);
            } else {
                chain(Capability::EnvDump, false);
            }
        }
        Sink::EnvDump => {
            chain(Capability::EnvDump, false);
        }
        Sink::TermWrite => {
            chain(Capability::TermWrite, false);
        }
        Sink::TermRead => {
            chain(Capability::TermRead, false);
        }
        Sink::TermRaw => {
            chain(Capability::TermRaw, false);
        }
        Sink::TermReadWrite => {
            chain(Capability::TermRead, false);
            chain(Capability::TermWrite, false);
        }
    }
    Ok(())
}

fn enter_fn(
    an: &mut Analyzer,
    name: &str,
    arg_taints: Vec<Taint>,
    nodes: Vec<CallTraceNode>,
    stack: &mut Vec<String>,
) -> Result<(), SecurityDiagnostic> {
    an.burn()?;
    let (params, body) = {
        let f = an.fns.get(name).expect("resolved");
        (f.params.clone(), f.body)
    };
    let mut env: Vec<(String, Taint)> = params
        .iter()
        .enumerate()
        .map(|(i, p)| {
            (
                p.clone(),
                arg_taints.get(i).cloned().unwrap_or(Taint::Ambient),
            )
        })
        .collect();
    match body {
        FnBody::Block(b) => walk_block(an, b, &params, &mut env, &nodes, stack),
        FnBody::Expr(e) => walk_expr(an, e, &params, &mut env, &nodes, stack),
    }
}

fn walk_block(
    an: &mut Analyzer,
    block: &Block,
    params: &[String],
    env: &mut Vec<(String, Taint)>,
    nodes: &[CallTraceNode],
    stack: &mut Vec<String>,
) -> Result<(), SecurityDiagnostic> {
    for stmt in &block.stmts {
        an.burn()?;
        walk_stmt(an, stmt, params, env, nodes, stack)?;
    }
    Ok(())
}

fn bind_let(
    an: &mut Analyzer,
    name: &str,
    value: &Spanned<Expr>,
    env: &mut Vec<(String, Taint)>,
    params: &[String],
) -> Result<(), SecurityDiagnostic> {
    let t = taint_of(an, value, env, params)?;
    if let Some(slot) = env.iter_mut().rev().find(|(k, _)| k == name) {
        slot.1 = t;
    } else {
        env.push((name.to_string(), t));
    }
    Ok(())
}

fn walk_stmt(
    an: &mut Analyzer,
    stmt: &Spanned<Stmt>,
    params: &[String],
    env: &mut Vec<(String, Taint)>,
    nodes: &[CallTraceNode],
    stack: &mut Vec<String>,
) -> Result<(), SecurityDiagnostic> {
    an.burn()?;
    match &stmt.node {
        Stmt::Var { name, value, .. } => {
            walk_expr(an, value, params, env, nodes, stack)?;
            bind_let(an, name, value, env, params)?;
        }
        Stmt::DestructureTuple { value, .. }
        | Stmt::DestructureRecord { value, .. }
        | Stmt::DestructureArray { value, .. } => {
            walk_expr(an, value, params, env, nodes, stack)?;
        }
        Stmt::Assign { target, value, .. } => {
            walk_expr(an, target, params, env, nodes, stack)?;
            walk_expr(an, value, params, env, nodes, stack)?;
            if let Expr::Ident(n) = &target.node {
                let t = match taint_of(an, value, env, params)? {
                    Taint::Param => Taint::Param,
                    Taint::Derived => Taint::Derived,
                    _ => Taint::Ambient,
                };
                if let Some(slot) = env.iter_mut().rev().find(|(k, _)| k == n) {
                    slot.1 = t;
                } else {
                    env.push((n.clone(), t));
                }
            }
        }
        Stmt::Expr(e) => {
            walk_expr(an, e, params, env, nodes, stack)?;
        }
        Stmt::If { cond, then, otherwise } => {
            match cond {
                IfCond::Expr(e) => walk_expr(an, e, params, env, nodes, stack)?,
                IfCond::Let { value, .. } => walk_expr(an, value, params, env, nodes, stack)?,
            }
            walk_block(an, then, params, env, nodes, stack)?;
            if let Some(e) = otherwise {
                match e {
                    Else::Block(b) => walk_block(an, b, params, env, nodes, stack)?,
                    Else::If(s) => walk_stmt(an, s, params, env, nodes, stack)?,
                }
            }
        }
        Stmt::For { iter, body, .. } => {
            walk_expr(an, iter, params, env, nodes, stack)?;
            walk_block(an, body, params, env, nodes, stack)?;
        }
        Stmt::While { cond, body } | Stmt::DoWhile { cond, body } => {
            walk_expr(an, cond, params, env, nodes, stack)?;
            walk_block(an, body, params, env, nodes, stack)?;
        }
        Stmt::Switch {
            scrutinee,
            cases,
            default,
        } => {
            walk_expr(an, scrutinee, params, env, nodes, stack)?;
            for c in cases {
                an.burn()?;
                if let Some(g) = &c.guard {
                    walk_expr(an, g, params, env, nodes, stack)?;
                }
                for s in &c.body {
                    an.burn()?;
                    walk_stmt(an, s, params, env, nodes, stack)?;
                }
            }
            if let Some(d) = default {
                for s in d {
                    an.burn()?;
                    walk_stmt(an, s, params, env, nodes, stack)?;
                }
            }
        }
        Stmt::Return(e) => {
            if let Some(e) = e {
                walk_expr(an, e, params, env, nodes, stack)?;
            }
        }
        Stmt::Assert(e) => {
            walk_expr(an, e, params, env, nodes, stack)?;
        }
        Stmt::Defer(b) => {
            walk_block(an, b, params, env, nodes, stack)?;
        }
        Stmt::Guard { value, otherwise, .. } => {
            walk_expr(an, value, params, env, nodes, stack)?;
            walk_block(an, otherwise, params, env, nodes, stack)?;
        }
        Stmt::Try { body, catch, finally } => {
            walk_block(an, body, params, env, nodes, stack)?;
            if let Some((_, b)) = catch {
                walk_block(an, b, params, env, nodes, stack)?;
            }
            if let Some(b) = finally {
                walk_block(an, b, params, env, nodes, stack)?;
            }
        }
        Stmt::Throw(e) => {
            if let Some(e) = e {
                walk_expr(an, e, params, env, nodes, stack)?;
            }
        }
        Stmt::UnsafeBlock(b) => {
            let node = an.node("unsafe", stmt.span);
            an.record(Capability::UnsafeRawMemory, false, {
                let mut full = nodes.to_vec();
                full.push(node);
                full
            }, stmt.span);
            walk_block(an, b, params, env, nodes, stack)?;
        }
        Stmt::Break | Stmt::Continue | Stmt::Fallthrough | Stmt::Pass | Stmt::Empty => {}
    }
    Ok(())
}

fn walk_expr(
    an: &mut Analyzer,
    expr: &Spanned<Expr>,
    params: &[String],
    env: &mut Vec<(String, Taint)>,
    nodes: &[CallTraceNode],
    stack: &mut Vec<String>,
) -> Result<(), SecurityDiagnostic> {
    an.burn()?;
    match &expr.node {
        Expr::Ident(n) if n == "Pointer" || n == "Address" => {
            let mut full = nodes.to_vec();
            full.push(an.node(n, expr.span));
            an.record(Capability::UnsafeRawMemory, false, full, expr.span);
        }
        Expr::Member { base, .. } => {
            if let Some(b) = dotted_name(&base.node)
                && (b == "Pointer" || b == "Address")
            {
                let mut full = nodes.to_vec();
                full.push(an.node(&b, base.span));
                an.record(Capability::UnsafeRawMemory, false, full, base.span);
            }
            walk_expr(an, base, params, env, nodes, stack)?;
        }
        Expr::Call { callee, args, .. } => {
            walk_expr(an, callee, params, env, nodes, stack)?;
            for a in args {
                an.burn()?;
                walk_expr(an, &a.value, params, env, nodes, stack)?;
            }
            if let Some(dotted) = dotted_name(&callee.node) {
                visit_call(an, &dotted, expr.span, args, params, env, nodes, stack)?;
            }
        }
        Expr::New { args, .. } => {
            for a in args {
                an.burn()?;
                walk_expr(an, &a.value, params, env, nodes, stack)?;
            }
        }
        Expr::OptCall { base, args, .. } => {
            walk_expr(an, base, params, env, nodes, stack)?;
            for a in args {
                an.burn()?;
                walk_expr(an, &a.value, params, env, nodes, stack)?;
            }
        }
        Expr::Interp(parts) => {
            for p in parts {
                an.burn()?;
                if let InterpPart::Expr(e) = p {
                    walk_expr(an, e, params, env, nodes, stack)?;
                }
            }
        }
        Expr::Array(elems) => {
            for e in elems {
                an.burn()?;
                walk_expr(an, &e.expr, params, env, nodes, stack)?;
            }
        }
        Expr::Record(fields) => {
            for e in fields {
                an.burn()?;
                walk_expr(an, e.value(), params, env, nodes, stack)?;
            }
        }
        Expr::MapLiteral(entries) => {
            for e in entries {
                an.burn()?;
                walk_expr(an, e.value(), params, env, nodes, stack)?;
            }
        }
        Expr::Binary { lhs, rhs, .. } => {
            walk_expr(an, lhs, params, env, nodes, stack)?;
            walk_expr(an, rhs, params, env, nodes, stack)?;
        }
        Expr::Unary { op, rhs } => {
            if matches!(op, UnOp::AddrOf) {
                let mut full = nodes.to_vec();
                full.push(an.node("addrof", expr.span));
                an.record(Capability::UnsafeRawMemory, false, full, expr.span);
            }
            walk_expr(an, rhs, params, env, nodes, stack)?;
        }
        Expr::Postfix { expr: inner, .. } => {
            walk_expr(an, inner, params, env, nodes, stack)?;
        }
        Expr::Ternary {
            cond,
            then,
            otherwise,
        } => {
            walk_expr(an, cond, params, env, nodes, stack)?;
            walk_expr(an, then, params, env, nodes, stack)?;
            walk_expr(an, otherwise, params, env, nodes, stack)?;
        }
        Expr::Coalesce { lhs, rhs } => {
            walk_expr(an, lhs, params, env, nodes, stack)?;
            walk_expr(an, rhs, params, env, nodes, stack)?;
        }
        Expr::OptChain { base, .. } => {
            walk_expr(an, base, params, env, nodes, stack)?;
        }
        Expr::Range { lo, hi, .. } => {
            walk_expr(an, lo, params, env, nodes, stack)?;
            walk_expr(an, hi, params, env, nodes, stack)?;
        }
        Expr::Index { base, index } => {
            walk_expr(an, base, params, env, nodes, stack)?;
            walk_expr(an, index, params, env, nodes, stack)?;
        }
        Expr::Cast { expr: inner, .. } | Expr::Is { base: inner, .. } => {
            walk_expr(an, inner, params, env, nodes, stack)?;
        }
        Expr::Macro { args, .. } => {
            for a in args {
                an.burn()?;
                walk_expr(an, a, params, env, nodes, stack)?;
            }
        }
        Expr::Closure {
            params: cparams,
            body,
            ..
        } => {
            let mut cenv: Vec<(String, Taint)> = cparams
                .iter()
                .map(|p| (p.name.clone(), Taint::Ambient))
                .collect();
            let cnames: Vec<String> = cparams.iter().map(|p| p.name.clone()).collect();
            match body {
                FnBody::Block(b) => walk_block(an, b, &cnames, &mut cenv, nodes, stack)?,
                FnBody::Expr(e) => walk_expr(an, e, &cnames, &mut cenv, nodes, stack)?,
            }
        }
        Expr::UnsafeBlock(b) => {
            let node = an.node("unsafe", expr.span);
            an.record(Capability::UnsafeRawMemory, false, {
                let mut full = nodes.to_vec();
                full.push(node);
                full
            }, expr.span);
            walk_block(an, b, params, env, nodes, stack)?;
        }
        Expr::Await(inner) | Expr::Propagate(inner) => {
            walk_expr(an, inner, params, env, nodes, stack)?;
        }
        Expr::Tuple(elems) => {
            for e in elems {
                an.burn()?;
                walk_expr(an, e, params, env, nodes, stack)?;
            }
        }
        Expr::TupleGet { base, .. } => {
            walk_expr(an, base, params, env, nodes, stack)?;
        }
        Expr::Switch {
            scrutinee, cases, ..
        } => {
            walk_expr(an, scrutinee, params, env, nodes, stack)?;
            for c in cases {
                an.burn()?;
                if let Some(g) = &c.guard {
                    walk_expr(an, g, params, env, nodes, stack)?;
                }
                match &c.body {
                    SwitchExprBody::Expr(e) => walk_expr(an, e, params, env, nodes, stack)?,
                    SwitchExprBody::Block(b) => walk_block(an, b, params, env, nodes, stack)?,
                }
            }
        }
        Expr::Ident(_)
        | Expr::This
        | Expr::Super
        | Expr::Bool(_)
        | Expr::Null
        | Expr::Int(_)
        | Expr::Float(_)
        | Expr::ImplicitMember(_) => {}
    }
    Ok(())
}

fn visit_call(
    an: &mut Analyzer,
    dotted: &str,
    call_span: Span,
    args: &[CallArg],
    params: &[String],
    env: &mut Vec<(String, Taint)>,
    nodes: &[CallTraceNode],
    stack: &mut Vec<String>,
) -> Result<(), SecurityDiagnostic> {
    an.burn()?;
    if let Some(sink) = sink_for(dotted) {
        let taints = arg_taints(an, args, env, params)?;
        return classify_sink(an, sink, call_span, args, &taints, nodes, dotted);
    }
    let target = match resolve_local(an, dotted) {
        Some(f) => f.name.clone(),
        None => return Ok(()),
    };
    if stack.contains(&target) {
        return Ok(());
    }
    let taints = arg_taints(an, args, env, params)?;
    let mut full = nodes.to_vec();
    full.push(an.node(&target, call_span));
    stack.push(target.clone());
    let r = enter_fn(an, &target, taints, full, stack);
    stack.pop();
    r
}
