use crate::ast::*;
use crate::token::{Token, TokenKind};
use diagnostics::{Code, Diagnostic, Span};

pub struct Parser {
    toks: Vec<Token>,
    pos: usize,
    brace_ok: bool,
    errors: Vec<Diagnostic>,
    depth: u32,
}

const MAX_PARSE_DEPTH: u32 = 64;

type PR<T> = Result<T, Diagnostic>;

impl Parser {
    pub fn new(toks: Vec<Token>) -> Parser {
        Parser {
            toks,
            pos: 0,
            brace_ok: true,
            errors: Vec::new(),
            depth: 0,
        }
    }

    pub fn parse_module(src: &str) -> Result<Module, Diagnostic> {
        let toks = crate::lexer::lex(src)?;
        let mut p = Parser::new(toks);
        match p.module() {
            Ok(mut m) => {
                m.warnings = p.take_warnings();
                match p.errors.into_iter().next() {
                    Some(e) => Err(e),
                    None => Ok(m),
                }
            }
            Err(e) => Err(e),
        }
    }

    pub fn parse_module_all(src: &str) -> Result<Module, Vec<Diagnostic>> {
        let toks = match crate::lexer::lex(src) {
            Ok(toks) => toks,
            Err(e) => return Err(vec![e]),
        };
        let mut p = Parser::new(toks);
        match p.module() {
            Ok(mut m) => {
                m.warnings = p.take_warnings();
                if p.errors.is_empty() {
                    Ok(m)
                } else {
                    Err(p.errors)
                }
            }
            Err(e) => {
                p.errors.push(e);
                Err(p.errors)
            }
        }
    }

    fn peek(&self) -> &TokenKind {
        &self.toks[self.pos].kind
    }

    fn peek_span(&self) -> Span {
        self.toks[self.pos].span
    }

    fn bump(&mut self) -> Token {
        let t = self.toks[self.pos].clone();
        if self.pos + 1 < self.toks.len() {
            self.pos += 1;
        }
        t
    }

    fn at(&self, k: &TokenKind) -> bool {
        self.peek() == k
    }

    fn eat(&mut self, k: &TokenKind) -> bool {
        if self.at(k) {
            self.bump();
            true
        } else {
            false
        }
    }

    fn expect(&mut self, k: &TokenKind, what: &str) -> PR<Token> {
        if self.at(k) {
            Ok(self.bump())
        } else {
            Err(Diagnostic::new(
                Code::E108,
                format!("expected {what}, found {}", describe(self.peek())),
            )
            .with_span(self.peek_span()))
        }
    }

    fn eat_semi(&mut self) {
        while self.at(&TokenKind::Semi) {
            self.bump();
        }
    }

    fn close_angle(&mut self) -> PR<()> {
        if self.at(&TokenKind::Gt) {
            self.bump();
            return Ok(());
        }
        let span = self.peek_span();
        match self.peek() {
            TokenKind::Shr => {
                self.toks[self.pos] = Token {
                    kind: TokenKind::Gt,
                    span: Span { start: span.start, end: span.start + 1 },
                };
                self.toks.insert(
                    self.pos + 1,
                    Token {
                        kind: TokenKind::Gt,
                        span: Span { start: span.start + 1, end: span.end },
                    },
                );
                self.bump();
                Ok(())
            }
            TokenKind::Zshr => {
                self.toks[self.pos] = Token {
                    kind: TokenKind::Gt,
                    span: Span { start: span.start, end: span.start + 1 },
                };
                self.toks.insert(
                    self.pos + 1,
                    Token {
                        kind: TokenKind::Shr,
                        span: Span { start: span.start + 1, end: span.end },
                    },
                );
                self.bump();
                Ok(())
            }
            _ => self.expect(&TokenKind::Gt, "`>`").map(|_| ()),
        }
    }

    fn is_decl_start(k: &TokenKind) -> bool {
        matches!(
            k,
            TokenKind::Fn
                | TokenKind::Class
                | TokenKind::Struct
                | TokenKind::Record
                | TokenKind::Trait
                | TokenKind::Enum
                | TokenKind::Import
                | TokenKind::Export
                | TokenKind::Native
                | TokenKind::Const
                | TokenKind::Static
                | TokenKind::Public
                | TokenKind::Private
                | TokenKind::Unsafe
                | TokenKind::Async
                | TokenKind::Hash
                | TokenKind::DocComment(_)
                | TokenKind::DocModule(_)
        )
    }

    fn is_stmt_start(k: &TokenKind) -> bool {
        matches!(
            k,
            TokenKind::Let
                | TokenKind::Const
                | TokenKind::Static
                | TokenKind::If
                | TokenKind::While
                | TokenKind::For
                | TokenKind::Switch
                | TokenKind::Return
                | TokenKind::Break
                | TokenKind::Continue
                | TokenKind::Defer
                | TokenKind::Guard
                | TokenKind::Try
                | TokenKind::Throw
                | TokenKind::Unsafe
                | TokenKind::Do
                | TokenKind::Fallthrough
                | TokenKind::Pass
                | TokenKind::Fn
                | TokenKind::Class
                | TokenKind::Case
                | TokenKind::Default
        )
    }

    fn is_member_start(k: &TokenKind) -> bool {
        matches!(
            k,
            TokenKind::Public
                | TokenKind::Private
                | TokenKind::Static
                | TokenKind::Fn
                | TokenKind::Unsafe
                | TokenKind::Let
                | TokenKind::Const
                | TokenKind::Init
                | TokenKind::Deinit
                | TokenKind::OnReload
                | TokenKind::Ident(_)
                | TokenKind::DocComment(_)
        )
    }

    fn is_enum_start(k: &TokenKind) -> bool {
        matches!(
            k,
            TokenKind::Let
                | TokenKind::Const
                | TokenKind::Ident(_)
                | TokenKind::DocComment(_)
                | TokenKind::Hash
        )
    }

    fn enum_member(&mut self) -> PR<EnumMember> {
        let pending = self.doc_prefix();
        self.eat(&TokenKind::Let);
        self.eat(&TokenKind::Const);
        let (n, _) = self.ident()?;
        let mut payload = Vec::new();
        if self.eat(&TokenKind::LParen) {
            while !self.at(&TokenKind::RParen) {
                if self.at(&TokenKind::Eof) {
                    return Err(Diagnostic::new(Code::E108, "unterminated enum payload")
                        .with_span(self.peek_span()));
                }
                payload.push(self.ty()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
        }
        self.eat_semi();
        self.eat(&TokenKind::Comma);
        Ok(EnumMember {
            name: n,
            payload,
            docs: pending,
        })
    }

    fn skip_arrow_body(&mut self) {
        self.bump();
        if self.at(&TokenKind::LBrace) {
            let mut depth = 0u32;
            while !self.at(&TokenKind::Eof) {
                match self.peek() {
                    TokenKind::LBrace => {
                        depth += 1;
                        self.bump();
                    }
                    TokenKind::RBrace => {
                        self.bump();
                        if depth <= 1 {
                            return;
                        }
                        depth -= 1;
                    }
                    _ => {
                        self.bump();
                    }
                }
            }
            return;
        }
        let mut depth = 0u32;
        while !self.at(&TokenKind::Eof) {
            match self.peek() {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RParen | TokenKind::RBracket => {
                    if depth > 0 {
                        depth -= 1;
                    }
                    self.bump();
                }
                TokenKind::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.bump();
                }
                TokenKind::Semi => {
                    self.bump();
                    return;
                }
                _ => {
                    self.bump();
                }
            }
        }
    }

    fn synchronize(&mut self, is_follower: fn(&TokenKind) -> bool) {
        let mut depth = 0u32;
        while !self.at(&TokenKind::Eof) {
            match self.peek() {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => {
                    depth += 1;
                    self.bump();
                }
                TokenKind::RParen | TokenKind::RBracket => {
                    if depth > 0 {
                        depth -= 1;
                    }
                    self.bump();
                }
                TokenKind::RBrace => {
                    if depth == 0 {
                        return;
                    }
                    depth -= 1;
                    self.bump();
                }
                TokenKind::Semi => {
                    self.bump();
                    if depth == 0 {
                        return;
                    }
                }
                _ => {
                    if depth == 0 && is_follower(self.peek()) {
                        return;
                    }
                    self.bump();
                }
            }
        }
    }

    fn record_key(&mut self) -> PR<String> {
        if let TokenKind::Ident(s) = self.peek().clone() {
            self.bump();
            return Ok(s);
        }
        self.member_name()
    }

    fn member_name(&mut self) -> PR<String> {
        if let TokenKind::Ident(s) = self.peek().clone() {
            self.bump();
            return Ok(s);
        }
        let w = match self.peek() {
            TokenKind::Default => "default",
            TokenKind::Case => "case",
            TokenKind::Switch => "switch",
            TokenKind::Do => "do",
            TokenKind::Is => "is",
            TokenKind::As => "as",
            TokenKind::From => "from",
            TokenKind::Export => "export",
            TokenKind::Native => "native",
            TokenKind::In => "in",
            TokenKind::With => "with",
            TokenKind::Init => "init",
            TokenKind::New => "new",
            TokenKind::Deinit => "deinit",
            TokenKind::OnReload => "onReload",
            TokenKind::This => "this",
            TokenKind::Record => "record",
            TokenKind::Unsafe => "unsafe",
            TokenKind::Await => "await",
            TokenKind::Public => "pub",
            TokenKind::Private => "private",
            TokenKind::Const => "const",
            TokenKind::Static => "static",
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("expected member name, found {}", describe(self.peek())),
                )
                .with_span(self.peek_span()));
            }
        };
        self.bump();
        Ok(w.to_string())
    }

    fn ident(&mut self) -> PR<(String, Span)> {
        match self.peek().clone() {
            TokenKind::Ident(s) => {
                let sp = self.peek_span();
                self.bump();
                Ok((s, sp))
            }
            TokenKind::This => {
                let sp = self.peek_span();
                self.bump();
                Ok(("this".to_string(), sp))
            }
            TokenKind::OnReload => {
                let sp = self.peek_span();
                self.bump();
                Ok(("onReload".to_string(), sp))
            }
            TokenKind::Init => {
                let sp = self.peek_span();
                self.bump();
                Ok(("init".to_string(), sp))
            }
            TokenKind::Deinit => {
                let sp = self.peek_span();
                self.bump();
                Ok(("deinit".to_string(), sp))
            }
            _ => Err(Diagnostic::new(
                Code::E108,
                format!("expected identifier, found {}", describe(self.peek())),
            )
            .with_span(self.peek_span())),
        }
    }

    fn doc_prefix(&mut self) -> String {
        let mut parts = Vec::new();
        while let TokenKind::DocComment(t) = self.peek().clone() {
            self.bump();
            parts.push(t);
        }
        parts.join("\n\n")
    }

    fn module(&mut self) -> PR<Module> {
        let mut inner_attrs = Vec::new();
        while self.at(&TokenKind::Hash) {
            if self.pos + 1 < self.toks.len()
                && matches!(self.toks[self.pos + 1].kind, TokenKind::Bang)
            {
                inner_attrs.push(self.attr(true)?);
            } else {
                break;
            }
        }
        let mut decls = Vec::new();
        let mut module_docs = String::new();
        let mut pending: Vec<String> = Vec::new();
        while !self.at(&TokenKind::Eof) {
            if self.at(&TokenKind::Semi) {
                self.bump();
                continue;
            }
            match self.peek().clone() {
                TokenKind::DocModule(t) => {
                    self.bump();
                    if !module_docs.is_empty() {
                        module_docs.push_str("\n\n");
                    }
                    module_docs.push_str(&t);
                    continue;
                }
                TokenKind::DocComment(t) => {
                    self.bump();
                    pending.push(t);
                    continue;
                }
                _ => {}
            }
            let docs = pending.join("\n\n");
            pending.clear();
            let save = self.pos;
            let s = self.peek_span().start;
            match self.decl() {
                Ok(mut d) => {
                    Self::attach_docs(&mut d, &docs);
                    let e = self.peek_span().start;
                    decls.push(sp(d, Span { start: s, end: e }));
                }
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize(Self::is_decl_start);
                    if self.pos == save {
                        self.bump();
                    }
                }
            }
        }
        Ok(Module { inner_attrs, decls, docs: module_docs, warnings: Vec::new() })
    }

    fn take_warnings(&mut self) -> Vec<Diagnostic> {
        let mut warnings = Vec::new();
        self.errors.retain(|e| {
            if e.code.is_warning() {
                warnings.push(e.clone());
                false
            } else {
                true
            }
        });
        warnings
    }

fn attach_docs(d: &mut Decl, docs: &str) {
    match d {
        Decl::Fn(f) => f.docs = docs.to_string(),
        Decl::Class { docs: d, .. }
        | Decl::Struct { docs: d, .. }
        | Decl::Trait { docs: d, .. }
        | Decl::Interface { docs: d, .. }
        | Decl::Extension { docs: d, .. }
        | Decl::Enum { docs: d, .. }
        | Decl::Record { docs: d, .. }
        | Decl::Const { docs: d, .. } => *d = docs.to_string(),
        _ => {}
    }
}

