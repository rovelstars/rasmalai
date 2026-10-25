use crate::checksum::Sha256;
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const TITLE: &str = "bindgen.rnx";
pub const KIND: &str = "bindings";
pub const ROLE: &str = "bindgen";

pub fn generator_string() -> String {
    format!("rnx-bindgen {}", env!("CARGO_PKG_VERSION"))
}

#[derive(Debug, Clone)]
pub struct BindgenOptions {
    pub headers: Vec<PathBuf>,
    pub include_dirs: Vec<PathBuf>,
    pub native_lib: String,
    pub pkg_name: String,
    pub target: String,
    pub prefixes: Vec<String>,
    pub skips: Vec<String>,
    pub upstream_name: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ClassTally {
    pub proxyable: usize,
    pub marshalable: usize,
    pub handletable: usize,
    pub not_proxyable: usize,
}

#[derive(Debug, Clone)]
pub struct BindgenOutput {
    pub rnx: String,
    pub oci_json: String,
    pub annotations: BTreeMap<String, String>,
    pub tally: ClassTally,
    pub emitted: usize,
    pub consts: usize,
    pub skipped: Vec<(String, String)>,
    pub warnings: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Class {
    Proxyable,
    Marshalable,
    Handletable,
    NotProxyable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cty {
    Void,
    Bool,
    Byte,
    Int,
    Float,
    Ptr(Box<Cty>),
    Rec(String),
    Union(String),
    EnumTy(String),
    Fun,
    VaList,
    Named(String),
    Unknown(String),
}

#[derive(Debug, Clone)]
struct CFunc {
    name: String,
    ret: Cty,
    params: Vec<(String, Cty)>,
    variadic: bool,
}

#[derive(Debug, Clone)]
struct ConstVal {
    name: String,
    value: ConstLit,
}

#[derive(Debug, Clone)]
enum ConstLit {
    Int(i64),
    Str(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Ctok {
    Ident(String),
    Num(String),
    Str(String),
    Chr(String),
    Punct(String),
}

fn is_ident_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_ident_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn lex_c(text: &str) -> Vec<Ctok> {
    let bytes = text.as_bytes();
    let mut out = Vec::new();
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b.is_ascii_whitespace() {
            i += 1;
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'/' {
            while i < bytes.len() && bytes[i] != b'\n' {
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            continue;
        }
        if b == b'"' {
            let mut s = String::new();
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    s.push(bytes[i] as char);
                    s.push(bytes[i + 1] as char);
                    i += 2;
                } else {
                    s.push(bytes[i] as char);
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            out.push(Ctok::Str(s));
            continue;
        }
        if b == b'\'' {
            let mut s = String::new();
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    s.push(bytes[i] as char);
                    s.push(bytes[i + 1] as char);
                    i += 2;
                } else {
                    s.push(bytes[i] as char);
                    i += 1;
                }
            }
            i = (i + 1).min(bytes.len());
            out.push(Ctok::Chr(s));
            continue;
        }
        if is_ident_start(b) {
            let start = i;
            while i < bytes.len() && is_ident_char(bytes[i]) {
                i += 1;
            }
            out.push(Ctok::Ident(text[start..i].to_string()));
            continue;
        }
        if b.is_ascii_digit()
            || (b == b'.' && i + 1 < bytes.len() && bytes[i + 1].is_ascii_digit())
        {
            let start = i;
            while i < bytes.len()
                && (bytes[i].is_ascii_alphanumeric()
                    || bytes[i] == b'.'
                    || bytes[i] == b'_'
                    || bytes[i] == b'\'')
            {
                i += 1;
            }
            out.push(Ctok::Num(text[start..i].to_string()));
            continue;
        }
        let three = if i + 3 <= bytes.len() { Some(&text[i..i + 3]) } else { None };
        let two = if i + 2 <= bytes.len() { Some(&text[i..i + 2]) } else { None };
        if three == Some("...") || three == Some("<<=") || three == Some(">>=") {
            out.push(Ctok::Punct(three.unwrap().to_string()));
            i += 3;
            continue;
        }
        if matches!(
            two,
            Some("->") | Some("==") | Some("!=") | Some("<=") | Some(">=") | Some("&&")
                | Some("||") | Some("<<") | Some(">>") | Some("+=") | Some("-=")
                | Some("*=") | Some("/=") | Some("%=") | Some("&=") | Some("|=")
                | Some("^=") | Some("##")
        ) {
            out.push(Ctok::Punct(two.unwrap().to_string()));
            i += 2;
            continue;
        }
        out.push(Ctok::Punct((b as char).to_string()));
        i += 1;
    }
    out
}

fn strip_block_comments(text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len());
    let mut i = 0;
    while i < bytes.len() {
        let b = bytes[i];
        if b == b'"' {
            out.push(b as char);
            i += 1;
            while i < bytes.len() && bytes[i] != b'"' {
                if bytes[i] == b'\n' {
                    out.push('\n');
                    i += 1;
                } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i] as char);
                    out.push(bytes[i + 1] as char);
                    i += 2;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            if i < bytes.len() {
                out.push('"');
                i += 1;
            }
            continue;
        }
        if b == b'\'' {
            out.push(b as char);
            i += 1;
            while i < bytes.len() && bytes[i] != b'\'' {
                if bytes[i] == b'\n' {
                    out.push('\n');
                    i += 1;
                } else if bytes[i] == b'\\' && i + 1 < bytes.len() {
                    out.push(bytes[i] as char);
                    out.push(bytes[i + 1] as char);
                    i += 2;
                } else {
                    out.push(bytes[i] as char);
                    i += 1;
                }
            }
            if i < bytes.len() {
                out.push('\'');
                i += 1;
            }
            continue;
        }
        if b == b'/' && i + 1 < bytes.len() && bytes[i + 1] == b'*' {
            i += 2;
            while i + 1 < bytes.len() && !(bytes[i] == b'*' && bytes[i + 1] == b'/') {
                if bytes[i] == b'\n' {
                    out.push('\n');
                }
                i += 1;
            }
            i = (i + 2).min(bytes.len());
            out.push(' ');
            continue;
        }
        out.push(b as char);
        i += 1;
    }
    out
}

fn join_continuations(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\' && chars.peek() == Some(&'\n') {
            chars.next();
            out.push(' ');
        } else {
            out.push(c);
        }
    }
    out
}

struct PpState {
    defines: BTreeMap<String, String>,
    fun_like: BTreeMap<String, (Vec<String>, String)>,
    primary_defines: BTreeSet<String>,
    active: Vec<bool>,
    taken: Vec<bool>,
}

impl PpState {
    fn is_active(&self) -> bool {
        self.active.iter().all(|a| *a)
    }
}

fn parse_num_value(text: &str) -> Option<i64> {
    let t = text.trim().replace('\'', "");
    let t = t.trim_end_matches(['u', 'U', 'l', 'L', 'z', 'Z']);
    if t.is_empty() {
        return None;
    }
    if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
        return i64::from_str_radix(hex, 16).ok();
    }
    if t.len() > 1 && t.starts_with('0') && t.bytes().all(|b| b.is_ascii_digit()) {
        return i64::from_str_radix(t, 8).ok();
    }
    t.parse::<i64>().ok()
}

struct IfEval<'a> {
    toks: Vec<Ctok>,
    pos: usize,
    defines: &'a BTreeMap<String, String>,
    consts: &'a BTreeMap<String, i64>,
}

fn if_eval(
    text: &str,
    defines: &BTreeMap<String, String>,
    consts: &BTreeMap<String, i64>,
) -> i64 {
    let toks: Vec<Ctok> = lex_c(text);
    let mut e = IfEval { toks, pos: 0, defines, consts };
    e.parse_or()
}

impl IfEval<'_> {
    fn peek(&self) -> Option<&Ctok> {
        self.toks.get(self.pos)
    }

    fn next(&mut self) -> Option<Ctok> {
        let t = self.toks.get(self.pos).cloned();
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, p: &str) -> bool {
        if matches!(self.peek(), Some(Ctok::Punct(q)) if q == p) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn parse_or(&mut self) -> i64 {
        let mut v = self.parse_and();
        while self.eat("||") {
            let r = self.parse_and();
            v = i64::from(v != 0 || r != 0);
        }
        v
    }

    fn parse_and(&mut self) -> i64 {
        let mut v = self.parse_bor();
        while self.eat("&&") {
            let r = self.parse_bor();
            v = i64::from(v != 0 && r != 0);
        }
        v
    }

    fn parse_bor(&mut self) -> i64 {
        let mut v = self.parse_bxor();
        while self.eat("|") && !matches!(self.peek(), Some(Ctok::Punct(q)) if q == "|") {
            let r = self.parse_bxor();
            v |= r;
        }
        v
    }

    fn parse_bxor(&mut self) -> i64 {
        let mut v = self.parse_band();
        while self.eat("^") {
            let r = self.parse_band();
            v ^= r;
        }
        v
    }

    fn parse_band(&mut self) -> i64 {
        let mut v = self.parse_eq();
        while self.eat("&") && !matches!(self.peek(), Some(Ctok::Punct(q)) if q == "&") {
            let r = self.parse_eq();
            v &= r;
        }
        v
    }

    fn parse_eq(&mut self) -> i64 {
        let mut v = self.parse_rel();
        loop {
            if self.eat("==") {
                let r = self.parse_rel();
                v = i64::from(v == r);
            } else if self.eat("!=") {
                let r = self.parse_rel();
                v = i64::from(v != r);
            } else {
                break;
            }
        }
        v
    }

    fn parse_rel(&mut self) -> i64 {
        let mut v = self.parse_shift();
        loop {
            if self.eat("<=") {
                let r = self.parse_shift();
                v = i64::from(v <= r);
            } else if self.eat(">=") {
                let r = self.parse_shift();
                v = i64::from(v >= r);
            } else if self.eat("<") {
                let r = self.parse_shift();
                v = i64::from(v < r);
            } else if self.eat(">") {
                let r = self.parse_shift();
                v = i64::from(v > r);
            } else {
                break;
            }
        }
        v
    }

    fn parse_shift(&mut self) -> i64 {
        let mut v = self.parse_add();
        loop {
            if self.eat("<<") {
                let r = self.parse_add();
                v = v.wrapping_shl((r as u32) & 63);
            } else if self.eat(">>") {
                let r = self.parse_add();
                v = v.wrapping_shr((r as u32) & 63);
            } else {
                break;
            }
        }
        v
    }

    fn parse_add(&mut self) -> i64 {
        let mut v = self.parse_mul();
        loop {
            if self.eat("+") {
                let r = self.parse_mul();
                v = v.wrapping_add(r);
            } else if self.eat("-") {
                let r = self.parse_mul();
                v = v.wrapping_sub(r);
            } else {
                break;
            }
        }
        v
    }

    fn parse_mul(&mut self) -> i64 {
        let mut v = self.parse_unary();
        loop {
            if self.eat("*") {
                let r = self.parse_unary();
                v = v.wrapping_mul(r);
            } else if self.eat("/") {
                let r = self.parse_unary();
                v = if r == 0 { 0 } else { v.wrapping_div(r) };
            } else if self.eat("%") {
                let r = self.parse_unary();
                v = if r == 0 { 0 } else { v.wrapping_rem(r) };
            } else {
                break;
            }
        }
        v
    }

    fn parse_unary(&mut self) -> i64 {
        if self.eat("!") {
            return i64::from(self.parse_unary() == 0);
        }
        if self.eat("~") {
            return !self.parse_unary();
        }
        if self.eat("-") {
            return -self.parse_unary();
        }
        if self.eat("+") {
            return self.parse_unary();
        }
        self.parse_primary()
    }

    fn parse_primary(&mut self) -> i64 {
        match self.next() {
            Some(Ctok::Num(n)) => parse_num_value(&n).unwrap_or(0),
            Some(Ctok::Chr(c)) => char_lit_value(&c).unwrap_or(0),
            Some(Ctok::Ident(name)) => {
                if name == "defined" {
                    let in_paren = self.eat("(");
                    let target = match self.next() {
                        Some(Ctok::Ident(id)) => id,
                        _ => String::new(),
                    };
                    if in_paren {
                        self.eat(")");
                    }
                    return i64::from(self.defines.contains_key(&target));
                }
                if let Some(v) = self.consts.get(&name) {
                    return *v;
                }
                if let Some(body) = self.defines.get(&name).cloned() {
                    let body = body.trim().to_string();
                    if body.is_empty() {
                        return 0;
                    }
                    return if_eval(&body, self.defines, self.consts);
                }
                0
            }
            Some(Ctok::Punct(p)) if p == "(" => {
                let v = self.parse_or();
                self.eat(")");
                v
            }
            _ => 0,
        }
    }
}

fn char_lit_value(text: &str) -> Option<i64> {
    let mut chars = text.chars();
    let first = chars.next()?;
    if first == '\\' {
        return match chars.next()? {
            'n' => Some(10),
            't' => Some(9),
            'r' => Some(13),
            '0' => Some(0),
            '\'' => Some(39),
            '\\' => Some(92),
            _ => None,
        };
    }
    if chars.next().is_none() {
        return Some(first as i64);
    }
    None
}

fn const_expr_value(
    text: &str,
    defines: &BTreeMap<String, String>,
    consts: &BTreeMap<String, i64>,
) -> Option<i64> {
    let t = text.trim();
    if t.is_empty() || t.starts_with('"') {
        return None;
    }
    if (t.starts_with('(') && t.ends_with(')'))
        || t.contains(['+', '-', '*', '/', '%', '<', '>', '|', '&', '^', '~', '(', '\''])
    {
        return Some(if_eval(t, defines, consts));
    }
    if let Some(v) = parse_num_value(t) {
        return Some(v);
    }
    if let Some(v) = consts.get(t) {
        return Some(*v);
    }
    if let Some(body) = defines.get(t) {
        let body = body.trim().to_string();
        if body.is_empty() || body == t {
            return None;
        }
        return const_expr_value(&body, defines, consts);
    }
    None
}

fn predefines(target: &str) -> Result<BTreeMap<String, String>, String> {
    let mut out = BTreeMap::new();
    out.insert("__STDC__".to_string(), "1".to_string());
    match target {
        "x86_64-unknown-linux-gnu" => {
            for m in ["__x86_64__", "__linux__", "__unix__", "__LP64__", "__ELF__"] {
                out.insert(m.to_string(), "1".to_string());
            }
        }
        "aarch64-unknown-linux-gnu" => {
            for m in ["__aarch64__", "__linux__", "__unix__", "__LP64__", "__ELF__"] {
                out.insert(m.to_string(), "1".to_string());
            }
        }
        "x86_64-pc-windows-msvc" => {
            for m in ["_WIN32", "_WIN64", "__x86_64__"] {
                out.insert(m.to_string(), "1".to_string());
            }
            out.insert("_MSC_VER".to_string(), "1930".to_string());
        }
        "aarch64-apple-darwin" => {
            for m in ["__aarch64__", "__APPLE__", "__LP64__", "__MACH__"] {
                out.insert(m.to_string(), "1".to_string());
            }
        }
        "wasm32-unknown-unknown" => {
            out.insert("__wasm32__".to_string(), "1".to_string());
        }
        _ => {
            return Err(format!(
                "unsupported bindgen target `{target}` (expected x86_64-unknown-linux-gnu, aarch64-unknown-linux-gnu, x86_64-pc-windows-msvc, aarch64-apple-darwin, or wasm32-unknown-unknown)"
            ));
        }
    }
    Ok(out)
}

struct Pp<'a> {
    include_dirs: &'a [PathBuf],
    primary: bool,
    toks: Vec<(Ctok, bool)>,
    state: PpState,
    consts: BTreeMap<String, i64>,
    visited: BTreeSet<PathBuf>,
}

fn process_file(path: &Path, pp: &mut Pp, depth: usize) -> Result<(), String> {
    if depth > 16 {
        return Err(format!("include nesting too deep at `{}`", path.display()));
    }
    let canon = path
        .canonicalize()
        .map(|p| p.to_string_lossy().into_owned())
        .unwrap_or_else(|_| path.display().to_string());
    if !pp.visited.insert(PathBuf::from(&canon)) {
        return Ok(());
    }
    let text =
        std::fs::read_to_string(path).map_err(|e| format!("cannot read `{}`: {e}", path.display()))?;
    process_text(&text, path, pp, depth)
}