fn attach_member_docs(m: &mut ClassMember, docs: &str) {
    match m {
        ClassMember::Method(f) => f.docs = docs.to_string(),
        ClassMember::Field(f) => f.docs = docs.to_string(),
        _ => {}
    }
}

    fn attr(&mut self, inner: bool) -> PR<Attr> {
        let s = self.peek_span().start;
        self.expect(&TokenKind::Hash, "`#`")?;
        if inner {
            self.expect(&TokenKind::Bang, "`!`")?;
        }
        self.expect(&TokenKind::LBracket, "`[`")?;
        let (name, _) = self.ident()?;
        let mut args = Vec::new();
        if self.eat(&TokenKind::LParen) {
            while !self.at(&TokenKind::RParen) {
                if self.at(&TokenKind::Eof) {
                    return Err(Diagnostic::new(Code::E108, "unterminated attribute")
                        .with_span(self.peek_span()));
                }
                let es = self.peek_span().start;
                let e = self.expr()?;
                let ee = self.peek_span().start;
                args.push(sp(e, Span { start: es, end: ee }));
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
        }
        self.expect(&TokenKind::RBracket, "`]`")?;
        let e = self.peek_span().start;
        Ok(Attr {
            inner,
            name,
            args,
            span: Span { start: s, end: e },
        })
    }

    fn outer_attrs(&mut self) -> PR<Vec<Attr>> {
        let mut out = Vec::new();
        while self.at(&TokenKind::Hash) {
            out.push(self.attr(false)?);
        }
        Ok(out)
    }

    fn access(&mut self) -> Visibility {
        if self.eat(&TokenKind::Export) {
            Visibility::Export
        } else if self.eat(&TokenKind::Public) {
            let span = self.toks[self.pos.saturating_sub(1)].span;
            self.errors.push(
                Diagnostic::new(
                    Code::W204,
                    "'pub' is deprecated and will be removed in a future release; use 'export'",
                )
                .with_span(span),
            );
            Visibility::Export
        } else if self.eat(&TokenKind::Private) {
            Visibility::Private
        } else {
            Visibility::Internal
        }
    }

    fn let_decl_as_decl(&mut self) -> PR<Decl> {
        let s = self.peek_span().start;
        match self.stmt() {
            Ok(st) => {
                let e = self.peek_span().start;
                Ok(Decl::Stmt(Box::new(sp(st, Span { start: s, end: e }))))
            }
            Err(e) if e.code == Code::E206 => {
                self.errors.push(e);
                self.eat_semi();
                let epos = self.peek_span().start;
                Ok(Decl::Stmt(Box::new(sp(
                    Stmt::Empty,
                    Span { start: s, end: epos },
                ))))
            }
            Err(e) => Err(e),
        }
    }

    fn decl(&mut self) -> PR<Decl> {
        if self.at(&TokenKind::Import) {
            return self.import_decl();
        }
        if self.at(&TokenKind::Export) {
            return self.export_decl();
        }
        if self.at(&TokenKind::Native) {
            return Err(Diagnostic::new(
                Code::E108,
                "bare `native \"lib\" { ... }` blocks are removed; declare foreign functions with `import { fn ... } from native \"lib\"`",
            )
            .with_span(self.peek_span())
            .with_hint("use `import { fn name(params): Ret } from native \"lib\"`"));
        }
        let attrs = self.outer_attrs()?;
        let access = self.access();
        match self.peek().clone() {
            TokenKind::Class => {
                self.bump();
                let (name, _) = self.ident()?;
                let type_params = self.type_params()?;
                let extends = if self.eat(&TokenKind::Extends) {
                    Some(self.ty()?)
                } else {
                    None
                };
                let mut with = Vec::new();
                if self.eat(&TokenKind::With) {
                    loop {
                        with.push(self.ty()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                if self.eat(&TokenKind::Colon) {
                    loop {
                        with.push(self.ty()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                }
                self.expect(&TokenKind::LBrace, "`{`")?;
                let mut members = Vec::new();
                while !self.at(&TokenKind::RBrace) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated class")
                            .with_span(self.peek_span()));
                    }
                    let pending = self.doc_prefix();
                    let save = self.pos;
                    let s = self.peek_span().start;
                    match self.class_member() {
                        Ok(mut m) => {
                            Self::attach_member_docs(&mut m, &pending);
                            let e = self.peek_span().start;
                            members.push(sp(m, Span { start: s, end: e }));
                        }
                        Err(e) => {
                            self.errors.push(e);
                            self.synchronize(Self::is_member_start);
                            if self.pos == save {
                                self.bump();
                            }
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                Ok(Decl::Class {
                    docs: String::new(),
                    access,
                    name,
                    type_params,
                    extends,
                    with,
                    members,
                })
            }
            TokenKind::Struct => {
                self.bump();
                let (name, _) = self.ident()?;
                let type_params = self.type_params()?;
                self.expect(&TokenKind::LBrace, "`{`")?;
                let mut members = Vec::new();
                while !self.at(&TokenKind::RBrace) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated struct")
                            .with_span(self.peek_span()));
                    }
                    let pending = self.doc_prefix();
                    let save = self.pos;
                    let s = self.peek_span().start;
                    match self.class_member() {
                        Ok(mut m) => {
                            Self::attach_member_docs(&mut m, &pending);
                            let e = self.peek_span().start;
                            members.push(sp(m, Span { start: s, end: e }));
                        }
                        Err(e) => {
                            self.errors.push(e);
                            self.synchronize(Self::is_member_start);
                            if self.pos == save {
                                self.bump();
                            }
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                let _ = attrs;
                Ok(Decl::Struct {
                    docs: String::new(),
                    access,
                    name,
                    type_params,
                    members,
                })
            }
            TokenKind::Record => {
                self.bump();
                let (name, _) = self.ident()?;
                let type_params = self.type_params()?;
                self.expect(&TokenKind::LParen, "`(`")?;
                let mut fields = Vec::new();
                while !self.at(&TokenKind::RParen) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated record")
                            .with_span(self.peek_span()));
                    }
                    let (fname, _) = self.ident()?;
                    self.expect(&TokenKind::Colon, "`:`")?;
                    let fty = self.ty()?;
                    fields.push(RecordField { name: fname, ty: fty });
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                let _ = attrs;
                Ok(Decl::Record {
                    docs: String::new(),
                    access,
                    name,
                    type_params,
                    fields,
                })
            }
            TokenKind::Extension => {
                self.bump();
                let target = self.ty()?;
                self.expect(&TokenKind::LBrace, "`{`")?;
                let mut members = Vec::new();
                while !self.at(&TokenKind::RBrace) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated extension")
                            .with_span(self.peek_span()));
                    }
                    let pending = self.doc_prefix();
                    let save = self.pos;
                    let s = self.peek_span().start;
                    match self.class_member() {
                        Ok(mut m) => {
                            match &m {
                                ClassMember::Method(_) => {}
                                _ => {
                                    self.errors.push(
                                        Diagnostic::new(Code::E108, "extensions hold methods only")
                                            .with_span(self.peek_span()),
                                    );
                                    self.synchronize(Self::is_member_start);
                                    if self.pos == save {
                                        self.bump();
                                    }
                                    continue;
                                }
                            }
                            Self::attach_member_docs(&mut m, &pending);
                            let e = self.peek_span().start;
                            members.push(sp(m, Span { start: s, end: e }));
                        }
                        Err(e) => {
                            self.errors.push(e);
                            self.synchronize(Self::is_member_start);
                            if self.pos == save {
                                self.bump();
                            }
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                let _ = attrs;
                Ok(Decl::Extension {
                    docs: String::new(),
                    access,
                    target,
                    members,
                })
            }
            TokenKind::Interface => {                self.bump();
                let (name, _) = self.ident()?;
                let itype_params = self.type_params()?;
                self.expect(&TokenKind::LBrace, "`{`")?;
                let mut members = Vec::new();
                while !self.at(&TokenKind::RBrace) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated interface")
                            .with_span(self.peek_span()));
                    }
                    let pending = self.doc_prefix();
                    let s = self.peek_span().start;
                    match self.peek().clone() {
                        TokenKind::Fn => {
                            self.bump();
                            let (mname, _) = self.ident()?;
                            let mtype_params = self.type_params()?;
                            let params = self.params()?;
                            let ret = if self.eat(&TokenKind::Colon) {
                                Some(self.ty()?)
                            } else {
                                None
                            };
                            let throws = self.eat(&TokenKind::Throws);
                            self.expect(&TokenKind::Semi, "`;`")?;
                            let e = self.peek_span().start;
                            members.push(sp(
                                ClassMember::Method(FnDecl {
                                    docs: pending,
                                    access: Visibility::Internal,
                                    name: mname,
                                    type_params: mtype_params,
                                    params,
                                    ret,
                                    throws,
                                    is_unsafe: false,
                                    is_async: false,
                                    is_static: false,
                                    is_test: false,
                                    is_bench: false,
                                    body: FnBody::Block(Block { stmts: Vec::new() }),
                                    attrs: Vec::new(),
                                }),
                                Span { start: s, end: e },
                            ));
                        }
                        _ => {
                            self.errors.push(
                                Diagnostic::new(Code::E108, "interfaces hold method signatures only")
                                    .with_span(self.peek_span()),
                            );
                            self.synchronize(Self::is_member_start);
                            if self.at(&TokenKind::RBrace) {
                                continue;
                            }
                            self.bump();
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                let _ = attrs;
                Ok(Decl::Interface {
                    docs: String::new(),
                    access,
                    name,
                    type_params: itype_params,
                    members,
                })
            }
            TokenKind::Trait => {
                self.bump();
                let (name, _) = self.ident()?;
                self.expect(&TokenKind::LBrace, "`{`")?;
                let mut members = Vec::new();
                while !self.at(&TokenKind::RBrace) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated trait")
                            .with_span(self.peek_span()));
                    }
                    let pending = self.doc_prefix();
                    let save = self.pos;
                    let s = self.peek_span().start;
                    match self.class_member() {
                        Ok(mut m) => {
                            Self::attach_member_docs(&mut m, &pending);
                            let e = self.peek_span().start;
                            members.push(sp(m, Span { start: s, end: e }));
                        }
                        Err(e) => {
                            self.errors.push(e);
                            self.synchronize(Self::is_member_start);
                            if self.pos == save {
                                self.bump();
                            }
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                let _ = attrs;
                Ok(Decl::Trait {
                    docs: String::new(),
                    access,
                    name,
                    members,
                })
            }
            TokenKind::Enum => {
                self.bump();
                let (name, _) = self.ident()?;
                let type_params = self.type_params()?;
                self.expect(&TokenKind::LBrace, "`{`")?;
                let mut members = Vec::new();
                while !self.at(&TokenKind::RBrace) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated enum")
                            .with_span(self.peek_span()));
                    }
                    let save = self.pos;
                    match self.enum_member() {
                        Ok(m) => members.push(m),
                        Err(e) => {
                            self.errors.push(e);
                            self.synchronize(Self::is_enum_start);
                            if self.pos == save {
                                self.bump();
                            }
                        }
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                let _ = attrs;
                Ok(Decl::Enum {
                    docs: String::new(),
                    access,
                    name,
                    type_params,
                    members,
                })
            }
            TokenKind::Fn => {
                let f = self.fn_decl(access, attrs)?;
                Ok(Decl::Fn(f))
            }
            TokenKind::Ident(s) if s == "test" => {
                self.bump();
                if !self.at(&TokenKind::Fn) {
                    return Err(Diagnostic::new(Code::E108, "expected `fn` after `test`")
                        .with_span(self.peek_span()));
                }
                let mut f = self.fn_decl(access, attrs)?;
                f.is_test = true;
                Ok(Decl::Fn(f))
            }
            TokenKind::Ident(s) if s == "bench" => {
                self.bump();
                let raw = self.plain_string()?;
                let san: String = raw
                    .chars()
                    .map(|c| if c.is_ascii_alphanumeric() { c } else { '_' })
                    .collect();
                if san.trim_matches('_').is_empty() {
                    return Err(Diagnostic::new(Code::E108, "bench needs a name")
                        .with_span(self.peek_span()));
                }
                let body = FnBody::Block(self.block()?);
                Ok(Decl::Fn(FnDecl {
                    docs: String::new(),
                    access: Visibility::Internal,
                    name: format!("__rnx_bench_{san}"),
                    type_params: Vec::new(),
                    params: Vec::new(),
                    ret: None,
                    throws: false,
                    is_unsafe: false,
                    is_async: false,
                    is_static: false,
                    is_test: false,
                    is_bench: true,
                    body,
                    attrs,
                }))
            }
            TokenKind::Unsafe => {
                if self.pos + 1 < self.toks.len()
                    && !matches!(self.toks[self.pos + 1].kind, TokenKind::Fn)
                {
                    let save = self.pos;
                    let s = self.peek_span().start;
                    match self.stmt() {
                        Ok(st) => {
                            let e = self.peek_span().start;
                            let _ = attrs;
                            return Ok(Decl::Stmt(Box::new(sp(st, Span { start: s, end: e }))));
                        }
                        Err(_) => {
                            self.pos = save;
                        }
                    }
                }
                self.bump();
                let mut f = self.fn_decl(access, attrs)?;
                f.is_unsafe = true;
                Ok(Decl::Fn(f))
            }
            TokenKind::Async => {
                self.bump();
                let mut f = self.fn_decl(access, attrs)?;
                f.is_async = true;
                Ok(Decl::Fn(f))
            }
            TokenKind::Const => {
                let s = self.peek_span().start;
                self.bump();
                let (name, _) = self.ident()?;
                let ty = if self.eat(&TokenKind::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                if !self.at(&TokenKind::Eq) {
                    let span = self.peek_span();
                    self.eat_semi();
                    self.errors.push(
                        Diagnostic::new(
                            Code::E206,
                            "variable declaration must be initialized with an expression",
                        )
                        .with_span(span)
                        .with_hint("expected `=` followed by an initializer"),
                    );
                    let e = self.peek_span().start;
                    return Ok(Decl::Stmt(Box::new(sp(
                        Stmt::Empty,
                        Span { start: s, end: e },
                    ))));
                }
                self.expect(&TokenKind::Eq, "`=`")?;
                let vs = self.peek_span().start;
                let v = self.expr()?;
                let ve = self.peek_span().start;
                self.eat_semi();
                Ok(Decl::Const {
                    docs: String::new(),
                    access,
                    name,
                    ty,
                    value: sp(v, Span { start: vs, end: ve }),
                })
            }
            TokenKind::Let => self.let_decl_as_decl(),
            _ => {
                let save = self.pos;
                let s = self.peek_span().start;
                match self.stmt() {
                    Ok(st) => {
                        let e = self.peek_span().start;
                        let _ = attrs;
                        Ok(Decl::Stmt(Box::new(sp(st, Span { start: s, end: e }))))
                    }
                    Err(e) => {
                        let committed = matches!(
                            self.toks.get(save).map(|t| &t.kind),
                            Some(TokenKind::Let) | Some(TokenKind::Const)
                        ) || (matches!(
                            self.toks.get(save).map(|t| &t.kind),
                            Some(TokenKind::Static)
                        ) && matches!(
                            self.toks.get(save + 1).map(|t| &t.kind),
                            Some(TokenKind::Let) | Some(TokenKind::Const)
                        ));
                        if committed {
                            if e.code == Code::E206 {
                                self.errors.push(e);
                                self.eat_semi();
                                let epos = self.peek_span().start;
                                return Ok(Decl::Stmt(Box::new(sp(
                                    Stmt::Empty,
                                    Span { start: s, end: epos },
                                ))));
                            }
                            return Err(e);
                        }
                        self.pos = save;
                        Err(Diagnostic::new(
                            Code::E108,
                            format!("expected declaration, found {}", describe(self.peek())),
                        )
                        .with_span(self.peek_span()))
                    }
                }
            }
        }
    }

    fn import_source(&mut self) -> PR<(ImportSource, Span)> {
        let str_start = self.peek_span().start;
        if self.eat(&TokenKind::Native) {
            let lib = self.plain_string()?;
            let span = Span { start: str_start, end: self.peek_span().start };
            return Ok((ImportSource::Native(lib), span));
        }
        match self.peek().clone() {
            TokenKind::StrOpen => {
                let path = self.plain_string()?;
                let span = Span { start: str_start, end: self.peek_span().start };
                Ok((ImportSource::Module(path), span))
            }
            _ => Err(Diagnostic::new(Code::E108, "expected module string or `native \"lib\"`")
                .with_span(self.peek_span())),
        }
    }

    fn import_decl(&mut self) -> PR<Decl> {
        self.expect(&TokenKind::Import, "`import`")?;
        if self.at(&TokenKind::StrOpen) {
            let (source, source_span) = self.import_source()?;
            if !matches!(source, ImportSource::Module(_)) {
                return Err(Diagnostic::new(
                    Code::E108,
                    "side-effect import requires a module path; native imports need `{ fn ... }` specifiers",
                )
                .with_span(source_span));
            }
            self.eat_semi();
            return Ok(Decl::Import(ImportDecl {
                clause: ImportClause::SideEffect,
                source,
                source_span,
            }));
        }
        if self.at(&TokenKind::Native) || self.at(&TokenKind::From) {
            return Err(Diagnostic::new(
                Code::E108,
                "expected `{` with function declarations after `import`",
            )
            .with_span(self.peek_span())
            .with_hint("use `import { fn name(params): Ret } from native \"lib\"`"));
        }
        let clause: Option<ImportClause>;
        if self.eat(&TokenKind::Star) {
            if self.eat(&TokenKind::As) {
                let (n, _) = self.ident()?;
                clause = Some(ImportClause::Namespace(n));
            } else {
                clause = Some(ImportClause::Star);
            }
        } else if self.eat(&TokenKind::LBrace) {
            let specs = self.import_named_list(false)?;
            self.expect(&TokenKind::RBrace, "`}`")?;
            clause = Some(ImportClause::Named(specs));
        } else {
            let (n, _) = self.ident()?;
            if self.eat(&TokenKind::Comma) {
                if self.eat(&TokenKind::LBrace) {
                    let specs = self.import_named_list(false)?;
                    self.expect(&TokenKind::RBrace, "`}`")?;
                    clause = Some(ImportClause::DefaultAndNamed(n, specs));
                } else {
                    return Err(Diagnostic::new(Code::E108, "expected `{...}` after default import")
                        .with_span(self.peek_span()));
                }
            } else {
                clause = Some(ImportClause::Default(n));
            }
        }
        self.expect(&TokenKind::From, "`from`")?;
        let (source, source_span) = self.import_source()?;
        let clause = clause.unwrap_or(ImportClause::SideEffect);
        if matches!(source, ImportSource::Native(_)) {
            match clause {
                ImportClause::Named(specs) => {
                    let native_specs = self.reparse_native_specs(&specs, &source_span)?;
                    self.eat_semi();
                    return Ok(Decl::Import(ImportDecl {
                        clause: ImportClause::Named(native_specs),
                        source,
                        source_span,
                    }));
                }
                ImportClause::DefaultAndNamed(d, specs) => {
                    let native_specs = self.reparse_native_specs(&specs, &source_span)?;
                    self.eat_semi();
                    return Ok(Decl::Import(ImportDecl {
                        clause: ImportClause::DefaultAndNamed(d, native_specs),
                        source,
                        source_span,
                    }));
                }
                _ => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        "native imports require `{ fn ... }` specifiers",
                    )
                    .with_span(source_span)
                    .with_hint(
                        "use `import { fn name(params): Ret } from native \"lib\"`",
                    ));
                }
            }
        }
        match &clause {
            ImportClause::Named(specs) | ImportClause::DefaultAndNamed(_, specs)
                if specs.iter().any(|s| s.native_fn.is_some()) =>
            {
                return Err(Diagnostic::new(
                    Code::E108,
                    "type annotations are forbidden on module imports; use `from native` for foreign C ABI",
                )
                .with_span(source_span)
                .with_hint("use `import { fn ... } from native \"lib\"` for C functions"));
            }
            _ => {}
        }
        self.eat_semi();
        Ok(Decl::Import(ImportDecl { clause, source, source_span }))
    }

    fn reparse_native_specs(
        &mut self,
        specs: &[ImportSpecifier],
        source_span: &Span,
    ) -> PR<Vec<ImportSpecifier>> {
        if specs.iter().all(|s| s.native_fn.is_some()) {
            return Ok(specs.to_vec());
        }
        Err(Diagnostic::new(
            Code::E108,
            "native import specifiers must be function declarations (`fn name(params): Ret`)",
        )
        .with_span(*source_span))
    }

    fn import_named_list(&mut self, _native: bool) -> PR<Vec<ImportSpecifier>> {
        let mut out = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated import list")
                    .with_span(self.peek_span()));
            }
            let is_export = self.eat(&TokenKind::Export);
            if self.at(&TokenKind::Fn) {
                self.bump();
                let (name, _) = self.ident()?;
                let params = self.params()?;
                let ret = if self.eat(&TokenKind::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                let alias = if self.eat(&TokenKind::As) {
                    let (a, _) = self.ident()?;
                    Some(a)
                } else {
                    None
                };
                out.push(ImportSpecifier {
                    name: name.clone(),
                    alias: None,
                    is_export,
                    native_fn: Some(NativeFnSig { name, params, ret, alias }),
                });
            } else {
                let (n, _) = self.ident()?;
                let alias = if self.eat(&TokenKind::As) {
                    let (a, _) = self.ident()?;
                    Some(a)
                } else {
                    None
                };
                out.push(ImportSpecifier { name: n, alias, is_export, native_fn: None });
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        Ok(out)
    }

    fn export_decl(&mut self) -> PR<Decl> {
        let start = self.peek_span().start;
        self.expect(&TokenKind::Export, "`export`")?;
        if self.eat(&TokenKind::Default) {
            return self.export_default(start);
        }
        if self.eat(&TokenKind::Star) {
            let alias = if self.eat(&TokenKind::As) {
                let (a, _) = self.ident()?;
                Some(a)
            } else {
                None
            };
            self.expect(&TokenKind::From, "`from`")?;
            let (source, source_span) = self.import_source()?;
            if matches!(source, ImportSource::Native(_)) {
                return Err(Diagnostic::new(
                    Code::E108,
                    "`export *` cannot re-export a native library; list functions explicitly",
                )
                .with_span(source_span)
                .with_hint("use `export { fn name(params): Ret } from native \"lib\"`"));
            }
            self.eat_semi();
            return Ok(Decl::ExportFrom(ExportFromDecl {
                clause: ExportClause::All { alias },
                source,
                source_span,
            }));
        }
        if self.eat(&TokenKind::LBrace) {
            let mut specs = Vec::new();
            let mut native_specs = Vec::new();
            let mut saw_fn = false;
            let mut saw_plain = false;
            while !self.at(&TokenKind::RBrace) {
                if self.at(&TokenKind::Eof) {
                    return Err(Diagnostic::new(Code::E108, "unterminated export list")
                        .with_span(self.peek_span()));
                }
                if self.at(&TokenKind::Fn) {
                    self.bump();
                    saw_fn = true;
                    let (name, _) = self.ident()?;
                    let params = self.params()?;
                    let ret = if self.eat(&TokenKind::Colon) {
                        Some(self.ty()?)
                    } else {
                        None
                    };
                    let alias = if self.eat(&TokenKind::As) {
                        let (a, _) = self.ident()?;
                        Some(a)
                    } else {
                        None
                    };
                    native_specs.push(NativeFnSig { name, params, ret, alias });
                } else {
                    saw_plain = true;
                    let (n, _) = self.ident()?;
                    let alias = if self.eat(&TokenKind::As) {
                        let (a, _) = self.ident()?;
                        Some(a)
                    } else {
                        None
                    };
                    specs.push(ExportSpecifier { name: n, alias });
                }
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RBrace, "`}`")?;
            if self.eat(&TokenKind::From) {
                let (source, source_span) = self.import_source()?;
                if matches!(source, ImportSource::Native(_)) {
                    if saw_plain || native_specs.is_empty() {
                        return Err(Diagnostic::new(
                            Code::E108,
                            "native re-export specifiers must be function declarations (`fn name(params): Ret`)",
                        )
                        .with_span(source_span));
                    }
                    self.eat_semi();
                    return Ok(Decl::ExportFrom(ExportFromDecl {
                        clause: ExportClause::Native(native_specs),
                        source,
                        source_span,
                    }));
                }
                if saw_fn {
                    return Err(Diagnostic::new(
                        Code::E108,
                        "type annotations are forbidden on module re-exports; use `from native` for foreign C ABI",
                    )
                    .with_span(source_span)
                    .with_hint("use `export { fn ... } from native \"lib\"` for C functions"));
                }
                self.eat_semi();
                return Ok(Decl::ExportFrom(ExportFromDecl {
                    clause: ExportClause::Named(specs),
                    source,
                    source_span,
                }));
            }
            if saw_fn {
                return Err(Diagnostic::new(
                    Code::E108,
                    "function signatures in `export { ... }` require a `from` source",
                )
                .with_span(self.peek_span())
                .with_hint("use `export { fn ... } from native \"lib\"` for C functions"));
            }
            let _ = saw_plain;
            self.eat_semi();
            return Ok(Decl::ExportList(specs));
        }
        let attrs: Vec<Attr> = Vec::new();
        self.export_item(attrs)
    }

    fn export_default(&mut self, start: u32) -> PR<Decl> {
        match self.peek().clone() {
            TokenKind::Fn
            | TokenKind::Class
            | TokenKind::Struct
            | TokenKind::Record
            | TokenKind::Trait
            | TokenKind::Interface
            | TokenKind::Enum
            | TokenKind::Const
            | TokenKind::Static
            | TokenKind::Async
            | TokenKind::Unsafe => {
                let attrs: Vec<Attr> = Vec::new();
                self.export_item(attrs)
            }
            _ => {
                let s = self.peek_span().start;
                let e = self.expr()?;
                let en = self.peek_span().start;
                self.eat_semi();
                Ok(Decl::ExportDefault(ExportDefaultDecl {
                    expr: sp(e, Span { start: s, end: en }),
                    span: Span { start, end: en },
                }))
            }
        }
    }

    fn export_item(&mut self, _attrs: Vec<Attr>) -> PR<Decl> {
        let mut d = self.decl()?;
        match &mut d {
            Decl::Fn(f) => f.access = Visibility::Export,
            Decl::Class { access, .. }
            | Decl::Struct { access, .. }
            | Decl::Trait { access, .. }
            | Decl::Interface { access, .. }
            | Decl::Extension { access, .. }
            | Decl::Enum { access, .. }
            | Decl::Record { access, .. }
            | Decl::Const { access, .. } => *access = Visibility::Export,
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    "only functions, types, and constants can follow `export`",
                )
                .with_span(self.peek_span()));
            }
        }
        Ok(d)
    }

    fn plain_string(&mut self) -> PR<String> {
        self.expect(&TokenKind::StrOpen, "string")?;
        let mut out = String::new();
        loop {
            match self.peek().clone() {
                TokenKind::StrText(t) => {
                    out.push_str(&t);
                    self.bump();
                }
                TokenKind::StrClose => {
                    self.bump();
                    break;
                }
                _ => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        "interpolation not allowed here",
                    )
                    .with_span(self.peek_span()));
                }
            }
        }
        Ok(out)
    }

    fn class_member(&mut self) -> PR<ClassMember> {
        let attrs = self.outer_attrs()?;
        let access = self.access();
        let is_static = self.eat(&TokenKind::Static);
        match self.peek().clone() {
            TokenKind::Init => {
                if is_static {
                    return Err(Diagnostic::new(Code::E108, "`static init` is not supported")
                        .with_span(self.peek_span()));
                }
                self.bump();
                let params = self.params()?;
                let body = self.block()?;
                Ok(ClassMember::Init { params, body })
            }
            TokenKind::Deinit => {
                self.bump();
                let body = self.block()?;
                Ok(ClassMember::Deinit(body))
            }
            TokenKind::OnReload => {
                self.bump();
                let params = self.params()?;
                let body = self.block()?;
                Ok(ClassMember::OnReload { params, body })
            }
            TokenKind::Fn => {
                let mut f = self.fn_decl(Access::Internal, attrs)?;
                if access != Access::Internal {
                    f.access = access;
                }
                f.is_static = is_static;
                Ok(ClassMember::Method(f))
            }
            TokenKind::Unsafe => {
                self.bump();
                let mut f = self.fn_decl(Access::Internal, attrs)?;
                if access != Access::Internal {
                    f.access = access;
                }
                f.is_unsafe = true;
                Ok(ClassMember::Method(f))
            }
            TokenKind::Let | TokenKind::Const => {
                let mut f = self.field_decl()?;
                if access != Access::Internal {
                    f.access = access;
                }
                f.attrs = attrs;
                Ok(ClassMember::Field(f))
            }
            TokenKind::Ident(_) => {
                let (name, name_span) = self.ident()?;
                if name == "constructor"
                    && self.toks.get(self.pos).map(|t| &t.kind) == Some(&TokenKind::LParen)
                {
                    return Err(Diagnostic::new(
                        Code::E108,
                        "classes use `init` for initialization, not `constructor`",
                    )
                    .with_span(name_span)
                    .with_hint("rename it to `init`"));
                }
                let mtype_params = self.type_params()?;
                let params = self.params()?;
                let ret = if self.eat(&TokenKind::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                let throws = self.eat(&TokenKind::Throws);
                let body = if self.at(&TokenKind::LBrace) {
                    FnBody::Block(self.block()?)
                } else if self.at(&TokenKind::FatArrow) {
                    let span = self.peek_span();
                    self.skip_arrow_body();
                    return Err(Diagnostic::new(Code::E105, "invalid lambda syntax")
                        .with_span(span)
                        .with_hint("named functions require a block body. Use `fn mul(x) { ... }`; for lambdas use `(x) => x * 2`."));
                } else {
                    return Err(Diagnostic::new(Code::E108, "expected method body")
                        .with_span(self.peek_span()));
                };
                Ok(ClassMember::Method(FnDecl {
                    docs: String::new(),
                    access,
                    name,
                    type_params: mtype_params,
                    params,
                    ret,
                    throws,
                    is_unsafe: false,
                    is_async: false,
                    is_static,
                    is_test: false,
                    is_bench: false,
                    body,
                    attrs,
                }))
            }
            _ => Err(Diagnostic::new(
                Code::E108,
                format!("expected class member, found {}", describe(self.peek())),
            )
            .with_span(self.peek_span())),
        }
    }

    fn field_decl(&mut self) -> PR<FieldDecl> {
        let mutable = if self.eat(&TokenKind::Let) {
            true
        } else if self.eat(&TokenKind::Const) {
            false
        } else {
            return Err(Diagnostic::new(Code::E108, "expected `let` or `const`")
                .with_span(self.peek_span()));
        };
        let (name, _) = self.ident()?;
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        let value = if self.eat(&TokenKind::Eq) {
            let s = self.peek_span().start;
            let v = self.expr()?;
            let e = self.peek_span().start;
            Some(sp(v, Span { start: s, end: e }))
        } else {
            None
        };
        self.eat_semi();
        Ok(FieldDecl {
            docs: String::new(),
            access: Visibility::Internal,
            mutable,
            name,
            ty,
            value,
            attrs: Vec::new(),
        })
    }

    fn fn_decl(&mut self, access: Access, attrs: Vec<Attr>) -> PR<FnDecl> {
        self.expect(&TokenKind::Fn, "`fn`")?;
        let (name, _) = self.ident()?;
        let type_params = self.type_params()?;
        let params = self.params()?;
        let ret = if self.eat(&TokenKind::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        let throws = self.eat(&TokenKind::Throws);
        let body = if self.at(&TokenKind::LBrace) {
            FnBody::Block(self.block()?)
        } else if self.at(&TokenKind::FatArrow) {
            let span = self.peek_span();
            self.skip_arrow_body();
            return Err(Diagnostic::new(Code::E105, "invalid lambda syntax")
                .with_span(span)
                .with_hint("named functions require a block body. Use `fn mul(x) { ... }`; for lambdas use `(x) => x * 2`."));
        } else {
            return Err(Diagnostic::new(Code::E108, "expected `{`")
                .with_span(self.peek_span()));
        };
        Ok(FnDecl {
            docs: String::new(),
            access,
            name,
            type_params,
            params,
            ret,
            throws,
            is_unsafe: false,
            is_async: false,
                    is_static: false,
            is_test: false,
            is_bench: false,
            body,
            attrs,
        })
    }

    fn type_params(&mut self) -> PR<Vec<String>> {
        let mut out = Vec::new();
        if !self.eat(&TokenKind::Lt) {
            return Ok(out);
        }
        loop {
            let (n, _) = self.ident()?;
            out.push(n);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::Gt, "`>`")?;
        Ok(out)
    }

    fn params(&mut self) -> PR<Vec<Param>> {
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut out = Vec::new();
        while !self.at(&TokenKind::RParen) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated params")
                    .with_span(self.peek_span()));
            }
            let promote = self.at(&TokenKind::Public) || self.at(&TokenKind::Private);
            if promote {
                self.bump();
            }
            let (name, span) = self.ident()?;
            let ty = if self.eat(&TokenKind::Colon) {
                Some(self.ty()?)
            } else {
                None
            };
            let default = if self.eat(&TokenKind::Eq) {
                let s = self.peek_span().start;
                let e = self.expr()?;
                let en = self.peek_span().start;
                Some(sp(e, Span { start: s, end: en }))
            } else {
                None
            };
            out.push(Param {
                name,
                span,
                ty,
                default,
                promote,
            });
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        Ok(out)
    }

    fn ty(&mut self) -> PR<Type> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(Diagnostic::new(Code::E108, "type nesting is too deep")
                .with_span(self.peek_span()));
        }
        self.depth += 1;
        let out = self.ty_inner();
        self.depth -= 1;
        out
    }

    fn ty_inner(&mut self) -> PR<Type> {
        if self.at(&TokenKind::LParen) {
            self.bump();
            if self.at(&TokenKind::RParen) {
                return Err(Diagnostic::new(Code::E108, "empty tuple type")
                    .with_span(self.peek_span()));
            }
            let first = self.ty()?;
            if !self.eat(&TokenKind::Comma) {
                self.expect(&TokenKind::RParen, "`)`")?;
                return Ok(first);
            }
            let mut items = vec![first];
            loop {
                items.push(self.ty()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
            return Ok(Type {
                path: Vec::new(),
                args: Vec::new(),
                nullable: false,
                fn_sig: None,
                tuple: items,
            });
        }
        if self.eat(&TokenKind::Fn) {
            self.expect(&TokenKind::LParen, "`(`")?;
            let mut params = Vec::new();
            while !self.at(&TokenKind::RParen) {
                params.push(self.ty()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.expect(&TokenKind::RParen, "`)`")?;
            let ret = if self.eat(&TokenKind::Colon) {
                Some(Box::new(self.ty()?))
            } else {
                None
            };
            return Ok(Type {
                path: vec!["fn".to_string()],
                args: Vec::new(),
                nullable: false,
                fn_sig: Some(FnType { params, ret }),
                tuple: Vec::new(),
            });
        }
        let (first, _) = self.ident()?;
        let mut path = vec![first];
        while self.eat(&TokenKind::Dot) {
            let (n, _) = self.ident()?;
            path.push(n);
        }
        let mut args = Vec::new();
        if self.eat(&TokenKind::Lt) {
            loop {
                args.push(self.ty()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.close_angle()?;
        }
        let mut nullable = false;
        while self.eat(&TokenKind::Question) {
            nullable = true;
        }
        Ok(Type {
            path,
            args,
            nullable,
            fn_sig: None,
            tuple: Vec::new(),
        })
    }

    fn block(&mut self) -> PR<Block> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(Diagnostic::new(Code::E108, "block nesting is too deep")
                .with_span(self.peek_span()));
        }
        self.depth += 1;
        let out = self.block_inner();
        self.depth -= 1;
        out
    }

    fn block_inner(&mut self) -> PR<Block> {
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut stmts = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated block")
                    .with_span(self.peek_span()));
            }
            let save = self.pos;
            let s = self.peek_span().start;
            match self.stmt() {
                Ok(st) => {
                    let e = self.peek_span().start;
                    stmts.push(sp(st, Span { start: s, end: e }));
                }
                Err(e) => {
                    self.errors.push(e);
                    self.synchronize(Self::is_stmt_start);
                    if self.pos == save {
                        self.bump();
                    }
                }
            }
        }
        self.expect(&TokenKind::RBrace, "`}`")?;
        Ok(Block { stmts })
    }

    fn destructure_tuple(&mut self) -> PR<Stmt> {
        self.expect(&TokenKind::LParen, "`(`")?;
        let mut names = Vec::new();
        while !self.at(&TokenKind::RParen) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated tuple pattern")
                    .with_span(self.peek_span()));
            }
            let (n, _) = self.ident()?;
            names.push(n);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        {
            let mut seen = std::collections::BTreeSet::new();
            for n in &names {
                if !seen.insert(n.clone()) {
                    return Err(Diagnostic::new(Code::E108, format!("duplicate binding `{n}` in tuple pattern"))
                        .with_span(self.peek_span()));
                }
            }
        }
        if names.len() < 2 {
            return Err(Diagnostic::new(Code::E108, "tuple pattern needs at least 2 names")
                .with_span(self.peek_span()));
        }
        let ty = if self.eat(&TokenKind::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect(&TokenKind::Eq, "`=`")?;
        let s = self.peek_span().start;
        let v = self.expr()?;
        let e = self.peek_span().start;
        self.eat_semi();
        Ok(Stmt::DestructureTuple { names, ty, value: sp(v, Span { start: s, end: e }) })
    }

    fn destructure_array(&mut self) -> PR<Stmt> {
        self.expect(&TokenKind::LBracket, "`[`")?;
        let mut names = Vec::new();
        let mut rest = None;
        while !self.at(&TokenKind::RBracket) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated array pattern")
                    .with_span(self.peek_span()));
            }
            if self.eat(&TokenKind::Ellipsis) {
                let (n, _) = self.ident()?;
                if rest.is_some() {
                    return Err(Diagnostic::new(Code::E108, "array pattern takes at most one `...rest`")
                        .with_span(self.peek_span()));
                }
                rest = Some(n);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if !self.at(&TokenKind::RBracket) {
                    return Err(Diagnostic::new(Code::E108, "`...rest` must be last in an array pattern")
                        .with_span(self.peek_span()));
                }
                break;
            }
            let (n, _) = self.ident()?;
            names.push(n);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBracket, "`]`")?;
        {
            let mut seen = std::collections::BTreeSet::new();
            for n in names.iter().chain(rest.iter()) {
                if !seen.insert(n.clone()) {
                    return Err(Diagnostic::new(Code::E108, format!("duplicate binding `{n}` in array pattern"))
                        .with_span(self.peek_span()));
                }
            }
        }
        if names.is_empty() && rest.is_none() {
            return Err(Diagnostic::new(Code::E108, "array pattern needs at least one binding")
                .with_span(self.peek_span()));
        }
        self.expect(&TokenKind::Eq, "`=`")?;
        let s = self.peek_span().start;
        let v = self.expr()?;
        let e = self.peek_span().start;
        self.eat_semi();
        Ok(Stmt::DestructureArray { names, rest, value: sp(v, Span { start: s, end: e }) })
    }

    fn destructure_record(&mut self) -> PR<Stmt> {
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut fields = Vec::new();
        let mut rest = None;
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated record pattern")
                    .with_span(self.peek_span()));
            }
            if self.eat(&TokenKind::Ellipsis) {
                let (n, _) = self.ident()?;
                if rest.is_some() {
                    return Err(Diagnostic::new(Code::E108, "record pattern takes at most one `...rest`")
                        .with_span(self.peek_span()));
                }
                rest = Some(n);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
                if !self.at(&TokenKind::RBrace) {
                    return Err(Diagnostic::new(Code::E108, "`...rest` must be last in a record pattern")
                        .with_span(self.peek_span()));
                }
                break;
            }
            let (field, _) = self.ident()?;
            let local = if self.eat(&TokenKind::Colon) {
                let (n, _) = self.ident()?;
                n
            } else {
                field.clone()
            };
            fields.push((field, local));
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, "`}`")?;
        {
            let mut seen = std::collections::BTreeSet::new();
            for local in fields.iter().map(|(_, l)| l).chain(rest.iter()) {
                if !seen.insert(local.clone()) {
                    return Err(Diagnostic::new(Code::E108, format!("duplicate binding `{local}` in record pattern"))
                        .with_span(self.peek_span()));
                }
            }
        }
        self.expect(&TokenKind::Eq, "`=`")?;
        let s = self.peek_span().start;
        let v = self.expr()?;
        let e = self.peek_span().start;
        self.eat_semi();
        Ok(Stmt::DestructureRecord { fields, rest, value: sp(v, Span { start: s, end: e }) })
    }

    fn single_or_block(&mut self) -> PR<Block> {
        if self.at(&TokenKind::LBrace) {
            return self.block();
        }
        let s = self.peek_span().start;
        let st = self.stmt()?;
        let e = self.peek_span().start;
        let span = Span { start: s, end: e };
        match &st {
            Stmt::Var { .. }
            | Stmt::DestructureTuple { .. }
            | Stmt::DestructureRecord { .. }
            | Stmt::DestructureArray { .. } => {
                return Err(Diagnostic::new(
                    Code::E108,
                    "declarations need `{ ... }` in single-statement `if` bodies",
                )
                .with_span(span));
            }
            _ => {}
        }
        Ok(Block {
            stmts: vec![sp(st, span)],
        })
    }

    fn stmt(&mut self) -> PR<Stmt> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(Diagnostic::new(Code::E108, "statement nesting is too deep")
                .with_span(self.peek_span()));
        }
        self.depth += 1;
        let out = self.stmt_inner();
        self.depth -= 1;
        out
    }

    fn stmt_inner(&mut self) -> PR<Stmt> {
        if self.at(&TokenKind::Pass) {
            self.bump();
            self.eat_semi();
            return Ok(Stmt::Pass);
        }
        match self.peek().clone() {
            TokenKind::Semi => {
                self.bump();
                Ok(Stmt::Empty)
            }
            TokenKind::Let | TokenKind::Const | TokenKind::Static => {
                let is_static = self.eat(&TokenKind::Static);
                let mutable = if !is_static && self.eat(&TokenKind::Let) {
                    true
                } else if self.eat(&TokenKind::Const) {
                    false
                } else if is_static && matches!(self.peek(), TokenKind::Let | TokenKind::Const) {
                    let m = matches!(self.peek(), TokenKind::Let);
                    self.bump();
                    m
                } else if is_static {
                    true
                } else {
                    return Err(Diagnostic::new(Code::E108, "expected `let`/`const`")
                        .with_span(self.peek_span()));
                };
                if self.at(&TokenKind::LParen) {
                    return self.destructure_tuple();
                }
                if self.at(&TokenKind::LBrace) {
                    return self.destructure_record();
                }
                if self.at(&TokenKind::LBracket) {
                    return self.destructure_array();
                }
                let (name, _) = self.ident()?;
                let ty =                 if self.eat(&TokenKind::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                if !self.at(&TokenKind::Eq) {
                    if mutable {
                        let span = self.peek_span();
                        self.eat_semi();
                        return Ok(Stmt::Var {
                            mutable,
                            is_static,
                            name,
                            ty,
                            value: sp(Expr::Null, span),
                        });
                    }
                    let span = self.peek_span();
                    self.eat_semi();
                    return Err(Diagnostic::new(
                        Code::E206,
                        "variable declaration must be initialized with an expression",
                    )
                    .with_span(span)
                    .with_hint("expected `=` followed by an initializer"));
                }
                self.expect(&TokenKind::Eq, "`=`")?;
                let s = self.peek_span().start;
                let v = self.expr()?;
                let e = self.peek_span().start;
                self.eat_semi();
                Ok(Stmt::Var {
                    mutable,
                    is_static,
                    name,
                    ty,
                    value: sp(v, Span { start: s, end: e }),
                })
            }
            TokenKind::If => {
                self.bump();
                let cond = if self.eat(&TokenKind::Let) {
                    return Err(Diagnostic::new(Code::E108, "`if let` was removed; compare against `null` (`if x != null`) or use `guard let ... else`")
                        .with_span(self.peek_span()));
                } else {
                    let s = self.peek_span().start;
                    let v = self.cond()?;
                    let e = self.peek_span().start;
                    IfCond::Expr(sp(v, Span { start: s, end: e }))
                };
                let then = self.single_or_block()?;
                let otherwise = if self.eat(&TokenKind::Else) {
                    if self.at(&TokenKind::If) {
                        let s = self.peek_span().start;
                        let st = self.stmt()?;
                        let e = self.peek_span().start;
                        Some(Else::If(Box::new(sp(st, Span { start: s, end: e }))))
                    } else {
                        Some(Else::Block(self.single_or_block()?))
                    }
                } else {
                    None
                };
                Ok(Stmt::If {
                    cond,
                    then,
                    otherwise,
                })
            }
            TokenKind::For => self.for_stmt(),
            TokenKind::While => {
                self.bump();
                let s = self.peek_span().start;
                let c = self.cond()?;
                let e = self.peek_span().start;
                let body = self.block()?;
                Ok(Stmt::While {
                    cond: sp(c, Span { start: s, end: e }),
                    body,
                })
            }
            TokenKind::Do => {
                self.bump();
                let body = self.block()?;
                self.expect(&TokenKind::While, "`while`")?;
                let s = self.peek_span().start;
                let c = self.expr()?;
                let e = self.peek_span().start;
                self.eat_semi();
                Ok(Stmt::DoWhile {
                    body,
                    cond: sp(c, Span { start: s, end: e }),
                })
            }
            TokenKind::Switch => {
                self.bump();
                let s = self.peek_span().start;
                let sc = self.cond()?;
                let e = self.peek_span().start;
                let scrutinee = sp(sc, Span { start: s, end: e });
                self.expect(&TokenKind::LBrace, "`{`")?;
                let mut cases = Vec::new();
                let mut default = None;
                loop {
                    if self.eat(&TokenKind::Case) {
                        let pattern = self.pattern()?;
                        let guard = if self.eat(&TokenKind::If) {
                            let s = self.peek_span().start;
                            let g = self.cond()?;
                            let e = self.peek_span().start;
                            Some(sp(g, Span { start: s, end: e }))
                        } else {
                            None
                        };
                        self.expect(&TokenKind::Colon, "`:`")?;
                        let mut body = Vec::new();
                        while !matches!(
                            self.peek(),
                            TokenKind::Case | TokenKind::Default | TokenKind::RBrace | TokenKind::Eof
                        ) {
                            let save = self.pos;
                            let s = self.peek_span().start;
                            match self.stmt() {
                                Ok(st) => {
                                    let e = self.peek_span().start;
                                    body.push(sp(st, Span { start: s, end: e }));
                                }
                                Err(e) => {
                                    self.errors.push(e);
                                    self.synchronize(Self::is_stmt_start);
                                    if self.pos == save {
                                        self.bump();
                                    }
                                }
                            }
                        }
                        cases.push(SwitchCase {
                            pattern,
                            guard,
                            body,
                        });
                    } else if self.eat(&TokenKind::Default) {
                        self.expect(&TokenKind::Colon, "`:`")?;
                        let mut body = Vec::new();
                        while !matches!(self.peek(), TokenKind::RBrace | TokenKind::Eof) {
                            let save = self.pos;
                            let s = self.peek_span().start;
                            match self.stmt() {
                                Ok(st) => {
                                    let e = self.peek_span().start;
                                    body.push(sp(st, Span { start: s, end: e }));
                                }
                                Err(e) => {
                                    self.errors.push(e);
                                    self.synchronize(Self::is_stmt_start);
                                    if self.pos == save {
                                        self.bump();
                                    }
                                }
                            }
                        }
                        default = Some(body);
                    } else {
                        break;
                    }
                }
                self.expect(&TokenKind::RBrace, "`}`")?;
                Ok(Stmt::Switch {
                    scrutinee,
                    cases,
                    default,
                })
            }
            TokenKind::Return => {
                self.bump();
                let v = if matches!(
                    self.peek(),
                    TokenKind::Semi
                        | TokenKind::RBrace
                        | TokenKind::Eof
                        | TokenKind::Case
                        | TokenKind::Default
                ) {
                    None
                } else {
                    let s = self.peek_span().start;
                    let e = self.expr()?;
                    let en = self.peek_span().start;
                    Some(sp(e, Span { start: s, end: en }))
                };
                self.eat_semi();
                Ok(Stmt::Return(v))
            }
            TokenKind::Break => {
                self.bump();
                self.eat_semi();
                Ok(Stmt::Break)
            }
            TokenKind::Continue => {
                self.bump();
                self.eat_semi();
                Ok(Stmt::Continue)
            }
            TokenKind::Fallthrough => {
                self.bump();
                self.eat_semi();
                Ok(Stmt::Fallthrough)
            }
            TokenKind::Defer => {
                self.bump();
                if self.at(&TokenKind::LBrace) {
                    Ok(Stmt::Defer(self.block()?))
                } else {
                    let s = self.peek_span().start;
                    let st = self.stmt()?;
                    let e = self.peek_span().start;
                    Ok(Stmt::Defer(Block {
                        stmts: vec![sp(st, Span { start: s, end: e })],
                    }))
                }
            }
            TokenKind::Unsafe => {
                self.bump();
                Ok(Stmt::UnsafeBlock(self.block()?))
            }
            TokenKind::Guard => {
                self.bump();
                self.expect(&TokenKind::Let, "`let`")?;
                let (name, _) = self.ident()?;
                self.expect(&TokenKind::Eq, "`=`")?;
                let s = self.peek_span().start;
                let v = self.expr()?;
                let e = self.peek_span().start;
                self.expect(&TokenKind::Else, "`else`")?;
                let otherwise = self.block()?;
                Ok(Stmt::Guard {
                    name,
                    value: sp(v, Span { start: s, end: e }),
                    otherwise,
                })
            }
            TokenKind::Try => {
                self.bump();
                let body = self.block()?;
                let catch = if self.eat(&TokenKind::Catch) {
                    self.expect(&TokenKind::LParen, "`(`")?;
                    let (id, _) = self.ident()?;
                    if self.at(&TokenKind::Colon) {
                        return Err(Diagnostic::new(
                            Code::E108,
                            "`catch` binds a bare identifier; narrow the value with `is` inside the block",
                        )
                        .with_span(self.peek_span()));
                    }
                    self.expect(&TokenKind::RParen, "`)`")?;
                    Some((id, self.block()?))
                } else {
                    None
                };
                let finally = if self.eat(&TokenKind::Finally) {
                    Some(self.block()?)
                } else {
                    None
                };
                if catch.is_none() && finally.is_none() {
                    return Err(Diagnostic::new(
                        Code::E108,
                        "`try` needs `catch` or `finally`",
                    )
                    .with_span(self.peek_span()));
                }
                Ok(Stmt::Try {
                    body,
                    catch,
                    finally,
                })
            }
            TokenKind::Throw => {
                self.bump();
                let v = if matches!(
                    self.peek(),
                    TokenKind::Semi
                        | TokenKind::RBrace
                        | TokenKind::Eof
                        | TokenKind::Case
                        | TokenKind::Default
                ) {
                    None
                } else {
                    let s = self.peek_span().start;
                    let e = self.expr()?;
                    let en = self.peek_span().start;
                    Some(sp(e, Span { start: s, end: en }))
                };
                self.eat_semi();
                Ok(Stmt::Throw(v))
            }
            _ => {
                let s = self.peek_span().start;
                let e = self.expr()?;
                let en = self.peek_span().start;
                let lhs = sp(e, Span { start: s, end: en });
                let op = match self.peek() {
                    TokenKind::Eq => AssignOp::Eq,
                    TokenKind::PlusEq => AssignOp::PlusEq,
                    TokenKind::MinusEq => AssignOp::MinusEq,
                    TokenKind::StarEq => AssignOp::StarEq,
                    TokenKind::SlashEq => AssignOp::SlashEq,
                    TokenKind::PercentEq => AssignOp::PercentEq,
                    TokenKind::ShlEq => AssignOp::ShlEq,
                    TokenKind::ShrEq => AssignOp::ShrEq,
                    TokenKind::ZshrEq => AssignOp::ZshrEq,
                    _ => {
                        self.eat_semi();
                        return Ok(Stmt::Expr(lhs));
                    }
                };
                self.bump();
                let s = self.peek_span().start;
                let v = self.expr()?;
                let en = self.peek_span().start;
                self.eat_semi();
                Ok(Stmt::Assign {
                    target: lhs,
                    op,
                    value: sp(v, Span { start: s, end: en }),
                })
            }
        }
    }

    fn for_stmt(&mut self) -> PR<Stmt> {
        self.expect(&TokenKind::For, "`for`")?;
        if self.at(&TokenKind::LParen) {
            let s = self.peek_span();
            let mut depth = 0usize;
            let mut j = self.pos;
            let mut saw_semi = false;
            while j < self.toks.len() {
                match &self.toks[j].kind {
                    TokenKind::LParen => depth += 1,
                    TokenKind::RParen => {
                        depth -= 1;
                        if depth == 0 {
                            break;
                        }
                    }
                    TokenKind::Semi if depth == 1 => {
                        saw_semi = true;
                        break;
                    }
                    TokenKind::Eof => break,
                    _ => {}
                }
                j += 1;
            }
            if saw_semi {
                return Err(Diagnostic::new(
                    Code::E110,
                    "C-style `for` loops are not supported",
                )
                .with_span(s)
                .with_hint("use `for x in range` or `while`/`do-while`"));
            }
        }
        let (binding, paren_single) = if self.eat(&TokenKind::LParen) {
            let mut names = Vec::new();
            loop {
                let (n, _) = self.ident()?;
                names.push(n);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            if names.len() == 1 && self.at(&TokenKind::In) {
                (ForBinding::One(names.pop().unwrap()), true)
            } else {
                self.expect(&TokenKind::RParen, "`)`")?;
                (ForBinding::Many(names), false)
            }
        } else {
            let (n, _) = self.ident()?;
            (ForBinding::One(n), false)
        };
        self.expect(&TokenKind::In, "`in`")?;
        let s = self.peek_span().start;
        let it = self.cond()?;
        let e = self.peek_span().start;
        if paren_single {
            self.eat(&TokenKind::RParen);
        }
        let body = self.block()?;
        Ok(Stmt::For {
            binding,
            iter: sp(it, Span { start: s, end: e }),
            body,
        })
    }

    fn pattern(&mut self) -> PR<Pattern> {
        if self.eat(&TokenKind::Is) {
            return Ok(Pattern::Is(self.ty()?));
        }
        if matches!(self.peek(), TokenKind::Dot) {
            let mut path = Vec::new();
            while self.eat(&TokenKind::Dot) {
                let (n, _) = self.ident()?;
                path.push(n);
            }
            let args = if self.eat(&TokenKind::LParen) {
                let mut a = Vec::new();
                while !self.at(&TokenKind::RParen) {
                    a.push(self.pattern()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                a
            } else {
                Vec::new()
            };
            return Ok(Pattern::Enum { path, args });
        }
        if matches!(self.peek(), TokenKind::Ident(_))
            && self.toks.get(self.pos + 1).map(|t| &t.kind) == Some(&TokenKind::Dot)
        {
            let mut path = Vec::new();
            let (n, _) = self.ident()?;
            path.push(n);
            while self.at(&TokenKind::Dot) {
                self.bump();
                let (n, _) = self.ident()?;
                path.push(n);
            }
            let args = if self.eat(&TokenKind::LParen) {
                let mut a = Vec::new();
                while !self.at(&TokenKind::RParen) {
                    a.push(self.pattern()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                a
            } else {
                Vec::new()
            };
            return Ok(Pattern::Enum { path, args });
        }
        let s = self.peek_span().start;
        let lo = self.add()?;
        let e = self.peek_span().start;
        let lo = sp(lo, Span { start: s, end: e });
        if self.eat(&TokenKind::DotDot) {
            let s = self.peek_span().start;
            let hi = self.add()?;
            let e = self.peek_span().start;
            return Ok(Pattern::Range {
                lo,
                hi: sp(hi, Span { start: s, end: e }),
                inclusive: false,
            });
        }
        if self.eat(&TokenKind::DotDotEq) {
            let s = self.peek_span().start;
            let hi = self.add()?;
            let e = self.peek_span().start;
            return Ok(Pattern::Range {
                lo,
                hi: sp(hi, Span { start: s, end: e }),
                inclusive: true,
            });
        }
        Ok(Pattern::Literal(lo))
    }

    fn cond(&mut self) -> PR<Expr> {
        let save = self.brace_ok;
        self.brace_ok = false;
        let e = self.expr();
        self.brace_ok = save;
        e
    }

    fn expr(&mut self) -> PR<Expr> {
        if self.depth >= MAX_PARSE_DEPTH {
            return Err(Diagnostic::new(Code::E108, "expression nesting is too deep")
                .with_span(self.peek_span()));
        }
        self.depth += 1;
        let out = self.assign();
        self.depth -= 1;
        out
    }

    fn assign(&mut self) -> PR<Expr> {
        self.ternary()
    }

    fn ternary(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let cond = self.nullish()?;
        let ce = self.peek_span().start;
        if self.eat(&TokenKind::Question) {
            let ts = self.peek_span().start;
            let then = self.expr()?;
            let te = self.peek_span().start;
            self.expect(&TokenKind::Colon, "`:`")?;
            let os = self.peek_span().start;
            let otherwise = self.expr()?;
            let oe = self.peek_span().start;
            return Ok(Expr::Ternary {
                cond: Box::new(sp(cond, Span { start: s, end: ce })),
                then: Box::new(sp(then, Span { start: ts, end: te })),
                otherwise: Box::new(sp(otherwise, Span { start: os, end: oe })),
            });
        }
        Ok(cond)
    }

    fn nullish(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.or()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        while self.eat(&TokenKind::QuestionQuestion) {
            let rs = self.peek_span().start;
            let rhs = self.or()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = Expr::Coalesce {
                lhs: Box::new(sp(lhs, lhs_span)),
                rhs: Box::new(sp(rhs, rhs_span)),
            };
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn or(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.and()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        while self.eat(&TokenKind::PipePipe) {
            let rs = self.peek_span().start;
            let rhs = self.and()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(BinOp::Or, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn and(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.bitor()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        while self.eat(&TokenKind::AmpAmp) {
            let rs = self.peek_span().start;
            let rhs = self.bitor()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(BinOp::And, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn bitor(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.bitxor()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        while self.eat(&TokenKind::Pipe) {
            let rs = self.peek_span().start;
            let rhs = self.bitxor()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(BinOp::BitOr, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn bitxor(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.bitand()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        while self.eat(&TokenKind::Caret) {
            let rs = self.peek_span().start;
            let rhs = self.bitand()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(BinOp::BitXor, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn bitand(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.equality()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        while self.eat(&TokenKind::Amp) {
            let rs = self.peek_span().start;
            let rhs = self.equality()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(BinOp::BitAnd, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn equality(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.compare()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        loop {
            let op = if self.eat(&TokenKind::EqEq) {
                BinOp::Eq
            } else if self.eat(&TokenKind::BangEq) {
                BinOp::NotEq
            } else {
                break;
            };
            let rs = self.peek_span().start;
            let rhs = self.compare()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(op, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn compare(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.shift()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        loop {
            let op = if self.eat(&TokenKind::LtEq) {
                BinOp::LtEq
            } else if self.eat(&TokenKind::GtEq) {
                BinOp::GtEq
            } else if self.eat(&TokenKind::Lt) {
                BinOp::Lt
            } else if self.eat(&TokenKind::Gt) {
                BinOp::Gt
            } else {
                break;
            };
            let rs = self.peek_span().start;
            let rhs = self.shift()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(op, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        if self.eat(&TokenKind::Is) {
            let target = self.ty()?;
            let e = self.peek_span().start;
            lhs = Expr::Is {
                base: Box::new(sp(lhs, Span { start: s, end: e })),
                target,
            };
        }
        Ok(lhs)
    }

    fn shift(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.range()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        loop {
            let op = if self.eat(&TokenKind::Shl) {
                BinOp::Shl
            } else if self.eat(&TokenKind::Shr) {
                BinOp::Shr
            } else if self.eat(&TokenKind::Zshr) {
                BinOp::Zshr
            } else {
                break;
            };
            let rs = self.peek_span().start;
            let rhs = self.range()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(op, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn range(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let lo = self.add()?;
        let e = self.peek_span().start;
        if self.eat(&TokenKind::DotDot) {
            let hs = self.peek_span().start;
            let hi = self.add()?;
            let he = self.peek_span().start;
            return Ok(Expr::Range {
                lo: Box::new(sp(lo, Span { start: s, end: e })),
                hi: Box::new(sp(hi, Span { start: hs, end: he })),
                inclusive: false,
            });
        }
        if self.eat(&TokenKind::DotDotEq) {
            let hs = self.peek_span().start;
            let hi = self.add()?;
            let he = self.peek_span().start;
            return Ok(Expr::Range {
                lo: Box::new(sp(lo, Span { start: s, end: e })),
                hi: Box::new(sp(hi, Span { start: hs, end: he })),
                inclusive: true,
            });
        }
        Ok(lo)
    }

    fn add(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.mul()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        loop {
            let op = if self.eat(&TokenKind::Plus) {
                BinOp::Add
            } else if self.eat(&TokenKind::Minus) {
                BinOp::Sub
            } else {
                break;
            };
            let rs = self.peek_span().start;
            let rhs = self.mul()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(op, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn mul(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut lhs = self.unary()?;
        let mut lhs_span = Span { start: s, end: self.peek_span().start };
        loop {
            let op = if self.eat(&TokenKind::Star) {
                BinOp::Mul
            } else if self.eat(&TokenKind::Slash) {
                BinOp::Div
            } else if self.eat(&TokenKind::Percent) {
                BinOp::Mod
            } else {
                break;
            };
            let rs = self.peek_span().start;
            let rhs = self.unary()?;
            let rhs_span = Span { start: rs, end: self.peek_span().start };
            lhs = self.bin(op, lhs, lhs_span, rhs, rhs_span);
            lhs_span = Span { start: s, end: self.peek_span().start };
        }
        Ok(lhs)
    }

    fn unary(&mut self) -> PR<Expr> {
        if self.eat(&TokenKind::Await) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Await(Box::new(sp(r, Span { start: s, end: e }))));
        }
        if self.eat(&TokenKind::PlusPlus) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Unary {
                op: UnOp::PreInc,
                rhs: Box::new(sp(r, Span { start: s, end: e })),
            });
        }
        if self.eat(&TokenKind::MinusMinus) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Unary {
                op: UnOp::PreDec,
                rhs: Box::new(sp(r, Span { start: s, end: e })),
            });
        }
        if self.eat(&TokenKind::Minus) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Unary {
                op: UnOp::Neg,
                rhs: Box::new(sp(r, Span { start: s, end: e })),
            });
        }
        if self.eat(&TokenKind::Bang) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Unary {
                op: UnOp::Not,
                rhs: Box::new(sp(r, Span { start: s, end: e })),
            });
        }
        if self.eat(&TokenKind::Tilde) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Unary {
                op: UnOp::BitNot,
                rhs: Box::new(sp(r, Span { start: s, end: e })),
            });
        }
        if self.eat(&TokenKind::Amp) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Unary {
                op: UnOp::AddrOf,
                rhs: Box::new(sp(r, Span { start: s, end: e })),
            });
        }
        if self.eat(&TokenKind::Star) {
            let s = self.peek_span().start;
            let r = self.unary()?;
            let e = self.peek_span().start;
            return Ok(Expr::Unary {
                op: UnOp::Deref,
                rhs: Box::new(sp(r, Span { start: s, end: e })),
            });
        }
        self.postfix_cast()
    }

    fn postfix_cast(&mut self) -> PR<Expr> {
        let s = self.peek_span().start;
        let mut e = self.postfix()?;
        loop {
            let ee = self.peek_span().start;
            if !self.eat(&TokenKind::As) {
                break;
            }
            let t = self.ty()?;
            e = Expr::Cast {
                expr: Box::new(sp(e, Span { start: s, end: ee })),
                ty: t,
            };
        }
        Ok(e)
    }

    fn bin(&self, op: BinOp, lhs: Expr, lhs_span: Span, rhs: Expr, rhs_span: Span) -> Expr {
        Expr::Binary {
            op,
            lhs: Box::new(sp(lhs, lhs_span)),
            rhs: Box::new(sp(rhs, rhs_span)),
        }
    }

    fn ternary_colon_ahead(&self) -> bool {
        let mut depth = 0usize;
        let mut j = self.pos + 1;
        while j < self.toks.len() {
            match &self.toks[j].kind {
                TokenKind::LParen | TokenKind::LBracket | TokenKind::LBrace => depth += 1,
                TokenKind::RParen | TokenKind::RBracket | TokenKind::RBrace => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::Colon => {
                    if depth == 0 {
                        return true;
                    }
                }
                TokenKind::Semi
                | TokenKind::Comma
                | TokenKind::Eof => {
                    if depth == 0 {
                        return false;
                    }
                }
                _ => {}
            }
            j += 1;
        }
        false
    }

    fn postfix(&mut self) -> PR<Expr> {
        let es = self.peek_span().start;
        let mut e = if self.at(&TokenKind::New) {
            self.new_expr()?
        } else {
            self.primary()?
        };
        loop {
            let ee = self.peek_span().start;
            if self.eat(&TokenKind::QuestionDot) {
                let f = self.member_name()?;
                if self.at(&TokenKind::LParen) {
                    let mut args = Vec::new();
                    self.expect(&TokenKind::LParen, "`(`")?;
                    while !self.at(&TokenKind::RParen) {
                        if self.at(&TokenKind::Eof) {
                            return Err(Diagnostic::new(Code::E108, "unterminated call")
                                .with_span(self.peek_span()));
                        }
                        args.push(self.call_arg()?);
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)`")?;
                    e = Expr::OptCall {
                        base: Box::new(sp(e, Span { start: es, end: ee })),
                        field: f,
                        args,
                    };
                } else {
                    e = Expr::OptChain {
                        base: Box::new(sp(e, Span { start: es, end: ee })),
                        field: f,
                    };
                }
            } else if self.at(&TokenKind::Question) && !self.ternary_colon_ahead() {
                self.bump();
                e = Expr::Propagate(Box::new(sp(e, Span { start: es, end: ee })));
            } else if self.at(&TokenKind::Dot) {
                if matches!(self.toks.get(self.pos + 1).map(|t| &t.kind), Some(TokenKind::Int(_))) {
                    self.bump();
                    let idx = match self.peek().clone() {
                        TokenKind::Int(v) => v as usize,
                        _ => {
                            return Err(Diagnostic::new(Code::E108, "expected tuple index")
                                .with_span(self.peek_span()));
                        }
                    };
                    self.bump();
                    e = Expr::TupleGet {
                        base: Box::new(sp(e, Span { start: es, end: ee })),
                        index: idx,
                    };
                } else if self.eat(&TokenKind::Dot) {
                    let f = self.member_name()?;
                    e = Expr::Member {
                        base: Box::new(sp(e, Span { start: es, end: ee })),
                        field: f,
                    };
                    if self.at(&TokenKind::LParen) {
                        let cs = Span { start: es, end: self.peek_span().start };
                        e = self.call_tail(e, cs)?;
                    }
                }
            } else if self.eat(&TokenKind::LBracket) {
                let s = self.peek_span().start;
                // Leading `..hi` / `..=hi` slice: omitted lower bound means 0.
                if self.at(&TokenKind::DotDot) || self.at(&TokenKind::DotDotEq) {
                    let inclusive = self.eat(&TokenKind::DotDotEq);
                    if !inclusive {
                        self.expect(&TokenKind::DotDot, "`..`")?;
                    }
                    let hs = self.peek_span().start;
                    let hi = self.expr()?;
                    let en = self.peek_span().start;
                    self.expect(&TokenKind::RBracket, "`]`")?;
                    let zero = sp(Expr::Int(0), Span { start: s, end: hs });
                    e = Expr::Index {
                        base: Box::new(sp(e, Span { start: es, end: ee })),
                        index: Box::new(sp(
                            Expr::Range {
                                lo: Box::new(zero),
                                hi: Box::new(sp(hi, Span { start: hs, end: en })),
                                inclusive,
                            },
                            Span { start: s, end: en },
                        )),
                    };
                    continue;
                }
                let ix = self.expr()?;
                let en = self.peek_span().start;
                self.expect(&TokenKind::RBracket, "`]`")?;
                e = Expr::Index {
                    base: Box::new(sp(e, Span { start: es, end: ee })),
                    index: Box::new(sp(ix, Span { start: s, end: en })),
                };
            } else if self.at(&TokenKind::LParen) {
                let cs = Span { start: es, end: self.peek_span().start };
                e = self.call_tail(e, cs)?;
            } else if self.at(&TokenKind::LBrace) && self.brace_ok && is_brace_callee(&e) {
                let trailing = self.block()?;
                match e {
                    Expr::Call {
                        callee,
                        type_args,
                        args,
                        trailing: None,
                    } => {
                        e = Expr::Call {
                            callee,
                            type_args,
                            args,
                            trailing: Some(trailing),
                        };
                    }
                    _ => {
                        e = Expr::Call {
                            callee: Box::new(sp(e, Span { start: es, end: ee })),
                            type_args: Vec::new(),
                            args: Vec::new(),
                            trailing: Some(trailing),
                        };
                    }
                }
            } else if self.at(&TokenKind::Lt)
                && matches!(e, Expr::Ident(_) | Expr::Member { .. })
                && self.angle_call_ahead()
            {
                self.bump();
                let mut targs = Vec::new();
                loop {
                    targs.push(self.ty()?);
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.close_angle()?;
                let cs = Span { start: es, end: self.peek_span().start };
                let mut c = self.call_tail(e, cs)?;
                if let Expr::Call { type_args, .. } = &mut c {
                    *type_args = targs;
                }
                e = c;
            } else if self.eat(&TokenKind::PlusPlus) {
                let en = self.peek_span().start;
                e = Expr::Postfix {
                    op: PostfixOp::PostInc,
                    expr: Box::new(sp(e, Span { start: es, end: en })),
                };
            } else if self.eat(&TokenKind::MinusMinus) {
                let en = self.peek_span().start;
                e = Expr::Postfix {
                    op: PostfixOp::PostDec,
                    expr: Box::new(sp(e, Span { start: es, end: en })),
                };
            } else {
                break;
            }
        }
        Ok(e)
    }

    fn angle_call_ahead(&self) -> bool {
        let mut depth = 0usize;
        let mut j = self.pos;
        let mut steps = 0usize;
        while j < self.toks.len() && steps < 48 {
            steps += 1;
            match &self.toks[j].kind {
                TokenKind::Lt => depth += 1,
                TokenKind::Gt => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.toks.get(j + 1).map(|t| &t.kind),
                            Some(TokenKind::LParen)
                        );
                    }
                }
                TokenKind::GtEq => {
                    if depth == 1 {
                        return matches!(
                            self.toks.get(j + 1).map(|t| &t.kind),
                            Some(TokenKind::LParen)
                        );
                    }
                    if depth == 0 {
                        return false;
                    }
                }
                TokenKind::Shr => {
                    if depth < 2 {
                        return false;
                    }
                    depth -= 2;
                    if depth == 0 {
                        return matches!(
                            self.toks.get(j + 1).map(|t| &t.kind),
                            Some(TokenKind::LParen)
                        );
                    }
                }
                TokenKind::Zshr => {
                    if depth < 3 {
                        return false;
                    }
                    depth -= 3;
                    if depth == 0 {
                        return matches!(
                            self.toks.get(j + 1).map(|t| &t.kind),
                            Some(TokenKind::LParen)
                        );
                    }
                }
                TokenKind::Ident(_)
                | TokenKind::Comma
                | TokenKind::Dot
                | TokenKind::Question => {}
                _ => return false,
            }
            j += 1;
        }
        false
    }

    fn new_expr(&mut self) -> PR<Expr> {
        self.expect(&TokenKind::New, "`new`")?;
        let (mut target, _) = self.ident()?;
        while self.eat(&TokenKind::Dot) {
            let seg = self.member_name()?;
            target.push('.');
            target.push_str(&seg);
        }
        let mut type_args = Vec::new();
        if self.at(&TokenKind::Lt) && self.angle_call_ahead() {
            self.bump();
            loop {
                type_args.push(self.ty()?);
                if !self.eat(&TokenKind::Comma) {
                    break;
                }
            }
            self.close_angle()?;
        }
        self.expect(&TokenKind::LParen, "`(` after `new Target`")?;
        let mut args = Vec::new();
        while !self.at(&TokenKind::RParen) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated `new` argument list")
                    .with_span(self.peek_span()));
            }
            args.push(self.call_arg()?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        Ok(Expr::New { target, type_args, args })
    }

    fn call_tail(&mut self, callee: Expr, callee_span: Span) -> PR<Expr> {        self.expect(&TokenKind::LParen, "`(`")?;
        let mut args = Vec::new();
        while !self.at(&TokenKind::RParen) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated call")
                    .with_span(self.peek_span()));
            }
            args.push(self.call_arg()?);
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RParen, "`)`")?;
        Ok(Expr::Call {
            callee: Box::new(sp(callee, callee_span)),
            type_args: Vec::new(),
            args,
            trailing: None,
        })
    }

    fn record_ahead(&self) -> bool {
        let is_key = matches!(
            self.toks.get(self.pos + 1).map(|t| &t.kind),
            Some(TokenKind::Ident(_))
                | Some(TokenKind::Default)
                | Some(TokenKind::Case)
                | Some(TokenKind::Switch)
                | Some(TokenKind::Do)
                | Some(TokenKind::Is)
                | Some(TokenKind::As)
                | Some(TokenKind::From)
                | Some(TokenKind::Export)
                | Some(TokenKind::Native)
                | Some(TokenKind::In)
                | Some(TokenKind::With)
                | Some(TokenKind::Init)
                | Some(TokenKind::New)
                | Some(TokenKind::Deinit)
                | Some(TokenKind::OnReload)
                | Some(TokenKind::This)
                | Some(TokenKind::Record)
                | Some(TokenKind::Unsafe)
                | Some(TokenKind::Await)
                | Some(TokenKind::Public)
                | Some(TokenKind::Private)
                | Some(TokenKind::Const)
                | Some(TokenKind::Static)
        );
        is_key
            && self.toks.get(self.pos + 2).map(|t| &t.kind) == Some(&TokenKind::Colon)
    }

    fn map_ahead(&self) -> bool {
        if !self.at(&TokenKind::LBrace) {
            return false;
        }
        let mut i = self.pos + 1;
        if matches!(self.toks.get(i).map(|t| &t.kind), Some(TokenKind::RBrace)) {
            return true;
        }
        let mut depth = 0usize;
        while i < self.toks.len() {
            match &self.toks[i].kind {
                TokenKind::LBrace | TokenKind::LBracket | TokenKind::LParen => depth += 1,
                TokenKind::RBrace | TokenKind::RBracket | TokenKind::RParen => {
                    if depth == 0 {
                        return false;
                    }
                    depth -= 1;
                }
                TokenKind::StrOpen if depth == 0 => {
                    let mut j = i + 1;
                    while j < self.toks.len() {
                        match &self.toks[j].kind {
                            TokenKind::StrText(_) => j += 1,
                            TokenKind::StrClose => break,
                            _ => return false,
                        }
                    }
                    if self.toks.get(j).map(|t| &t.kind) != Some(&TokenKind::StrClose) {
                        return false;
                    }
                    if self.toks.get(j + 1).map(|t| &t.kind) == Some(&TokenKind::Colon) {
                        return true;
                    }
                    i = j + 1;
                    continue;
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn map_key(&mut self) -> PR<String> {
        match self.peek().clone() {
            TokenKind::Ident(n) => {
                self.bump();
                Ok(n)
            }
            TokenKind::StrOpen => self.plain_string(),
            _ => Err(Diagnostic::new(Code::E108, "expected string key or identifier")
                .with_span(self.peek_span())),
        }
    }

    fn map_literal(&mut self) -> PR<Spanned<Expr>> {
        let s = self.peek_span().start;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut entries = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated map literal")
                    .with_span(self.peek_span()));
            }
            if self.eat(&TokenKind::Ellipsis) {
                let vs = self.peek_span().start;
                let v = self.expr()?;
                let ve = self.peek_span().start;
                entries.push(MapEntry::Spread(sp(v, Span { start: vs, end: ve })));
            } else {
                let k = self.map_key()?;
                self.expect(&TokenKind::Colon, "`:`")?;
                let vs = self.peek_span().start;
                let v = self.expr()?;
                let ve = self.peek_span().start;
                entries.push(MapEntry::Field(k, sp(v, Span { start: vs, end: ve })));
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, "`}`")?;
        let e = self.peek_span().start;
        Ok(sp(Expr::MapLiteral(entries), Span { start: s, end: e }))
    }

    fn call_arg(&mut self) -> PR<CallArg> {
        if self.at(&TokenKind::LBrace) {
            if self.map_ahead() {
                let v = self.map_literal()?;
                return Ok(CallArg { name: None, value: v });
            }
            let j = self.pos + 1;
            let mut is_record = false;
            if j < self.toks.len() {
                match &self.toks[j].kind {
                    TokenKind::Ident(_) => {
                        if let Some(TokenKind::Colon) = self.toks.get(j + 1).map(|t| &t.kind) {
                            is_record = true;
                        }
                    }
                    _ => {}
                }
            }
            if is_record {
                let v = self.record()?;
                return Ok(CallArg { name: None, value: v });
            }
        }
        if matches!(self.peek(), TokenKind::Ident(_)) {
            if let Some(TokenKind::Colon) = self.toks.get(self.pos + 1).map(|t| &t.kind) {
                let (n, _) = self.ident()?;
                self.expect(&TokenKind::Colon, "`:`")?;
                let s = self.peek_span().start;
                let v = self.expr()?;
                let e = self.peek_span().start;
                return Ok(CallArg {
                    name: Some(n),
                    value: sp(v, Span { start: s, end: e }),
                });
            }
        }
        let s = self.peek_span().start;
        let v = self.expr()?;
        let e = self.peek_span().start;
        Ok(CallArg {
            name: None,
            value: sp(v, Span { start: s, end: e }),
        })
    }

    fn record(&mut self) -> PR<Spanned<Expr>> {
        let s = self.peek_span().start;
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut fields = Vec::new();
        while !self.at(&TokenKind::RBrace) {
            if self.at(&TokenKind::Eof) {
                return Err(Diagnostic::new(Code::E108, "unterminated record")
                    .with_span(self.peek_span()));
            }
            if self.eat(&TokenKind::Ellipsis) {
                let vs = self.peek_span().start;
                let v = self.expr()?;
                let ve = self.peek_span().start;
                fields.push(RecordEntry::Spread(sp(v, Span { start: vs, end: ve })));
            } else {
                let n = self.record_key()?;
                self.expect(&TokenKind::Colon, "`:`")?;
                let vs = self.peek_span().start;
                let v = self.expr()?;
                let ve = self.peek_span().start;
                fields.push(RecordEntry::Field(n, sp(v, Span { start: vs, end: ve })));
            }
            if !self.eat(&TokenKind::Comma) {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, "`}`")?;
        let e = self.peek_span().start;
        Ok(sp(Expr::Record(fields), Span { start: s, end: e }))
    }

    fn lambda_ahead(&self) -> bool {
        let mut depth = 0usize;
        let mut i = self.pos;
        while i < self.toks.len() {
            match &self.toks[i].kind {
                TokenKind::LParen => depth += 1,
                TokenKind::RParen => {
                    depth -= 1;
                    if depth == 0 {
                        return matches!(
                            self.toks.get(i + 1).map(|t| &t.kind),
                            Some(TokenKind::FatArrow) | Some(TokenKind::Colon)
                        );
                    }
                }
                TokenKind::Eof => return false,
                _ => {}
            }
            i += 1;
        }
        false
    }

    fn arrow_body(&mut self) -> PR<FnBody> {
        if self.at(&TokenKind::LBrace) {
            Ok(FnBody::Block(self.block()?))
        } else {
            let s = self.peek_span().start;
            let e = self.expr()?;
            let en = self.peek_span().start;
            Ok(FnBody::Expr(Box::new(sp(e, Span { start: s, end: en }))))
        }
    }

    fn arrow_closure_paren(&mut self) -> PR<Expr> {
        let params = self.params()?;
        let ret = if self.eat(&TokenKind::Colon) {
            Some(self.ty()?)
        } else {
            None
        };
        self.expect(&TokenKind::FatArrow, "`=>`")?;
        let body = self.arrow_body()?;
        Ok(Expr::Closure {
            decay: false,
            is_async: false,
            params,
            ret,
            throws: false,
            body,
        })
    }

    fn switch_arm_body(&mut self) -> PR<SwitchExprBody> {
        if self.at(&TokenKind::LBrace) && !self.record_ahead() {
            let b = self.block()?;
            match b.stmts.last() {
                Some(st) if matches!(st.node, Stmt::Expr(_)) => Ok(SwitchExprBody::Block(b)),
                _ => Err(Diagnostic::new(Code::E108, "case block must end with a value expression")
                    .with_span(self.peek_span())),
            }
        } else {
            let s = self.peek_span().start;
            let e = self.expr()?;
            let en = self.peek_span().start;
            self.eat(&TokenKind::Comma);
            self.eat_semi();
            Ok(SwitchExprBody::Expr(Box::new(sp(e, Span { start: s, end: en }))))
        }
    }

    fn switch_expr(&mut self) -> PR<Expr> {
        self.bump();
        let s = self.peek_span().start;
        let sc = self.cond()?;
        let e = self.peek_span().start;
        let scrutinee = sp(sc, Span { start: s, end: e });
        self.expect(&TokenKind::LBrace, "`{`")?;
        let mut cases = Vec::new();
        let mut default = None;
        loop {
            if self.eat(&TokenKind::Case) {
                let pattern = self.pattern()?;
                let guard = if self.eat(&TokenKind::If) {
                    let s = self.peek_span().start;
                    let g = self.cond()?;
                    let e = self.peek_span().start;
                    Some(sp(g, Span { start: s, end: e }))
                } else {
                    None
                };
                self.expect(&TokenKind::Colon, "`:`")?;
                let body = self.switch_arm_body()?;
                cases.push(SwitchExprCase { pattern, guard, body });
            } else if self.eat(&TokenKind::Default) {
                self.expect(&TokenKind::Colon, "`:`")?;
                default = Some(self.switch_arm_body()?);
            } else {
                break;
            }
        }
        self.expect(&TokenKind::RBrace, "`}`")?;
        Ok(Expr::Switch { scrutinee: Box::new(scrutinee), cases, default })
    }

    fn primary(&mut self) -> PR<Expr> {
        if self.at(&TokenKind::LBrace) {
            if self.map_ahead() {
                return Ok(self.map_literal()?.node);
            }
            if self.record_ahead() {
                return Ok(self.record()?.node);
            }
            if matches!(self.toks.get(self.pos + 1).map(|t| &t.kind), Some(TokenKind::Ellipsis)) {
                let save = self.pos;
                let errlen = self.errors.len();
                match self.record() {
                    Ok(v) => return Ok(v.node),
                    Err(_) => {
                        self.pos = save;
                        self.errors.truncate(errlen);
                        return Ok(self.map_literal()?.node);
                    }
                }
            }
        }
        if self.at(&TokenKind::Switch) {
            return self.switch_expr();
        }
        match self.peek().clone() {
            TokenKind::Import => {
                return Err(Diagnostic::new(
                    Code::E108,
                    "dynamic `import(...)` is not supported; use static `import ... from \"...\"`",
                )
                .with_span(self.peek_span()));
            }
            TokenKind::Ident(name) => {
                let name_span = self.peek_span();
                self.bump();
                if self.eat(&TokenKind::FatArrow) {
                    let body = self.arrow_body()?;
                    return Ok(Expr::Closure {
                        decay: false,
                        is_async: false,
                        params: vec![Param {
                            name,
                            span: name_span,
                            ty: None,
                            default: None,
                            promote: false,
                        }],
                        ret: None,
                        throws: false,
                        body,
                    });
                }
                if self.eat(&TokenKind::Bang) {
                    let bracket = if self.at(&TokenKind::LBracket) {
                        self.bump();
                        true
                    } else {
                        self.expect(&TokenKind::LParen, "`(` or `[`")?;
                        false
                    };
                    let mut args = Vec::new();
                    let close = if bracket {
                        TokenKind::RBracket
                    } else {
                        TokenKind::RParen
                    };
                    while !self.at(&close) {
                        if self.at(&TokenKind::Eof) {
                            return Err(Diagnostic::new(
                                Code::E108,
                                "unterminated macro",
                            )
                            .with_span(self.peek_span()));
                        }
                        let s = self.peek_span().start;
                        let a = self.expr()?;
                        let e = self.peek_span().start;
                        args.push(sp(a, Span { start: s, end: e }));
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.bump();
                    return Ok(Expr::Macro {
                        name,
                        args,
                        bracket,
                    });
                }
                let mut path = vec![(name, name_span)];
                while self.at(&TokenKind::Dot)
                    && !matches!(
                        self.toks.get(self.pos + 1).map(|t| &t.kind),
                        Some(TokenKind::Int(_))
                    )
                {
                    self.bump();
                    let seg_span = self.peek_span();
                    let n = self.member_name()?;
                    let seg_span = Span { start: seg_span.start, end: self.peek_span().start };
                    path.push((n, seg_span));
                }
                Ok(path_to_expr(path))
            }
            TokenKind::This => {
                self.bump();
                Ok(Expr::This)
            }
            TokenKind::Super => {
                self.bump();
                Ok(Expr::Super)
            }
            TokenKind::True => {
                self.bump();
                Ok(Expr::Bool(true))
            }
            TokenKind::False => {
                self.bump();
                Ok(Expr::Bool(false))
            }
            TokenKind::Null => {
                self.bump();
                Ok(Expr::Null)
            }
            TokenKind::Int(v) => {
                self.bump();
                Ok(Expr::Int(v))
            }
            TokenKind::Float(v) => {
                self.bump();
                Ok(Expr::Float(v))
            }
            TokenKind::StrOpen => {
                self.bump();
                let mut parts = Vec::new();
                loop {
                    match self.peek().clone() {
                        TokenKind::StrText(t) => {
                            parts.push(InterpPart::Text(t));
                            self.bump();
                        }
                        TokenKind::InterpOpen => {
                            self.bump();
                            let s = self.peek_span().start;
                            let e = self.expr()?;
                            let en = self.peek_span().start;
                            self.expect(&TokenKind::InterpClose, "`}`")?;
                            parts.push(InterpPart::Expr(sp(
                                e,
                                Span { start: s, end: en },
                            )));
                        }
                        TokenKind::StrClose => {
                            self.bump();
                            break;
                        }
                        _ => {
                            return Err(Diagnostic::new(
                                Code::E108,
                                "unterminated string",
                            )
                            .with_span(self.peek_span()));
                        }
                    }
                }
                Ok(Expr::Interp(parts))
            }
            TokenKind::Dot => {
                let mut path = Vec::new();
                while self.eat(&TokenKind::Dot) {
                    let n = self.member_name()?;
                    path.push(n);
                }
                if path.is_empty() {
                    return Err(Diagnostic::new(Code::E108, "expected `.Member`")
                        .with_span(self.peek_span()));
                }
                Ok(Expr::ImplicitMember(path))
            }
            TokenKind::LParen => {
                if self.lambda_ahead() {
                    let save = self.pos;
                    match self.arrow_closure_paren() {
                        Ok(e) => return Ok(e),
                        Err(_) => {
                            self.pos = save;
                        }
                    }
                }
                self.bump();
                let s = self.peek_span().start;
                let first = self.expr()?;
                let en = self.peek_span().start;
                if self.eat(&TokenKind::Comma) {
                    let mut items = vec![sp(first, Span { start: s, end: en })];
                    loop {
                        if self.at(&TokenKind::Eof) {
                            return Err(Diagnostic::new(Code::E108, "unterminated tuple")
                                .with_span(self.peek_span()));
                        }
                        let s = self.peek_span().start;
                        let e = self.expr()?;
                        let en = self.peek_span().start;
                        items.push(sp(e, Span { start: s, end: en }));
                        if !self.eat(&TokenKind::Comma) {
                            break;
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)`")?;
                    return Ok(Expr::Tuple(items));
                }
                self.expect(&TokenKind::RParen, "`)`")?;
                Ok(first)
            }
            TokenKind::LBracket => {
                self.bump();
                let mut items = Vec::new();
                while !self.at(&TokenKind::RBracket) {
                    if self.at(&TokenKind::Eof) {
                        return Err(Diagnostic::new(Code::E108, "unterminated array")
                            .with_span(self.peek_span()));
                    }
                    let spread = self.eat(&TokenKind::Ellipsis);
                    let s = self.peek_span().start;
                    let e = self.expr()?;
                    let en = self.peek_span().start;
                    items.push(crate::ast::ArrayElem {
                        spread,
                        expr: sp(e, Span { start: s, end: en }),
                    });
                    if !self.eat(&TokenKind::Comma) {
                        break;
                    }
                }
                self.expect(&TokenKind::RBracket, "`]`")?;
                Ok(Expr::Array(items))
            }
            TokenKind::Fn => {
                let fn_span = self.peek_span();
                self.bump();
                if self.at(&TokenKind::LParen) {
                    return Err(Diagnostic::new(Code::E105, "invalid lambda syntax")
                        .with_span(fn_span)
                        .with_hint("arrow expressions do not use `fn`. Use `(x) => x * 2` or declare a named function `fn mul(x) { ... }`."));
                }
                let decay = if matches!(self.peek(), TokenKind::Ident(n) if n == "decay") {
                    self.bump();
                    self.expect(&TokenKind::LParen, "`(`")?;
                    match self.ident()? {
                        (n, _) if n == "this" => {}
                        _ => {
                            return Err(Diagnostic::new(
                                Code::E108,
                                "expected `decay(this)`",
                            )
                            .with_span(self.peek_span()));
                        }
                    }
                    self.expect(&TokenKind::RParen, "`)`")?;
                    true
                } else {
                    return Err(Diagnostic::new(Code::E105, "invalid lambda syntax")
                        .with_span(fn_span)
                        .with_hint("arrow expressions do not use `fn`. Use `(x) => x * 2` or declare a named function `fn mul(x) { ... }`."));
                };
                let params = if self.at(&TokenKind::LParen) {
                    self.params()?
                } else {
                    Vec::new()
                };
                let ret = if self.eat(&TokenKind::Colon) {
                    Some(self.ty()?)
                } else {
                    None
                };
                let throws = self.eat(&TokenKind::Throws);
                if !self.at(&TokenKind::LBrace) {
                    self.skip_arrow_body();
                    return Err(Diagnostic::new(Code::E105, "invalid lambda syntax")
                        .with_span(fn_span)
                        .with_hint("`fn decay(this)` requires a block body. Use `fn decay(this) { ... }`; for lambdas use `(x) => x * 2`."));
                }
                let body = FnBody::Block(self.block()?);
                Ok(Expr::Closure {
                    decay,
                    is_async: false,
                    params,
                    ret,
                    throws,
                    body,
                })
            }
            TokenKind::Unsafe => {
                self.bump();
                Ok(Expr::UnsafeBlock(self.block()?))
            }
            TokenKind::Async => {
                self.bump();
                if self.at(&TokenKind::LParen) {
                    let params = self.params()?;
                    let ret = if self.eat(&TokenKind::Colon) {
                        Some(self.ty()?)
                    } else {
                        None
                    };
                    self.expect(&TokenKind::FatArrow, "`=>`")?;
                    let body = self.arrow_body()?;
                    return Ok(Expr::Closure {
                        decay: false,
                        is_async: true,
                        params,
                        ret,
                        throws: false,
                        body,
                    });
                }
                match self.peek().clone() {
                    TokenKind::Ident(name) => {
                        let name_span = self.peek_span();
                        self.bump();
                        self.expect(&TokenKind::FatArrow, "`=>`")?;
                        let body = self.arrow_body()?;
                        Ok(Expr::Closure {
                            decay: false,
                            is_async: true,
                            params: vec![Param {
                                name,
                                span: name_span,
                                ty: None,
                                default: None,
                                promote: false,
                            }],
                            ret: None,
                            throws: false,
                            body,
                        })
                    }
                    _ => Err(Diagnostic::new(Code::E108, "expected `(` or parameter after `async`")
                        .with_span(self.peek_span())),
                }
            }
            _ => Err(Diagnostic::new(
                Code::E108,
                format!("expected expression, found {}", describe(self.peek())),
            )
            .with_span(self.peek_span())),
        }
    }
}

fn path_to_expr(path: Vec<(String, Span)>) -> Expr {
    let mut it = path.into_iter();
    let (first, first_span) = it.next().unwrap_or((String::new(), Span { start: 0, end: 0 }));
    let mut e = Expr::Ident(first);
    let mut chain_span = first_span;
    for (p, seg) in it {
        e = Expr::Member {
            base: Box::new(sp(e, chain_span)),
            field: p,
        };
        chain_span = Span { start: chain_span.start, end: seg.end };
    }
    e
}

fn is_brace_callee(e: &Expr) -> bool {
    matches!(e, Expr::Ident(_) | Expr::Member { .. } | Expr::ImplicitMember(_))
}

pub fn describe(k: &TokenKind) -> String {
    match k {
        TokenKind::Eof => "end of file".to_string(),
        TokenKind::Ident(s) => format!("identifier `{s}`"),
        TokenKind::Int(v) => format!("integer `{v}`"),
        TokenKind::Float(v) => format!("float `{v}`"),
        TokenKind::StrText(_) => "string text".to_string(),
        _ => format!("{k:?}"),
    }
}