fn process_text(
    text: &str,
    path: &Path,
    pp: &mut Pp,
    depth: usize,
) -> Result<(), String> {
    let joined = join_continuations(&strip_block_comments(text));
    for raw in joined.lines() {
        let line = raw.trim_start();
        if !line.starts_with('#') {
            if pp.state.is_active() {
                let lexed = lex_c(raw);
                let expanded = expand_defines(&lexed, &pp.state.defines);
                let fun_bodies: BTreeMap<String, (Vec<String>, Vec<Ctok>)> = pp
                    .state
                    .fun_like
                    .iter()
                    .map(|(n, (q, b))| (n.clone(), (q.clone(), lex_c(b))))
                    .collect();
                let expanded = expand_fun_like(&expanded, &fun_bodies);
                let primary = pp.primary;
                pp.toks.extend(expanded.into_iter().map(|t| (t, primary)));
            }
            continue;
        }
        let body = line[1..].trim_start();
        let (word, rest) = match body.find(|c: char| c.is_whitespace()) {
            Some(i) => (body[..i].to_string(), body[i..].trim_start().to_string()),
            None => (body.to_string(), String::new()),
        };
        match word.as_str() {
            "define" => {
                if !pp.state.is_active() {
                    continue;
                }
                let mut it = rest.splitn(2, |c: char| c.is_whitespace() || c == '(');
                let name = it.next().unwrap_or("").to_string();
                if name.is_empty() {
                    continue;
                }
                let after = rest[name.len()..].to_string();
                if after.starts_with('(') {
                    let inner = after[1..].to_string();
                    if let Some(end) = inner.find(')') {
                        let params = inner[..end]
                            .split(',')
                            .map(|p| p.trim().to_string())
                            .filter(|p| !p.is_empty())
                            .collect::<Vec<_>>();
                        let body = inner[end + 1..].trim_start().to_string();
                        if !params.iter().any(|p| p == "...") && !body.contains('#') {
                            pp.state.fun_like.insert(name, (params, body));
                        }
                    }
                    continue;
                }
                let value = after.trim_start().to_string();
                if pp.primary {
                    pp.state.primary_defines.insert(name.clone());
                }
                pp.state.defines.insert(name.clone(), value.clone());
                if !value.is_empty()
                    && !value.starts_with('"')
                    && let Some(v) = const_expr_value(&value, &pp.state.defines, &pp.consts)
                {
                    pp.consts.insert(name, v);
                }
            }
            "undef" => {
                if pp.state.is_active() {
                    let name = rest.split_whitespace().next().unwrap_or("").to_string();
                    pp.state.defines.remove(&name);
                    pp.state.primary_defines.remove(&name);
                    pp.consts.remove(&name);
                }
            }
            "ifdef" => {
                let parent = pp.state.is_active();
                let name = rest.split_whitespace().next().unwrap_or("");
                let cond = pp.state.defines.contains_key(name);
                pp.state.active.push(parent && cond);
                pp.state.taken.push(parent && cond);
            }
            "ifndef" => {
                let parent = pp.state.is_active();
                let name = rest.split_whitespace().next().unwrap_or("");
                let cond = !pp.state.defines.contains_key(name);
                pp.state.active.push(parent && cond);
                pp.state.taken.push(parent && cond);
            }
            "if" => {
                let parent = pp.state.is_active();
                let cond = parent && if_eval(&rest, &pp.state.defines, &pp.consts) != 0;
                pp.state.active.push(cond);
                pp.state.taken.push(cond);
            }
            "elif" => {
                if pp.state.active.pop().is_none() {
                    return Err(format!("stray `#elif` in `{}`", path.display()));
                }
                let parent = pp.state.active.iter().all(|a| *a);
                let taken = pp.state.taken.pop().unwrap_or(false);
                let cond =
                    parent && !taken && if_eval(&rest, &pp.state.defines, &pp.consts) != 0;
                pp.state.active.push(cond);
                pp.state.taken.push(taken || cond);
            }
            "else" => {
                if pp.state.active.pop().is_none() {
                    return Err(format!("stray `#else` in `{}`", path.display()));
                }
                let parent = pp.state.active.iter().all(|a| *a);
                let taken = pp.state.taken.pop().unwrap_or(false);
                let cond = parent && !taken;
                pp.state.active.push(cond);
                pp.state.taken.push(true);
            }
            "endif" => {
                if pp.state.active.pop().is_none() {
                    return Err(format!("stray `#endif` in `{}`", path.display()));
                }
                pp.state.taken.pop();
            }
            "include" => {
                if !pp.state.is_active() {
                    continue;
                }
                let arg = rest.trim();
                if let Some(quoted) = arg.strip_prefix('"') {
                    if let Some(end) = quoted.find('"') {
                        let name = &quoted[..end];
                        let mut cand =
                            path.parent().unwrap_or(Path::new(".")).join(name);
                        if !cand.is_file() {
                            cand = pp
                                .include_dirs
                                .iter()
                                .map(|d| d.join(name))
                                .find(|p| p.is_file())
                                .unwrap_or(cand);
                        }
                        if cand.is_file() {
                            let was = pp.primary;
                            pp.primary = false;
                            let r = process_file(&cand, pp, depth + 1);
                            pp.primary = was;
                            r?;
                        }
                    }
                }
            }
            _ => {}
        }
    }
    Ok(())
}

struct DeclParser {
    toks: Vec<(Ctok, bool)>,
    pos: usize,
    fun_like: BTreeMap<String, (Vec<String>, String)>,
    typedefs: BTreeMap<String, Cty>,
    defined_recs: BTreeSet<String>,
    unions: BTreeSet<String>,
    funcs: Vec<CFunc>,
    consts: Vec<(String, ConstLit)>,
    int_consts: BTreeMap<String, i64>,
    warnings: Vec<String>,
    defines: BTreeMap<String, String>,
}

fn tok_is(t: &Ctok, p: &str) -> bool {
    matches!(t, Ctok::Punct(q) if q == p)
}

impl DeclParser {
    fn peek(&self) -> Option<&Ctok> {
        self.toks.get(self.pos).map(|(t, _)| t)
    }

    fn at_primary(&self) -> bool {
        self.toks.get(self.pos).map(|(_, p)| *p).unwrap_or(false)
    }

    fn next(&mut self) -> Option<Ctok> {
        let t = self.toks.get(self.pos).map(|(t, _)| t.clone());
        if t.is_some() {
            self.pos += 1;
        }
        t
    }

    fn eat(&mut self, p: &str) -> bool {
        if matches!(self.peek(), Some(t) if tok_is(t, p)) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    fn skip_balanced(&mut self) {
        let mut depth = 0usize;
        while let Some(t) = self.next() {
            match &t {
                Ctok::Punct(p) if p == "(" => depth += 1,
                Ctok::Punct(p) if p == ")" => {
                    if depth == 0 {
                        break;
                    }
                    depth -= 1;
                }
                _ => {}
            }
        }
    }

    fn skip_braces(&mut self) {
        let mut depth = 1usize;
        while let Some(t) = self.next() {
            match &t {
                Ctok::Punct(p) if p == "{" => depth += 1,
                Ctok::Punct(p) if p == "}" => {
                    depth -= 1;
                    if depth == 0 {
                        break;
                    }
                }
                _ => {}
            }
        }
    }

    fn skip_to_semi(&mut self) {
        let mut pdepth = 0usize;
        let mut bdepth = 0usize;
        while let Some(t) = self.next() {
            match &t {
                Ctok::Punct(p) if p == "(" => pdepth += 1,
                Ctok::Punct(p) if p == ")" => pdepth = pdepth.saturating_sub(1),
                Ctok::Punct(p) if p == "{" => bdepth += 1,
                Ctok::Punct(p) if p == "}" => {
                    if bdepth == 0 {
                        break;
                    }
                    bdepth -= 1;
                }
                Ctok::Punct(p) if p == ";" && pdepth == 0 && bdepth == 0 => break,
                _ => {}
            }
        }
    }

    fn warn(&mut self, msg: String) {
        if self.at_primary() {
            self.warnings.push(msg);
        }
    }

    fn parse_unit(&mut self) {
        while self.pos < self.toks.len() {
            if self.eat(";") {
                continue;
            }
            if !self.parse_decl() {
                self.skip_to_semi();
            }
        }
    }

    fn parse_decl(&mut self) -> bool {
        if matches!(self.peek(), Some(Ctok::Punct(p)) if p == "}") {
            return false;
        }
        if matches!(self.peek(), Some(Ctok::Ident(s)) if s == "extern") {
            let save = self.pos;
            self.pos += 1;
            if matches!(self.peek(), Some(Ctok::Str(_))) {
                self.pos += 1;
            }
            if self.eat("{") {
                while self.pos < self.toks.len() {
                    if self.eat("}") {
                        return true;
                    }
                    if !self.parse_decl() {
                        self.skip_to_semi();
                    }
                }
                return true;
            }
            self.pos = save;
        }
        let primary = self.at_primary();
        let save = self.pos;
        let Some(specs) = self.parse_decl_specs() else {
            self.pos = save;
            return false;
        };
        if specs.is_typedef {
            self.parse_typedef_decl(specs);
            return true;
        }
        let Some(base) = specs.base.clone() else {
            self.pos = save;
            return false;
        };
        if self.eat(";") {
            return true;
        }
        let mut decl = self.parse_declarator();
        let macros = self.skip_fun_like_apps();
        if decl.name.is_none() {
            if self.eat("{") {
                self.skip_braces();
                self.eat(";");
                return true;
            }
            self.skip_to_semi();
            return true;
        }
        if !decl.is_fun && matches!(self.peek(), Some(Ctok::Punct(p)) if p == "(") {
            self.pos += 1;
            decl.is_fun = true;
            decl.fun_params = self.parse_raw_params();
            if !self.eat(")") {
                self.skip_to_semi();
                return true;
            }
        }
        if macros > 0 && !decl.is_fun {
            self.warn("skipping declarator wrapped in a function-like macro".to_string());
            self.skip_to_semi();
            return true;
        }
        if decl.is_fun {
            self.skip_fun_like_apps();
            let name = decl.name.clone().unwrap_or_default();
            let (params, variadic) = self.fun_param_types(decl.fun_params);
            if self.eat("{") {
                self.skip_braces();
            } else if !self.eat(";") {
                self.skip_to_semi();
            }
            if decl.is_funptr {
                return true;
            }
            if !name.is_empty() && primary {
                let mut ret = base;
                for _ in 0..decl.ptr_depth {
                    ret = Cty::Ptr(Box::new(ret));
                }
                self.funcs.push(CFunc { name, ret, params, variadic });
            }
            return true;
        }
        if self.eat("{") {
            self.skip_braces();
            self.eat(";");
            return true;
        }
        self.skip_to_semi();
        true
    }

    fn skip_fun_like_apps(&mut self) -> usize {
        let mut skipped = 0;
        loop {
            let name = match self.peek().cloned() {
                Some(Ctok::Ident(n)) if self.fun_like.contains_key(&n) => n,
                _ => break,
            };
            let after = self.toks.get(self.pos + 1).map(|(t, _)| t.clone());
            if !matches!(after, Some(Ctok::Punct(p)) if p == "(") {
                break;
            }
            let _ = name;
            self.pos += 2;
            let mut depth = 1usize;
            while let Some(t) = self.next() {
                match &t {
                    Ctok::Punct(p) if p == "(" => depth += 1,
                    Ctok::Punct(p) if p == ")" => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    _ => {}
                }
            }
            skipped += 1;
        }
        skipped
    }

    fn fun_param_types(&mut self, pre: Vec<RawParam>) -> (Vec<(String, Cty)>, bool) {
        let mut out = Vec::new();
        let mut variadic = false;
        if pre.len() == 1 && pre[0].is_void_alone {
            return (out, false);
        }
        for p in pre {
            if p.is_ellipsis {
                variadic = true;
                continue;
            }
            out.push((p.name, p.ty));
        }
        (out, variadic)
    }

    fn parse_typedef_decl(&mut self, specs: DeclSpecs) {
        loop {
            let save = self.pos;
            let decl = self.parse_declarator();
            let Some(name) = decl.name.clone() else {
                self.pos = save;
                self.skip_to_semi();
                return;
            };
            let mut ty = specs.base.clone().unwrap_or(Cty::Unknown(name.clone()));
            if decl.is_funptr || decl.is_fun {
                ty = Cty::Fun;
            } else {
                for _ in 0..decl.ptr_depth {
                    ty = Cty::Ptr(Box::new(ty));
                }
            }
            self.typedefs.insert(name, ty);
            if !self.eat(",") {
                break;
            }
        }
        if self.eat("{") {
            self.skip_braces();
        }
        self.eat(";");
    }

    fn parse_decl_specs(&mut self) -> Option<DeclSpecs> {
        let mut specs = DeclSpecs::default();
        let mut seen_type = false;
        let mut unsigned = false;
        let mut short = false;
        let mut long_count = 0usize;
        let mut kind: Option<String> = None;
        loop {
            let t = self.peek()?.clone();
            match &t {
                Ctok::Ident(s) if s == "typedef" => {
                    specs.is_typedef = true;
                    self.pos += 1;
                }
                Ctok::Ident(s)
                    if s == "extern" || s == "static" || s == "auto" || s == "register" =>
                {
                    self.pos += 1;
                }
                Ctok::Ident(s) if s == "const" || s == "volatile" || s == "restrict" => {
                    self.pos += 1;
                }
                Ctok::Ident(s) if s == "inline" || s == "__inline" || s == "__inline__" => {
                    self.pos += 1;
                }
                Ctok::Ident(s)
                    if s == "__cdecl" || s == "__stdcall" || s == "__fastcall"
                        || s == "__thiscall" || s == "SQLITE_CDECL"
                        || s == "SQLITE_STDCALL" =>
                {
                    self.pos += 1;
                }
                Ctok::Ident(s) if s == "__attribute__" => {
                    self.pos += 1;
                    if self.eat("(") {
                        self.skip_balanced();
                    }
                }
                Ctok::Ident(s) if s == "__declspec" => {
                    self.pos += 1;
                    if self.eat("(") {
                        self.skip_balanced();
                    }
                }
                Ctok::Ident(s)
                    if s == "void" || s == "char" || s == "short" || s == "int"
                        || s == "long" || s == "float" || s == "double"
                        || s == "signed" || s == "unsigned" || s == "_Bool" =>
                {
                    self.pos += 1;
                    seen_type = true;
                    match s.as_str() {
                        "unsigned" => unsigned = true,
                        "short" => short = true,
                        "long" => long_count += 1,
                        _ => kind = Some(s.clone()),
                    }
                }
                Ctok::Ident(s) if s == "struct" || s == "union" || s == "enum" => {
                    self.pos += 1;
                    let is_union = s == "union";
                    let is_enum = s == "enum";
                    let name = match self.peek().cloned() {
                        Some(Ctok::Ident(n)) => {
                            self.pos += 1;
                            Some(n)
                        }
                        _ => None,
                    };
                    if self.eat("{") {
                        if is_enum {
                            self.parse_enum_body();
                        } else {
                            self.skip_braces();
                        }
                        if let Some(n) = &name {
                            if is_union {
                                self.unions.insert(n.clone());
                            } else {
                                self.defined_recs.insert(n.clone());
                            }
                        }
                        specs.base = Some(if is_enum {
                            Cty::EnumTy(name.clone().unwrap_or_default())
                        } else if is_union {
                            Cty::Union(name.clone().unwrap_or_default())
                        } else {
                            Cty::Rec(name.clone().unwrap_or_default())
                        });
                        seen_type = true;
                    } else if let Some(n) = name {
                        specs.base = Some(if is_enum {
                            Cty::EnumTy(n)
                        } else if is_union {
                            Cty::Union(n)
                        } else {
                            Cty::Rec(n)
                        });
                        seen_type = true;
                    } else {
                        return None;
                    }
                }
                Ctok::Ident(s) if s == "_Complex" || s == "_Imaginary" => {
                    self.pos += 1;
                    kind = Some("double".to_string());
                    seen_type = true;
                }
                Ctok::Ident(s) => {
                    if seen_type {
                        break;
                    }
                    if self.typedefs.contains_key(s) || Self::is_known_type_name(s) {
                        self.pos += 1;
                        specs.base = Some(Cty::Named(s.clone()));
                        seen_type = true;
                    } else if self.peek_ahead_is_type_start() {
                        self.pos += 1;
                    } else {
                        self.pos += 1;
                        specs.base = Some(Cty::Named(s.clone()));
                        seen_type = true;
                    }
                }
                _ => break,
            }
        }
        if !seen_type && specs.base.is_none() {
            return None;
        }
        if specs.base.is_none() {
            specs.base = Some(combine_specs(kind.as_deref(), unsigned, short, long_count));
        }
        Some(specs)
    }

    fn is_known_type_name(s: &str) -> bool {
        matches!(
            s,
            "bool" | "size_t" | "ssize_t" | "ptrdiff_t" | "intptr_t" | "uintptr_t"
                | "wchar_t" | "char16_t" | "char32_t" | "int8_t" | "uint8_t"
                | "int16_t" | "uint16_t" | "int32_t" | "uint32_t" | "int64_t"
                | "uint64_t" | "int_least8_t" | "int_least16_t" | "int_least32_t"
                | "int_least64_t" | "uint_least8_t" | "uint_least16_t"
                | "uint_least32_t" | "uint_least64_t" | "int_fast8_t"
                | "int_fast16_t" | "int_fast32_t" | "int_fast64_t"
                | "uint_fast8_t" | "uint_fast16_t" | "uint_fast32_t"
                | "uint_fast64_t" | "intmax_t" | "uintmax_t" | "off_t"
                | "time_t" | "FILE" | "va_list" | "fpos_t" | "div_t" | "ldiv_t"
        )
    }

    fn peek_ahead_is_type_start(&self) -> bool {
        match self.toks.get(self.pos + 1).map(|(t, _)| t.clone()) {
            Some(Ctok::Ident(s)) => {
                matches!(
                    s.as_str(),
                    "void" | "char" | "short" | "int" | "long" | "float" | "double"
                        | "signed" | "unsigned" | "const" | "volatile" | "restrict"
                        | "struct" | "union" | "enum" | "bool" | "size_t"
                ) || self.typedefs.contains_key(&s)
                    || Self::is_known_type_name(&s)
            }
            Some(Ctok::Punct(p)) => p == "*" || p == "(",
            _ => false,
        }
    }

    fn parse_declarator(&mut self) -> Declarator {
        let mut d = Declarator::default();
        while self.eat("*") {
            d.ptr_depth += 1;
            while matches!(self.peek(), Some(Ctok::Ident(s)) if s == "const" || s == "volatile" || s == "restrict")
            {
                self.pos += 1;
            }
        }
        self.parse_direct_declarator(d)
    }

    fn parse_direct_declarator(&mut self, mut d: Declarator) -> Declarator {
        match self.peek().cloned() {
            Some(Ctok::Ident(name)) => {
                self.pos += 1;
                if d.name.is_none() {
                    d.name = Some(name);
                }
                self.parse_suffixes(d)
            }
            Some(Ctok::Punct(p)) if p == "(" => {
                self.pos += 1;
                let inner = self.parse_declarator();
                if !self.eat(")") {
                    return d;
                }
                if inner.ptr_depth > 0 || inner.name.is_some() {
                    d.name = inner.name;
                    d.ptr_depth += inner.ptr_depth;
                    d = self.parse_suffixes(d);
                    if d.is_fun {
                        d.is_funptr = true;
                    }
                } else {
                    d = self.parse_suffixes(d);
                }
                d
            }
            _ => d,
        }
    }

    fn parse_suffixes(&mut self, mut d: Declarator) -> Declarator {
        loop {
            if self.eat("[") {
                let mut pdepth = 0usize;
                while let Some(t) = self.next() {
                    match &t {
                        Ctok::Punct(p) if p == "[" => pdepth += 1,
                        Ctok::Punct(p) if p == "]" => {
                            if pdepth == 0 {
                                break;
                            }
                            pdepth -= 1;
                        }
                        _ => {}
                    }
                }
                d.ptr_depth += 1;
            } else if matches!(self.peek(), Some(t) if tok_is(t, "(")) {
                self.pos += 1;
                d.is_fun = true;
                d.fun_params = self.parse_raw_params();
                if !self.eat(")") {
                    return d;
                }
            } else {
                break;
            }
        }
        d
    }

    fn parse_raw_params(&mut self) -> Vec<RawParam> {
        let mut out = Vec::new();
        if self.eat(")") {
            self.pos -= 1;
            return out;
        }
        if matches!(self.peek(), Some(Ctok::Ident(s)) if s == "void") {
            let after = self.toks.get(self.pos + 1).map(|(t, _)| t.clone());
            if matches!(after, Some(Ctok::Punct(p)) if p == ")") {
                self.pos += 1;
                out.push(RawParam {
                    ty: Cty::Void,
                    name: String::new(),
                    is_ellipsis: false,
                    is_void_alone: true,
                });
                return out;
            }
        }
        loop {
            if self.eat("...") {
                out.push(RawParam {
                    ty: Cty::Void,
                    name: String::new(),
                    is_ellipsis: true,
                    is_void_alone: false,
                });
                break;
            }
            let save = self.pos;
            let Some(specs) = self.parse_decl_specs() else {
                self.pos = save;
                break;
            };
            let base = specs.base.clone().unwrap_or(Cty::Unknown("?".to_string()));
            let decl = self.parse_declarator();
            let pname = decl.name.clone().unwrap_or_default();
            let mut ty = base;
            if decl.is_funptr || decl.is_fun {
                ty = Cty::Fun;
            } else {
                for _ in 0..decl.ptr_depth {
                    ty = Cty::Ptr(Box::new(ty));
                }
            }
            out.push(RawParam { ty, name: pname, is_ellipsis: false, is_void_alone: false });
            if !self.eat(",") {
                break;
            }
            if self.eat("...") {
                out.push(RawParam {
                    ty: Cty::Void,
                    name: String::new(),
                    is_ellipsis: true,
                    is_void_alone: false,
                });
                break;
            }
        }
        out
    }

    fn parse_enum_body(&mut self) {
        let mut value: i64 = -1;
        loop {
            if self.eat("}") {
                break;
            }
            let enumerator = match self.next() {
                Some(Ctok::Ident(n)) => n,
                _ => {
                    self.skip_to_brace_or_comma();
                    continue;
                }
            };
            if self.eat("=") {
                let mut expr = String::new();
                let mut pdepth = 0usize;
                while let Some(t) = self.peek().cloned() {
                    match &t {
                        Ctok::Punct(p) if (p == "," || p == "}") && pdepth == 0 => break,
                        Ctok::Punct(p) if p == "(" => {
                            pdepth += 1;
                            expr.push('(');
                            self.pos += 1;
                        }
                        Ctok::Punct(p) if p == ")" => {
                            pdepth = pdepth.saturating_sub(1);
                            expr.push(')');
                            self.pos += 1;
                        }
                        Ctok::Ident(s) => {
                            expr.push_str(s);
                            expr.push(' ');
                            self.pos += 1;
                        }
                        Ctok::Num(n) => {
                            expr.push_str(n);
                            expr.push(' ');
                            self.pos += 1;
                        }
                        Ctok::Chr(c) => {
                            expr.push('\'');
                            expr.push_str(c);
                            expr.push('\'');
                            self.pos += 1;
                        }
                        _ => {
                            self.pos += 1;
                        }
                    }
                }
                value = if_eval(&expr, &self.defines, &self.int_consts);
            } else {
                value = value.wrapping_add(1);
            }
            if self.at_primary() {
                self.consts.push((enumerator.clone(), ConstLit::Int(value)));
            }
            self.int_consts.insert(enumerator, value);
            if !self.eat(",") {
                self.eat("}");
                break;
            }
        }
    }

    fn skip_to_brace_or_comma(&mut self) {
        while let Some(t) = self.peek().cloned() {
            if matches!(t, Ctok::Punct(p) if p == "," || p == "}") {
                break;
            }
            self.pos += 1;
        }
    }
}

#[derive(Debug, Clone, Default)]
struct DeclSpecs {
    is_typedef: bool,
    base: Option<Cty>,
}

#[derive(Debug, Clone)]
struct RawParam {
    ty: Cty,
    name: String,
    is_ellipsis: bool,
    is_void_alone: bool,
}

#[derive(Debug, Clone, Default)]
struct Declarator {
    name: Option<String>,
    ptr_depth: usize,
    is_funptr: bool,
    is_fun: bool,
    fun_params: Vec<RawParam>,
}

fn combine_specs(kind: Option<&str>, unsigned: bool, short: bool, long_count: usize) -> Cty {
    match kind {
        Some("void") => Cty::Void,
        Some("_Bool") => Cty::Bool,
        Some("float") | Some("double") => Cty::Float,
        Some("char") => Cty::Byte,
        Some("short") | Some("int") | Some("signed") | None => Cty::Int,
        _ if long_count > 0 || short || unsigned => Cty::Int,
        _ => Cty::Int,
    }
}

fn resolve_named(ty: &Cty, typedefs: &BTreeMap<String, Cty>) -> Cty {
    let mut cur = ty.clone();
    let mut seen = BTreeSet::new();
    loop {
        match cur {
            Cty::Named(n) => {
                if !seen.insert(n.clone()) {
                    return Cty::Unknown(n);
                }
                match typedefs.get(&n) {
                    Some(t) => cur = t.clone(),
                    None => {
                        let mapped = map_known_name(&n);
                        if matches!(mapped, Cty::Named(_)) {
                            return mapped;
                        }
                        cur = mapped;
                    }
                }
            }
            Cty::Ptr(inner) => {
                let r = resolve_named(&inner, typedefs);
                return Cty::Ptr(Box::new(r));
            }
            _ => return cur,
        }
    }
}

fn map_known_name(n: &str) -> Cty {
    match n {
        "bool" => Cty::Bool,
        "va_list" | "__builtin_va_list" | "__gnuc_va_list" => Cty::VaList,
        "FILE" | "fpos_t" => Cty::Rec(n.to_string()),
        "size_t" | "ssize_t" | "ptrdiff_t" | "intptr_t" | "uintptr_t" | "wchar_t"
        | "char16_t" | "char32_t" | "int16_t" | "uint16_t" | "int32_t"
        | "uint32_t" | "int64_t" | "uint64_t" | "int_least16_t" | "int_least32_t"
        | "int_least64_t" | "uint_least16_t" | "uint_least32_t" | "uint_least64_t"
        | "int_fast16_t" | "int_fast32_t" | "int_fast64_t" | "uint_fast16_t"
        | "uint_fast32_t" | "uint_fast64_t" | "intmax_t" | "uintmax_t" | "off_t"
        | "time_t" => Cty::Int,
        "int8_t" | "uint8_t" => Cty::Byte,
        _ => Cty::Named(n.to_string()),
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum Cat {
    Scalar,
    Str,
    PodPtr,
    ByValue,
    Opaque,
    VoidPtr,
    Callback,
    Complex,
}

fn categorize(ty: &Cty, typedefs: &BTreeMap<String, Cty>, defined: &BTreeSet<String>) -> Cat {
    let r = resolve_named(ty, typedefs);
    match r {
        Cty::Void => Cat::Scalar,
        Cty::Bool | Cty::Byte | Cty::Int | Cty::Float => Cat::Scalar,
        Cty::VaList => Cat::PodPtr,
        Cty::EnumTy(_) => Cat::Scalar,
        Cty::Fun => Cat::Callback,
        Cty::Rec(_) | Cty::Union(_) => Cat::ByValue,
        Cty::Unknown(_) | Cty::Named(_) => Cat::Complex,
        Cty::Ptr(inner) => categorize_ptr(&inner, typedefs, defined),
    }
}

fn categorize_ptr(
    inner: &Cty,
    typedefs: &BTreeMap<String, Cty>,
    defined: &BTreeSet<String>,
) -> Cat {
    let r = resolve_named(inner, typedefs);
    match r {
        Cty::Void => Cat::VoidPtr,
        Cty::Fun => Cat::Callback,
        Cty::VaList => Cat::Complex,
        Cty::Bool | Cty::Int | Cty::Float | Cty::EnumTy(_) => Cat::PodPtr,
        Cty::Byte => Cat::Str,
        Cty::Rec(n) => {
            if defined.contains(&n) {
                Cat::PodPtr
            } else {
                Cat::Opaque
            }
        }
        Cty::Union(_) => Cat::Opaque,
        Cty::Ptr(_) => Cat::Complex,
        Cty::Unknown(_) | Cty::Named(_) => Cat::Complex,
    }
}

#[cfg(test)]
fn cat_name(c: &Cat) -> &'static str {
    match c {
        Cat::Scalar => "scalar",
        Cat::Str => "string",
        Cat::PodPtr => "podptr",
        Cat::ByValue => "byvalue",
        Cat::Opaque => "opaque",
        Cat::VoidPtr => "voidptr",
        Cat::Callback => "callback",
        Cat::Complex => "complex",
    }
}

#[cfg(test)]
pub(crate) fn debug_class(
    f: &CFunc,
    typedefs: &BTreeMap<String, Cty>,
    defined: &BTreeSet<String>,
) -> String {
    let class = classify_func(f, typedefs, defined);
    match class {
        Class::Proxyable => "proxyable/scalars-only".to_string(),
        Class::Marshalable => {
            let mut cats = vec![categorize(&f.ret, typedefs, defined)];
            for (_, p) in &f.params {
                cats.push(categorize(p, typedefs, defined));
            }
            let mut kinds: BTreeSet<&str> = BTreeSet::new();
            for c in &cats {
                if *c != Cat::Scalar {
                    kinds.insert(cat_name(c));
                }
            }
            let v: Vec<&str> = kinds.into_iter().collect();
            format!("marshalable/{}", v.join("+"))
        }
        Class::Handletable => "handletable/opaque-handles-as-ids".to_string(),
        Class::NotProxyable(r) => format!("not-proxyable/{r}"),
    }
}

fn classify_func(f: &CFunc, typedefs: &BTreeMap<String, Cty>, defined: &BTreeSet<String>) -> Class {
    if f.variadic {
        return Class::NotProxyable("variadic".to_string());
    }
    let mut cats = vec![categorize(&f.ret, typedefs, defined)];
    for (_, p) in &f.params {
        cats.push(categorize(p, typedefs, defined));
    }
    if cats.contains(&Cat::Callback) {
        return Class::NotProxyable("callback".to_string());
    }
    if cats.iter().all(|c| {
        matches!(c, Cat::Scalar | Cat::Str | Cat::PodPtr | Cat::ByValue | Cat::Opaque)
    }) && cats.contains(&Cat::Opaque)
    {
        return Class::Handletable;
    }
    if cats.contains(&Cat::Opaque) || cats.contains(&Cat::VoidPtr) || cats.contains(&Cat::Complex)
    {
        let hit = cats
            .iter()
            .find(|c| matches!(c, Cat::Opaque | Cat::VoidPtr | Cat::Complex))
            .unwrap();
        let reason = match hit {
            Cat::Opaque => "opaque",
            Cat::VoidPtr => "voidptr",
            _ => "complex",
        };
        return Class::NotProxyable(reason.to_string());
    }
    if cats.iter().any(|c| matches!(c, Cat::Str | Cat::PodPtr | Cat::ByValue)) {
        return Class::Marshalable;
    }
    Class::Proxyable
}

fn rnx_scalar(ty: &Cty) -> Option<&'static str> {
    match ty {
        Cty::Bool => Some("Bool"),
        Cty::Byte => Some("Byte"),
        Cty::Int | Cty::EnumTy(_) => Some("Int"),
        Cty::Float => Some("Float"),
        _ => None,
    }
}

fn map_type(
    ty: &Cty,
    typedefs: &BTreeMap<String, Cty>,
    defined: &BTreeSet<String>,
) -> Result<Option<String>, String> {
    let r = resolve_named(ty, typedefs);
    match &r {
        Cty::Void | Cty::Fun | Cty::Rec(_) | Cty::Union(_) | Cty::VaList => Ok(None),
        Cty::Bool | Cty::Byte | Cty::Int | Cty::Float | Cty::EnumTy(_) => {
            Ok(Some(rnx_scalar(&r).unwrap().to_string()))
        }
        Cty::Ptr(inner) => {
            let d = resolve_named(inner, typedefs);
            match &d {
                Cty::Void | Cty::Fun | Cty::VaList => Ok(None),
                Cty::Rec(_) | Cty::Union(_) => Ok(Some("Pointer<Byte>".to_string())),
                Cty::Bool | Cty::Byte | Cty::Int | Cty::Float | Cty::EnumTy(_) => {
                    if matches!(&d, Cty::Byte) {
                        Ok(Some("Pointer<Byte>".to_string()))
                    } else {
                        Ok(Some(format!("Pointer<{}>", rnx_scalar(&d).unwrap())))
                    }
                }
                Cty::Ptr(deep) => {
                    let dd = resolve_named(deep, typedefs);
                    match &dd {
                        Cty::Rec(n) if !defined.contains(n) => {
                            Ok(Some("Pointer<Byte>".to_string()))
                        }
                        _ => Ok(None),
                    }
                }
                Cty::Unknown(n) | Cty::Named(n) => {
                    Err(format!("unknown C type `{n}` (add a typedef or skip the function)"))
                }
            }
        }
        Cty::Unknown(n) | Cty::Named(n) => {
            Err(format!("unknown C type `{n}` (add a typedef or skip the function)"))
        }
    }
}

fn expand_fun_like(
    toks: &[Ctok],
    fun_like: &BTreeMap<String, (Vec<String>, Vec<Ctok>)>,
) -> Vec<Ctok> {
    let mut out: Vec<Ctok> = Vec::with_capacity(toks.len());
    let mut i = 0;
    let mut expansions = 0usize;
    while i < toks.len() {
        let t = &toks[i];
        let name = match t {
            Ctok::Ident(n) if fun_like.contains_key(n) => Some(n.clone()),
            _ => None,
        };
        let Some(name) = name else {
            out.push(t.clone());
            i += 1;
            continue;
        };
        if expansions > 10_000 {
            out.push(t.clone());
            i += 1;
            continue;
        }
        let next_is_paren = matches!(toks.get(i + 1), Some(Ctok::Punct(p)) if p == "(");
        if !next_is_paren {
            out.push(t.clone());
            i += 1;
            continue;
        }
        let (params, body) = fun_like[&name].clone();
        let mut depth = 0usize;
        let mut args: Vec<Vec<Ctok>> = vec![Vec::new()];
        let mut j = i + 1;
        let mut ok = true;
        while j < toks.len() {
            match &toks[j] {
                Ctok::Punct(p) if p == "(" => {
                    depth += 1;
                    if depth > 1 {
                        args.last_mut().unwrap().push(toks[j].clone());
                    }
                    j += 1;
                }
                Ctok::Punct(p) if p == ")" => {
                    if depth == 0 {
                        ok = false;
                        break;
                    }
                    depth -= 1;
                    if depth == 0 {
                        j += 1;
                        break;
                    }
                    args.last_mut().unwrap().push(toks[j].clone());
                    j += 1;
                }
                Ctok::Punct(p) if p == "," && depth == 1 => {
                    args.push(Vec::new());
                    j += 1;
                }
                _ => {
                    args.last_mut().unwrap().push(toks[j].clone());
                    j += 1;
                }
            }
        }
        if !ok || depth != 0 {
            out.push(t.clone());
            i += 1;
            continue;
        }
        if args.len() != params.len() || (args.len() == 1 && params.is_empty() && !args[0].is_empty())
        {
            out.push(t.clone());
            i += 1;
            continue;
        }
        if params.is_empty() && args.len() == 1 && args[0].is_empty() {
            args.clear();
        }
        for b in &body {
            match b {
                Ctok::Ident(p) => {
                    if let Some(idx) = params.iter().position(|q| q == p) {
                        for a in &args[idx] {
                            out.push(a.clone());
                        }
                    } else {
                        out.push(b.clone());
                    }
                }
                _ => out.push(b.clone()),
            }
        }
        expansions += 1;
        i = j;
    }
    out
}

fn expand_defines(toks: &[Ctok], defines: &BTreeMap<String, String>) -> Vec<Ctok> {
    let mut obj: BTreeMap<String, Vec<Ctok>> = BTreeMap::new();
    for (name, body) in defines {
        obj.insert(name.clone(), lex_c(body));
    }
    let mut cur: Vec<Ctok> = toks.to_vec();
    for _ in 0..8 {
        let next = expand_once(&cur, &obj);
        if next == cur {
            return next;
        }
        cur = next;
    }
    cur
}

fn expand_once(toks: &[Ctok], obj: &BTreeMap<String, Vec<Ctok>>) -> Vec<Ctok> {
    let mut out = Vec::with_capacity(toks.len());
    for t in toks {
        match t {
            Ctok::Ident(n) => {
                if let Some(rep) = obj.get(n) {
                    for r in rep {
                        out.push(r.clone());
                    }
                } else {
                    out.push(t.clone());
                }
            }
            _ => out.push(t.clone()),
        }
    }
    out
}

fn sanitize_ident(name: &str) -> String {
    let mut out = String::new();
    for c in name.chars() {
        if c.is_ascii_alphanumeric() || c == '_' {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.starts_with(|c: char| c.is_ascii_digit()) {
        out.insert(0, '_');
    }
    if out.is_empty() {
        out.push_str("arg");
    }
    out
}

fn is_rnx_keyword(name: &str) -> bool {
    matches!(
        name,
        "fn" | "let" | "const" | "class" | "struct" | "enum" | "record" | "trait"
            | "interface" | "return" | "if" | "else" | "for" | "while" | "do"
            | "switch" | "break" | "continue" | "new" | "this" | "super"
            | "import" | "export" | "from" | "as" | "unsafe" | "async"
            | "await" | "try" | "catch" | "throw" | "defer" | "init"
            | "deinit" | "true" | "false" | "null" | "is" | "in"
    )
}

fn param_name(raw: &str, idx: usize) -> String {
    let base = if raw.is_empty() { format!("arg{idx}") } else { sanitize_ident(raw) };
    if is_rnx_keyword(&base) {
        format!("{base}_")
    } else {
        base
    }
}

fn escape_str(text: &str) -> String {
    let mut out = String::new();
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{{{:x}}}", c as u32)),
            c => out.push(c),
        }
    }
    out
}

pub fn annotation_set(
    target: &str,
    generator: &str,
    upstream: &str,
    title: &str,
) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    out.insert("run.rasmalai.kind".to_string(), KIND.to_string());
    out.insert("run.rasmalai.role".to_string(), ROLE.to_string());
    out.insert("run.rasmalai.target".to_string(), target.to_string());
    out.insert("run.rasmalai.generator".to_string(), generator.to_string());
    out.insert("run.rasmalai.upstream".to_string(), upstream.to_string());
    out.insert("org.opencontainers.image.title".to_string(), title.to_string());
    out
}

pub fn verify_annotations(ann: &BTreeMap<String, String>) -> Result<(), String> {
    let get = |k: &str| ann.get(k).map(|s| s.as_str()).unwrap_or("");
    if get("run.rasmalai.kind") != KIND {
        return Err(format!(
            "bad run.rasmalai.kind `{}` (expected `{KIND}`)",
            get("run.rasmalai.kind")
        ));
    }
    if get("run.rasmalai.role") != ROLE {
        return Err(format!(
            "bad run.rasmalai.role `{}` (expected `{ROLE}`)",
            get("run.rasmalai.role")
        ));
    }
    if !crate::oci::valid_target(get("run.rasmalai.target")) {
        return Err(format!(
            "bad run.rasmalai.target `{}` (expected an arch-vendor-os-env triple)",
            get("run.rasmalai.target")
        ));
    }
    if !crate::oci::valid_generator(get("run.rasmalai.generator")) {
        return Err(format!(
            "bad run.rasmalai.generator `{}` (expected `<name> <semver>`)",
            get("run.rasmalai.generator")
        ));
    }
    if !crate::oci::valid_upstream(get("run.rasmalai.upstream")) {
        return Err(format!(
            "bad run.rasmalai.upstream `{}` (expected `<name>@sha256:<hex>`)",
            get("run.rasmalai.upstream")
        ));
    }
    if !crate::oci::valid_title(get("org.opencontainers.image.title")) {
        return Err(format!(
            "bad org.opencontainers.image.title `{}` (expected a plain filename)",
            get("org.opencontainers.image.title")
        ));
    }
    Ok(())
}

fn render_oci_json(
    annotations: &BTreeMap<String, String>,
    digest: &str,
    tally: &ClassTally,
    skipped: &[(String, String)],
) -> String {
    let mut out = String::from("{\n");
    out.push_str(&format!("  \"digest\": \"{digest}\",\n"));
    out.push_str("  \"annotations\": {\n");
    let mut first = true;
    for (k, v) in annotations {
        if !first {
            out.push_str(",\n");
        }
        first = false;
        out.push_str(&format!("    \"{k}\": \"{v}\""));
    }
    out.push_str("\n  },\n");
    out.push_str(&format!(
        "  \"profile\": {{\"proxyable\": {}, \"marshalable\": {}, \"handletable\": {}, \"not_proxyable\": {}, \"skipped\": [",
        tally.proxyable, tally.marshalable, tally.handletable, tally.not_proxyable
    ));
    for (i, (name, reason)) in skipped.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!("{{\"name\": \"{name}\", \"reason\": \"{reason}\"}}"));
    }
    out.push_str("]}\n}\n");
    out
}

pub fn generate(opts: &BindgenOptions) -> Result<BindgenOutput, String> {
    if opts.headers.is_empty() {
        return Err("no --header given".to_string());
    }
    if opts.native_lib.is_empty() {
        return Err("no --native-lib given".to_string());
    }
    if opts.native_lib.contains('"') || opts.native_lib.contains('\n') {
        return Err("bad --native-lib (must not contain quotes)".to_string());
    }
    if opts.pkg_name.is_empty() {
        return Err("no --pkg-name given".to_string());
    }
    let mut sorted_headers = opts.headers.clone();
    sorted_headers.sort();
    let mut upstream_bytes = Vec::new();
    for h in &sorted_headers {
        let bytes =
            std::fs::read(h).map_err(|e| format!("cannot read `{}`: {e}", h.display()))?;
        upstream_bytes.extend_from_slice(&bytes);
    }
    let upstream_hex = Sha256::hexdigest(&upstream_bytes);
    let upstream_name = opts.upstream_name.clone().unwrap_or_else(|| opts.pkg_name.clone());
    if upstream_name.is_empty()
        || !upstream_name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
    {
        return Err(format!(
            "bad upstream name `{upstream_name}` (expected `[A-Za-z0-9._-]+`)"
        ));
    }
    let upstream = format!("{upstream_name}@sha256:{upstream_hex}");
    let header_names: Vec<String> = sorted_headers
        .iter()
        .map(|h| {
            h.file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_else(|| h.display().to_string())
        })
        .collect();

    let pred = predefines(&opts.target)?;
    let mut pp = Pp {
        include_dirs: &opts.include_dirs,
        primary: true,
        toks: Vec::new(),
        state: PpState {
            defines: pred,
            fun_like: BTreeMap::new(),
            primary_defines: BTreeSet::new(),
            active: Vec::new(),
            taken: Vec::new(),
        },
        consts: BTreeMap::new(),
        visited: BTreeSet::new(),
    };
    for h in &sorted_headers {
        process_file(h, &mut pp, 0)?;
    }
    if !pp.state.active.is_empty() {
        return Err("unterminated conditional in headers".to_string());
    }

    let mut parser = DeclParser {
        toks: pp.toks,
        pos: 0,
        fun_like: pp.state.fun_like,
        typedefs: BTreeMap::new(),
        defined_recs: BTreeSet::new(),
        unions: BTreeSet::new(),
        funcs: Vec::new(),
        consts: Vec::new(),
        int_consts: BTreeMap::new(),
        warnings: Vec::new(),
        defines: pp.state.defines,
    };
    parser.parse_unit();
    let primary_defs = pp.state.primary_defines;

    let mut seen_consts = BTreeSet::new();
    let mut consts: Vec<ConstVal> = Vec::new();
    for (name, lit) in parser.consts {
        if seen_consts.insert(name.clone()) {
            consts.push(ConstVal { name, value: lit });
        }
    }
    let mut define_consts: Vec<ConstVal> = Vec::new();
    let mut int_consts = parser.int_consts;
    for name in &primary_defs {
        if seen_consts.contains(name) {
            continue;
        }
        if let Some(body) = parser.defines.get(name).cloned() {
            let body = body.trim().to_string();
            if body.starts_with('"') && body.ends_with('"') && body.len() >= 2 {
                seen_consts.insert(name.clone());
                define_consts.push(ConstVal {
                    name: name.clone(),
                    value: ConstLit::Str(body[1..body.len() - 1].to_string()),
                });
                continue;
            }
            if !body.is_empty()
                && !body.starts_with('"')
                && let Some(v) = const_expr_value(&body, &parser.defines, &int_consts)
            {
                int_consts.insert(name.clone(), v);
                seen_consts.insert(name.clone());
                define_consts.push(ConstVal { name: name.clone(), value: ConstLit::Int(v) });
            }
        }
    }
    for c in define_consts {
        consts.push(c);
    }
    consts.sort_by(|a, b| a.name.cmp(&b.name));

    let mut tally = ClassTally::default();
    let mut kept: Vec<(CFunc, EmitShape)> = Vec::new();
    let mut skipped: Vec<(String, String)> = Vec::new();
    let mut warnings = parser.warnings;
    let mut seen_fns = BTreeSet::new();
    for f in parser.funcs {
        if !seen_fns.insert(f.name.clone()) {
            continue;
        }
        if !opts.prefixes.is_empty() && !opts.prefixes.iter().any(|p| f.name.starts_with(p)) {
            continue;
        }
        if opts.skips.iter().any(|s| s == &f.name) {
            skipped.push((f.name.clone(), "skip".to_string()));
            continue;
        }
        let class = classify_func(&f, &parser.typedefs, &parser.defined_recs);
        match &class {
            Class::Proxyable => tally.proxyable += 1,
            Class::Marshalable => tally.marshalable += 1,
            Class::Handletable => tally.handletable += 1,
            Class::NotProxyable(_) => tally.not_proxyable += 1,
        }
        let has_va_list = {
            let mut all: Vec<&Cty> = vec![&f.ret];
            all.extend(f.params.iter().map(|(_, t)| t));
            all.iter()
                .any(|t| matches!(resolve_named(t, &parser.typedefs), Cty::VaList))
        };
        if has_va_list {
            skipped.push((f.name.clone(), "va_list".to_string()));
            continue;
        }
        let hard_skip = match &class {
            Class::NotProxyable(r) => r == "variadic" || r == "callback" || r == "voidptr",
            _ => false,
        };
        if hard_skip {
            let reason = match class {
                Class::NotProxyable(r) => r,
                _ => "complex".to_string(),
            };
            skipped.push((f.name.clone(), reason));
            continue;
        }
        match emit_shape(&f, &parser.typedefs, &parser.defined_recs) {
            Ok(Some(shape)) => {
                skipped.retain(|(n, _)| n != &f.name);
                kept.push((f, shape));
            }
            Ok(None) => {
                let reason = match class {
                    Class::NotProxyable(r) => r,
                    _ => "complex".to_string(),
                };
                if !skipped.iter().any(|(n, _)| n == &f.name) {
                    skipped.push((f.name.clone(), reason));
                }
            }
            Err(e) => return Err(format!("{}: {e}", f.name)),
        }
    }
    skipped.sort();
    kept.sort_by(|(a, _), (b, _)| a.name.cmp(&b.name));

    let mut filtered_consts: Vec<ConstVal> = Vec::new();
    for c in consts {
        if !opts.prefixes.is_empty() && !opts.prefixes.iter().any(|p| c.name.starts_with(p)) {
            continue;
        }
        if opts.skips.iter().any(|s| s == &c.name) {
            continue;
        }
        filtered_consts.push(c);
    }

    let mut rnx = String::new();
    rnx.push_str(&format!(
        "// @generated {} target={} upstream={} from={}\n",
        generator_string(),
        opts.target,
        upstream,
        header_names.join(",")
    ));
    rnx.push('\n');
    if !kept.is_empty() {
        rnx.push_str("import {\n");
        for (i, (f, shape)) in kept.iter().enumerate() {
            if i > 0 {
                rnx.push_str(",\n");
            }
            if shape.ret == "Void" {
                rnx.push_str(&format!(
                    "    fn {}({}) as __rnx_{}",
                    f.name,
                    shape.params,
                    sanitize_ident(&f.name)
                ));
            } else {
                rnx.push_str(&format!(
                    "    fn {}({}): {} as __rnx_{}",
                    f.name,
                    shape.params,
                    shape.ret,
                    sanitize_ident(&f.name)
                ));
            }
        }
        rnx.push_str("\n} from native \"");
        rnx.push_str(&opts.native_lib);
        rnx.push_str("\"\n\n");
    }
    for (f, shape) in &kept {
        let ret_ann = if shape.ret == "Void" { String::new() } else { format!(": {}", shape.ret) };
        rnx.push_str(&format!("unsafe fn {}({}){} {{\n", f.name, shape.params, ret_ann));
        if shape.ret == "Void" {
            rnx.push_str(&format!(
                "    __rnx_{}({});\n",
                sanitize_ident(&f.name),
                shape.args
            ));
        } else {
            rnx.push_str(&format!(
                "    return __rnx_{}({});\n",
                sanitize_ident(&f.name),
                shape.args
            ));
        }
        rnx.push_str("}\n");
    }
    if !filtered_consts.is_empty() {
        rnx.push('\n');
        for c in &filtered_consts {
            let (ty, val) = match &c.value {
                ConstLit::Int(n) => ("Int", n.to_string()),
                ConstLit::Str(s) => ("String", format!("\"{}\"", escape_str(s))),
            };
            rnx.push_str(&format!("export const {}: {ty} = {val}\n", c.name));
        }
    }
    if !kept.is_empty() {
        rnx.push('\n');
        rnx.push_str("export {\n");
        for (i, (f, _)) in kept.iter().enumerate() {
            if i > 0 {
                rnx.push_str(",\n");
            }
            rnx.push_str(&format!("    {}", f.name));
        }
        rnx.push_str("\n}\n");
    }

    for (name, reason) in &skipped {
        if reason == "skip" {
            continue;
        }
        warnings.push(format!("{name}: not emitted ({reason})"));
    }
    warnings.sort();
    warnings.dedup();

    let annotations = annotation_set(&opts.target, &generator_string(), &upstream, TITLE);
    verify_annotations(&annotations)
        .map_err(|e| format!("generated annotations fail verification: {e}"))?;
    let digest = format!("sha256:{}", Sha256::hexdigest(rnx.as_bytes()));
    let oci_json = render_oci_json(&annotations, &digest, &tally, &skipped);
    let emitted = kept.len();
    let consts_count = filtered_consts.len();
    Ok(BindgenOutput { rnx, oci_json, annotations, tally, emitted, consts: consts_count, skipped, warnings })
}

struct EmitShape {
    params: String,
    args: String,
    ret: String,
}

fn emit_shape(
    f: &CFunc,
    typedefs: &BTreeMap<String, Cty>,
    defined: &BTreeSet<String>,
) -> Result<Option<EmitShape>, String> {
    if f.variadic {
        return Ok(None);
    }
    let mut params = Vec::new();
    let mut args = Vec::new();
    for (i, (pname, p)) in f.params.iter().enumerate() {
        let t = map_type(p, typedefs, defined)?;
        let Some(t) = t else {
            return Ok(None);
        };
        if t == "Void" {
            return Ok(None);
        }
        let aname = param_name(pname, i);
        params.push(format!("{aname}: {t}"));
        args.push(aname);
    }
    let ret = match &f.ret {
        Cty::Void => "Void".to_string(),
        _ => {
            let t = map_type(&f.ret, typedefs, defined)?;
            match t {
                Some(t) if t != "Void" => t,
                _ => return Ok(None),
            }
        }
    };
    Ok(Some(EmitShape { params: params.join(", "), args: args.join(", "), ret }))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_opts() -> BindgenOptions {
        BindgenOptions {
            headers: Vec::new(),
            include_dirs: Vec::new(),
            native_lib: "test".to_string(),
            pkg_name: "test".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            prefixes: Vec::new(),
            skips: Vec::new(),
            upstream_name: None,
        }
    }

    fn gen_src(src: &str) -> Result<BindgenOutput, String> {
        let dir = crate::scratch::dir("bindgen-unit");
        let h = dir.join("t.h");
        std::fs::write(&h, src).unwrap();
        let mut opts = test_opts();
        opts.headers = vec![h];
        generate(&opts)
    }

    #[test]
    fn bool_maps_to_bool_not_pointer() {
        let out = gen_src("typedef unsigned char Bool;\n_Bool flag(int x);\n").unwrap();
        assert!(out.rnx.contains("fn flag(x: Int): Bool as __rnx_flag"), "{}", out.rnx);
    }

    #[test]
    fn unknown_type_errors() {
        let err = gen_src("Mystery foo(int x);\n").unwrap_err();
        assert!(err.contains("unknown C type"), "{err}");
    }

    #[test]
    fn callback_voidptr_variadic_warn_not_emit() {
        let out = gen_src(
            "typedef void *Handle;\nint cb(int (*f)(int));\nvoid *alloc(int n);\nint flex(const char *fmt, ...);\nint add(int a, int b);\n",
        )
        .unwrap();
        assert!(out.rnx.contains("unsafe fn add"), "{}", out.rnx);
        assert!(!out.rnx.contains("unsafe fn cb"), "{}", out.rnx);
        assert!(!out.rnx.contains("unsafe fn alloc"), "{}", out.rnx);
        assert!(!out.rnx.contains("unsafe fn flex"), "{}", out.rnx);
        let reasons: BTreeMap<&str, &str> =
            out.skipped.iter().map(|(n, r)| (n.as_str(), r.as_str())).collect();
        assert_eq!(reasons.get("cb"), Some(&"callback"));
        assert_eq!(reasons.get("alloc"), Some(&"voidptr"));
        assert_eq!(reasons.get("flex"), Some(&"variadic"));
    }

    #[test]
    fn empty_prefix_means_everything_and_skip_is_exact() {
        let src = "int foo_one(int x);\nint foo_two(int x);\nint bar_two(int x);\n";
        let dir = crate::scratch::dir("bindgen-prefix");
        let h = dir.join("t.h");
        std::fs::write(&h, src).unwrap();
        let mut opts = test_opts();
        opts.headers = vec![h.clone()];
        opts.prefixes = vec!["foo_".to_string()];
        opts.skips = vec!["foo_two".to_string()];
        let out = generate(&opts).unwrap();
        assert!(out.rnx.contains("unsafe fn foo_one"), "{}", out.rnx);
        assert!(!out.rnx.contains("unsafe fn foo_two"), "{}", out.rnx);
        assert!(!out.rnx.contains("unsafe fn bar_two"), "{}", out.rnx);
        assert!(out.skipped.iter().any(|(n, r)| n == "foo_two" && r == "skip"));
    }

    #[test]
    fn opaque_handle_double_ptr_emits() {
        let out = gen_src("typedef struct sqlite3 sqlite3;\nint open(const char *name, sqlite3 **db);\n").unwrap();
        assert!(out.rnx.contains("unsafe fn open"), "{}", out.rnx);
        assert!(out.rnx.contains("Pointer<Byte>"), "{}", out.rnx);
    }

    #[test]
    fn golden_small_header() {
        let out = gen_src(
            "#define MAGIC 42\ntypedef unsigned char Byte;\nunsigned long size(int n);\nvoid done(void);\n",
        )
        .unwrap();
        assert!(out.rnx.contains("// @generated rnx-bindgen "), "{}", out.rnx);
        assert!(out.rnx.contains("export const MAGIC: Int = 42"), "{}", out.rnx);
        assert!(out.rnx.contains("unsafe fn size(n: Int): Int"), "{}", out.rnx);
        assert!(out.rnx.contains("unsafe fn done()"), "{}", out.rnx);
        assert!(out.rnx.contains("fn done() as __rnx_done"), "{}", out.rnx);
        verify_annotations(&out.annotations).unwrap();
        assert_eq!(out.annotations.get("run.rasmalai.kind").map(|s| s.as_str()), Some("bindings"));
        assert_eq!(out.annotations.get("run.rasmalai.role").map(|s| s.as_str()), Some("bindgen"));
    }

    #[test]
    fn determinism_same_bytes_twice() {
        let src = "int zeta(int a);\nint alpha(int b);\n#define Z 1\n#define A 2\n";
        let first = gen_src(src).unwrap();
        let second = gen_src(src).unwrap();
        let norm = |s: &str| {
            s.lines()
                .filter(|l| !l.contains("upstream="))
                .collect::<Vec<_>>()
                .join("\n")
        };
        assert_eq!(norm(&first.rnx), norm(&second.rnx));
        let pos_alpha = first.rnx.find("unsafe fn alpha").unwrap();
        let pos_zeta = first.rnx.find("unsafe fn zeta").unwrap();
        assert!(pos_alpha < pos_zeta, "{}", first.rnx);
    }

    #[test]
    fn profile_reports_classifier_refusals_as_advisory() {
        let out = gen_src(
            "typedef void *Handle;\nint cb(int (*f)(int));\nvoid *alloc(int n);\nint flex(const char *fmt, ...);\ntypedef struct sqlite3 sqlite3;\nint open(const char *name, sqlite3 **db);\nint add(int a, int b);\n",
        )
        .unwrap();
        assert!(out.skipped.iter().any(|(n, r)| n == "cb" && r == "callback"));
        assert!(out.skipped.iter().any(|(n, r)| n == "alloc" && r == "voidptr"));
        assert!(out.skipped.iter().any(|(n, r)| n == "flex" && r == "variadic"));
        assert!(out.oci_json.contains("\"profile\""), "{}", out.oci_json);
        assert!(out.oci_json.contains("\"reason\": \"callback\""), "{}", out.oci_json);
    }
}
