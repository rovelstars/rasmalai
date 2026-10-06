#[derive(Clone, Copy, PartialEq, Eq)]
enum Fk {
    Word,
    Num,
    Str,
    Line,
    Block,
    Newline,
    LBrace,
    RBrace,
    LParen,
    RParen,
    LBrack,
    RBrack,
    Comma,
    Semi,
    Colon,
    ColonColon,
    Dot,
    DotDot,
    Op,
    Question,
    QuestionQuestion,
    QuestionDot,
    Hash,
    FatArrow,
    Arrow,
}

struct Ft {
    kind: Fk,
    text: String,
    n: usize,
}

fn is_word_start(b: u8) -> bool {
    b.is_ascii_alphabetic() || b == b'_'
}

fn is_word_char(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn tokenize(src: &str) -> Result<Vec<Ft>, String> {
    let bytes = src.as_bytes();
    let mut toks = Vec::new();
    let mut i = 0usize;
    let mut push = |kind: Fk, text: String, n: usize| {
        toks.push(Ft { kind, text, n });
    };
    let slice = |s: usize, e: usize| src[s..e].to_string();
    while i < bytes.len() {
        let c = bytes[i];
        match c {
            b' ' | b'\t' | b'\r' => {
                i += 1;
            }
            b'\n' => {
                let mut n = 0usize;
                while i < bytes.len() && (bytes[i] == b'\n' || bytes[i] == b' ' || bytes[i] == b'\t' || bytes[i] == b'\r') {
                    if bytes[i] == b'\n' {
                        n += 1;
                    }
                    i += 1;
                }
                push(Fk::Newline, String::new(), n);
            }
            b'/' if bytes.get(i + 1) == Some(&b'/') => {
                let s = i;
                while i < bytes.len() && bytes[i] != b'\n' {
                    i += 1;
                }
                push(Fk::Line, slice(s, i), 0);
            }
            b'/' if bytes.get(i + 1) == Some(&b'*') => {
                let s = i;
                // Match the lexer: `/** ... */` (but not `/**/`) is a doc
                // comment that ends at the first `*/`, so a `/*` sequence
                // inside doc prose (e.g. a `src/*.rnx` glob) must not nest.
                let doc = bytes.get(i + 2) == Some(&b'*') && bytes.get(i + 3) != Some(&b'/');
                i += 2;
                if doc {
                    while i + 1 < bytes.len()
                        && !(bytes[i] == b'*' && bytes[i + 1] == b'/')
                    {
                        i += 1;
                    }
                    if i + 1 >= bytes.len() {
                        return Err(format!("unterminated block comment at byte {s}"));
                    }
                    i += 2;
                    push(Fk::Block, slice(s, i), 0);
                    continue;
                }
                let mut depth = 1usize;
                while i < bytes.len() && depth > 0 {
                    if bytes[i] == b'/' && bytes.get(i + 1) == Some(&b'*') {
                        depth += 1;
                        i += 2;
                    } else if bytes[i] == b'*' && bytes.get(i + 1) == Some(&b'/') {
                        depth -= 1;
                        i += 2;
                    } else {
                        i += 1;
                    }
                }
                if depth > 0 {
                    return Err(format!("unterminated block comment at byte {s}"));
                }
                push(Fk::Block, slice(s, i), 0);
            }
            b'"' => {
                let s = i;
                i += 1;
                let mut depth = 0usize;
                while i < bytes.len() {
                    let d = bytes[i];
                    if d == b'\\' {
                        i += 2;
                    } else if d == b'$' && bytes.get(i + 1) == Some(&b'{') {
                        depth += 1;
                        i += 2;
                    } else if d == b'}' && depth > 0 {
                        depth -= 1;
                        i += 1;
                    } else if d == b'"' && depth == 0 {
                        i += 1;
                        break;
                    } else {
                        i += 1;
                    }
                }
                if !src[s..i.min(src.len())].ends_with('"') {
                    return Err(format!("unterminated string at byte {s}"));
                }
                push(Fk::Str, slice(s, i), 0);
            }
            b'0'..=b'9' => {
                let s = i;
                if c == b'0' && matches!(bytes.get(i + 1), Some(b'x') | Some(b'X')) {
                    i += 2;
                    while i < bytes.len() && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_') {
                        i += 1;
                    }
                } else {
                    while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                        i += 1;
                    }
                    if bytes.get(i) == Some(&b'.')
                        && bytes.get(i + 1) != Some(&b'.')
                        && matches!(bytes.get(i + 1), Some(b'0'..=b'9'))
                    {
                        i += 1;
                        while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                            i += 1;
                        }
                    }
                }
                push(Fk::Num, slice(s, i), 0);
            }
            _ if is_word_start(c) => {
                let s = i;
                while i < bytes.len() && is_word_char(bytes[i]) {
                    i += 1;
                }
                push(Fk::Word, slice(s, i), 0);
            }
            b'{' => {
                push(Fk::LBrace, slice(i, i + 1), 0);
                i += 1;
            }
            b'}' => {
                push(Fk::RBrace, slice(i, i + 1), 0);
                i += 1;
            }
            b'(' => {
                push(Fk::LParen, slice(i, i + 1), 0);
                i += 1;
            }
            b')' => {
                push(Fk::RParen, slice(i, i + 1), 0);
                i += 1;
            }
            b'[' => {
                push(Fk::LBrack, slice(i, i + 1), 0);
                i += 1;
            }
            b']' => {
                push(Fk::RBrack, slice(i, i + 1), 0);
                i += 1;
            }
            b',' => {
                push(Fk::Comma, slice(i, i + 1), 0);
                i += 1;
            }
            b';' => {
                push(Fk::Semi, slice(i, i + 1), 0);
                i += 1;
            }
            b'?' if bytes.get(i + 1) == Some(&b'?') => {
                push(Fk::QuestionQuestion, slice(i, i + 2), 0);
                i += 2;
            }
            b'?' if bytes.get(i + 1) == Some(&b'.') => {
                push(Fk::QuestionDot, slice(i, i + 2), 0);
                i += 2;
            }
            b'?' => {
                push(Fk::Question, slice(i, i + 1), 0);
                i += 1;
            }
            b'#' => {
                push(Fk::Hash, slice(i, i + 1), 0);
                i += 1;
            }
            b':' if bytes.get(i + 1) == Some(&b':') => {
                push(Fk::ColonColon, slice(i, i + 2), 0);
                i += 2;
            }
            b':' => {
                push(Fk::Colon, slice(i, i + 1), 0);
                i += 1;
            }
            b'.' if bytes.get(i + 1) == Some(&b'.') => {
                if bytes.get(i + 2) == Some(&b'=') {
                    push(Fk::DotDot, slice(i, i + 3), 0);
                    i += 3;
                } else {
                    push(Fk::DotDot, slice(i, i + 2), 0);
                    i += 2;
                }
            }
            b'.' => {
                push(Fk::Dot, slice(i, i + 1), 0);
                i += 1;
            }
            b'=' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'=' if bytes.get(i + 1) == Some(&b'>') => {
                push(Fk::FatArrow, slice(i, i + 2), 0);
                i += 2;
            }
            b'=' => {
                push(Fk::Op, slice(i, i + 1), 0);
                i += 1;
            }
            b'!' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'<' if bytes.get(i + 1) == Some(&b'<') => {
                if bytes.get(i + 2) == Some(&b'=') {
                    push(Fk::Op, slice(i, i + 3), 0);
                    i += 3;
                } else {
                    push(Fk::Op, slice(i, i + 2), 0);
                    i += 2;
                }
            }
            b'>' if bytes.get(i + 1) == Some(&b'>') => {
                if bytes.get(i + 2) == Some(&b'>') {
                    if bytes.get(i + 3) == Some(&b'=') {
                        push(Fk::Op, slice(i, i + 4), 0);
                        i += 4;
                    } else {
                        push(Fk::Op, slice(i, i + 3), 0);
                        i += 3;
                    }
                } else if bytes.get(i + 2) == Some(&b'=') {
                    push(Fk::Op, slice(i, i + 3), 0);
                    i += 3;
                } else {
                    push(Fk::Op, slice(i, i + 2), 0);
                    i += 2;
                }
            }
            b'<' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'>' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'+' if bytes.get(i + 1) == Some(&b'+') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'+' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'-' if bytes.get(i + 1) == Some(&b'-') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'-' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'-' if bytes.get(i + 1) == Some(&b'>') => {
                push(Fk::Arrow, slice(i, i + 2), 0);
                i += 2;
            }
            b'*' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'/' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'%' if bytes.get(i + 1) == Some(&b'=') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'&' if bytes.get(i + 1) == Some(&b'&') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'|' if bytes.get(i + 1) == Some(&b'|') => {
                push(Fk::Op, slice(i, i + 2), 0);
                i += 2;
            }
            b'+' | b'-' | b'*' | b'/' | b'%' | b'<' | b'>' | b'!' | b'&' => {
                push(Fk::Op, slice(i, i + 1), 0);
                i += 1;
            }
            _ => {
                return Err(format!("unexpected character `{}` at byte {i}", c as char));
            }
        }
    }
    Ok(toks)
}

fn prev_sig_idx(toks: &[Ft], i: usize) -> Option<usize> {
    let mut j = i;
    while j > 0 {
        j -= 1;
        if !is_trivia(toks[j].kind) {
            return Some(j);
        }
    }
    None
}

fn match_generic_brackets(toks: &[Ft]) -> Vec<bool> {
    let mut generic = vec![false; toks.len()];
    for i in 0..toks.len() {
        if toks[i].kind != Fk::Op || toks[i].text != "<" {
            continue;
        }
        if !matches!(
            prev_sig_idx(toks, i).map(|j| toks[j].kind),
            Some(Fk::Word) | Some(Fk::RBrack) | Some(Fk::Question)
        ) {
            continue;
        }
        let mut depth = 1usize;
        let mut j = i + 1;
        let mut close = None;
        while j < toks.len() {
            match toks[j].kind {
                Fk::Op if toks[j].text == "<" => depth += 1,
                Fk::Op if toks[j].text == ">" || toks[j].text == ">>" || toks[j].text == ">>>" => {
                    let closes = toks[j].text.len();
                    if closes >= depth {
                        depth = 0;
                        close = Some(j);
                        break;
                    }
                    depth -= closes;
                }
                Fk::Word | Fk::Num | Fk::Comma | Fk::Question | Fk::Dot | Fk::DotDot
                | Fk::ColonColon | Fk::LBrack | Fk::RBrack => {}
                _ => break,
            }
            j += 1;
        }
        if let Some(j) = close {
            let follow_ok = {
                let mut k = j + 1;
                while k < toks.len() && is_trivia(toks[k].kind) {
                    k += 1;
                }
                if k >= toks.len() {
                    true
                } else {
                    match toks[k].kind {
                        Fk::Semi | Fk::Comma | Fk::RParen | Fk::RBrack | Fk::RBrace
                        | Fk::LBrace | Fk::LParen | Fk::Colon | Fk::ColonColon | Fk::Dot
                        | Fk::Question | Fk::FatArrow | Fk::Arrow | Fk::Newline => true,
                        Fk::Word => matches!(toks[k].text.as_str(), "with" | "extends" | "where"),
                        Fk::Op => {
                            toks[k].text == "="
                                || toks[k].text == ">"
                                || toks[k].text == ">>"
                                || toks[k].text == ">>>"
                        }
                        _ => false,
                    }
                }
            };
            if follow_ok {
                generic[i] = true;
                generic[j] = true;
            }
        }
    }
    generic
}

fn match_parens(toks: &[Ft]) -> Vec<Option<usize>> {
    let mut m = vec![None; toks.len()];
    let mut stack: Vec<usize> = Vec::new();
    for (idx, tok) in toks.iter().enumerate() {
        if tok.kind == Fk::LParen {
            stack.push(idx);
        } else if tok.kind == Fk::RParen && let Some(open) = stack.pop() {
            m[open] = Some(idx);
            m[idx] = Some(open);
        }
    }
    m
}

fn is_trivia(kind: Fk) -> bool {
    matches!(kind, Fk::Newline | Fk::Line | Fk::Block)
}

struct Emitter<'a> {
    toks: &'a [Ft],
    m: Vec<Option<usize>>,
    generic: Vec<bool>,
    out: String,
    indent: usize,
    start: bool,
    stack: Vec<(bool, usize, usize)>,
    stripped: Vec<usize>,
}

impl<'a> Emitter<'a> {
    fn prev_sig(&self, i: usize) -> Option<usize> {
        let mut j = i;
        while j > 0 {
            j -= 1;
            if !is_trivia(self.toks[j].kind) && !self.stripped.contains(&j) {
                return Some(j);
            }
        }
        None
    }

    fn next_sig(&self, i: usize) -> Option<usize> {
        let mut j = i + 1;
        while j < self.toks.len() {
            if !is_trivia(self.toks[j].kind) && !self.stripped.contains(&j) {
                return Some(j);
            }
            j += 1;
        }
        None
    }

    fn has_newline_between(&self, i: usize, j: usize) -> bool {
        (i + 1..j).any(|k| self.toks[k].kind == Fk::Newline)
    }

    fn nl(&mut self) {
        while self.out.ends_with(' ') || self.out.ends_with('\t') {
            self.out.pop();
        }
        self.out.push('\n');
        self.start = true;
    }

    fn ensure_indent(&mut self) {
        if self.start {
            for _ in 0..self.indent {
                self.out.push_str("    ");
            }
            self.start = false;
        }
    }

    fn sp(&mut self) {
        if !self.start && !self.out.ends_with(' ') && !self.out.ends_with('\n') {
            self.out.push(' ');
        }
    }

    fn cont_level(&self) -> usize {
        self.stack.last().map(|(_, base, _)| base + 1).unwrap_or(self.indent)
    }

    fn br(&mut self) {
        self.nl();
        self.indent = self.cont_level();
    }

    fn closer_ahead(&self, i: usize) -> bool {
        matches!(
            self.next_sig(i).map(|j| self.toks[j].kind),
            Some(Fk::RBrace) | Some(Fk::RParen) | Some(Fk::RBrack) | None
        )
    }

    fn emit_wordlike(&mut self, i: usize) {
        let text = self.toks[i].text.clone();
        if !self.start {
            let after_block = i > 0 && self.toks[i - 1].kind == Fk::Block;
            match self.prev_sig(i).map(|j| self.toks[j].kind) {
                Some(Fk::Word) | Some(Fk::Num) | Some(Fk::Str) | Some(Fk::RParen)
                | Some(Fk::RBrack) | Some(Fk::RBrace) | Some(Fk::Block) => self.sp(),
                _ => {
                    if after_block {
                        self.sp();
                    } else if let Some(j) = self.prev_sig(i)
                        && self.toks[j].kind == Fk::Op
                        && (self.toks[j].text == ">"
                            || self.toks[j].text == ">>"
                            || self.toks[j].text == ">>>")
                        && self.generic[j]
                    {
                        self.sp();
                    }
                }
            }
        }
        self.ensure_indent();
        self.out.push_str(&text);
    }

    fn run(&mut self) {
        let mut i = 0usize;
        while i < self.toks.len() {
            if self.stripped.contains(&i) {
                i += 1;
                continue;
            }
            let kind = self.toks[i].kind;
            let text = self.toks[i].text.clone();
            let n = self.toks[i].n;
            match kind {
                Fk::Newline => {
                    if self
                        .stripped
                        .chunks(2)
                        .any(|w| w.len() == 2 && w[0] < i && i < w[1])
                    {
                        self.sp();
                    } else if !self.closer_ahead(i) {
                        let blank = n >= 2
                            && !self.out.is_empty()
                            && !matches!(
                                self.prev_sig(i).map(|j| self.toks[j].kind),
                                Some(Fk::LBrace) | Some(Fk::LParen) | Some(Fk::LBrack) | None
                            );
                        if !self.start {
                            self.br();
                            if blank {
                                self.nl();
                            }
                        } else if blank {
                            self.nl();
                        }
                    }
                    i += 1;
                }
                Fk::Line => {
                    if !self.start {
                        self.sp();
                    }
                    self.ensure_indent();
                    self.out.push_str(&text);
                    self.nl();
                    i += 1;
                }
                Fk::Block => {
                    if !self.start {
                        self.sp();
                    }
                    self.ensure_indent();
                    let mut lines = text.split('\n');
                    if let Some(first) = lines.next() {
                        self.out.push_str(first);
                    }
                    for line in lines {
                        self.nl();
                        if line.trim().is_empty() {
                            continue;
                        }
                        self.out.push_str(line);
                        self.start = false;
                    }
                    i += 1;
                }
                Fk::LBrace => {
                    if !self.start {
                        self.sp();
                    }
                    self.ensure_indent();
                    self.out.push('{');
                    let pos = self.out.len();
                    self.stack.push((true, self.indent, pos));
                    self.indent += 1;
                    if self.next_sig(i).map(|j| self.toks[j].kind) != Some(Fk::RBrace) {
                        self.nl();
                    }
                    i += 1;
                }
                Fk::RBrace => {
                    let base = self.stack.pop().map(|(_, b, _)| b).unwrap_or(0);
                    self.indent = base;
                    if self.out.ends_with('{') {
                        self.out.push('}');
                        self.start = false;
                    } else {
                        if !self.start {
                            self.nl();
                        }
                        self.ensure_indent();
                        self.out.push('}');
                    }
                    i += 1;
                }
                Fk::LParen => {
                    let strip_it = match self.prev_sig(i) {
                        Some(j) if self.toks[j].kind == Fk::Word => {
                            let w = self.toks[j].text.as_str();
                            (w == "if" || w == "while" || w == "switch")
                                && self.m[i].is_some_and(|c| {
                                    self.next_sig(c).map(|k| self.toks[k].kind) == Some(Fk::LBrace)
                                })
                        }
                        _ => false,
                    };
                    if strip_it {
                        self.stripped.push(i);
                        self.stripped.push(self.m[i].unwrap());
                        i += 1;
                    } else {
                        self.ensure_indent();
                        self.out.push('(');
                        self.stack.push((false, self.indent, self.out.len()));
                        i += 1;
                    }
                }
                Fk::RParen | Fk::RBrack => {
                    self.stack.pop();
                    self.ensure_indent();
                    self.out.push(if kind == Fk::RParen { ')' } else { ']' });
                    i += 1;
                }
                Fk::LBrack => {
                    if !self.start {
                        match self.prev_sig(i).map(|j| self.toks[j].kind) {
                            Some(Fk::LParen) | Some(Fk::LBrack) | Some(Fk::Word)
                            | Some(Fk::Num) | Some(Fk::Str) | Some(Fk::RParen)
                            | Some(Fk::RBrack) | Some(Fk::RBrace) => {}
                            _ => self.sp(),
                        }
                    }
                    self.ensure_indent();
                    self.out.push('[');
                    self.stack.push((false, self.indent, self.out.len()));
                    i += 1;
                }
                Fk::Comma => {
                    self.out.push(',');
                    match self.next_sig(i).map(|j| self.toks[j].kind) {
                        Some(Fk::RBrace) | Some(Fk::RParen) | Some(Fk::RBrack) | None => {}
                        _ => {
                            if self.has_newline_between(i, self.next_sig(i).unwrap()) {
                                self.br();
                            } else {
                                self.sp();
                            }
                        }
                    }
                    i += 1;
                }
                Fk::Semi => {
                    self.out.push(';');
                    if !matches!(
                        self.next_sig(i).map(|j| self.toks[j].kind),
                        Some(Fk::RBrace) | Some(Fk::RParen) | None
                    ) {
                        self.nl();
                    }
                    i += 1;
                }
                Fk::Colon => {
                    self.out.push(':');
                    match self.next_sig(i).map(|j| self.toks[j].kind) {
                        Some(Fk::Semi)
                        | Some(Fk::Comma)
                        | Some(Fk::RParen)
                        | Some(Fk::RBrack)
                        | Some(Fk::RBrace)
                        | Some(Fk::Colon)
                        | None => {}
                        _ => self.sp(),
                    }
                    i += 1;
                }
                Fk::ColonColon | Fk::Dot | Fk::DotDot => {
                    self.ensure_indent();
                    self.out.push_str(&text);
                    i += 1;
                }
                Fk::QuestionQuestion => {
                    if !self.start {
                        self.sp();
                    }
                    self.ensure_indent();
                    self.out.push_str("??");
                    self.sp();
                    i += 1;
                }
                Fk::QuestionDot => {
                    self.ensure_indent();
                    self.out.push_str("?.");
                    i += 1;
                }
                Fk::Question => {
                    self.ensure_indent();
                    self.out.push('?');
                    match self.next_sig(i).map(|j| self.toks[j].kind) {
                        Some(Fk::Semi)
                        | Some(Fk::Comma)
                        | Some(Fk::RParen)
                        | Some(Fk::RBrack)
                        | Some(Fk::RBrace)
                        | Some(Fk::Dot)
                        | None => {}
                        _ => self.sp(),
                    }
                    i += 1;
                }
                Fk::Hash => {
                    if !self.start {
                        self.sp();
                    }
                    self.ensure_indent();
                    self.out.push('#');
                    i += 1;
                }
                Fk::FatArrow | Fk::Arrow => {
                    if !self.start {
                        self.sp();
                    }
                    self.ensure_indent();
                    self.out.push_str(&text);
                    match self.next_sig(i).map(|j| self.toks[j].kind) {
                        Some(Fk::Semi)
                        | Some(Fk::Comma)
                        | Some(Fk::RParen)
                        | Some(Fk::RBrack)
                        | Some(Fk::RBrace)
                        | None => {}
                        _ => self.sp(),
                    }
                    i += 1;
                }
                Fk::Op => {
                    if (text == "<" || text == ">" || text == ">>" || text == ">>>")
                        && self.generic[i]
                    {
                        self.ensure_indent();
                        self.out.push_str(&text);
                        i += 1;
                        continue;
                    }
                    if text == "++" || text == "--" {
                        let postfix = matches!(
                            self.prev_sig(i).map(|j| self.toks[j].kind),
                            Some(Fk::Word)
                                | Some(Fk::Num)
                                | Some(Fk::Str)
                                | Some(Fk::RParen)
                                | Some(Fk::RBrack)
                                | Some(Fk::RBrace)
                        );
                        self.ensure_indent();
                        if !postfix && !self.start {
                            let last = self.out.chars().last();
                            if !matches!(last, Some('(') | Some('[') | Some(' ') | None) {
                                self.sp();
                            }
                        }
                        self.out.push_str(&text);
                        i += 1;
                        continue;
                    }
                    let unary = (text == "-" || text == "!" || text == "&")
                        && !matches!(
                            self.prev_sig(i).map(|j| self.toks[j].kind),
                            Some(Fk::Word)
                                | Some(Fk::Num)
                                | Some(Fk::Str)
                                | Some(Fk::RParen)
                                | Some(Fk::RBrack)
                                | Some(Fk::RBrace)
                        );
                    if !unary {
                        if !self.start
                            && !self.out.ends_with('(')
                            && !self.out.ends_with('[')
                        {
                            self.sp();
                        }
                        self.ensure_indent();
                        self.out.push_str(&text);
                        match self.next_sig(i).map(|j| self.toks[j].kind) {
                            Some(Fk::Semi)
                            | Some(Fk::Comma)
                            | Some(Fk::RParen)
                            | Some(Fk::RBrack)
                            | Some(Fk::RBrace)
                            | Some(Fk::Colon)
                            | Some(Fk::Dot)
                            | Some(Fk::ColonColon)
                            | Some(Fk::Question)
                            | Some(Fk::DotDot)
                            | None => {}
                            _ => self.sp(),
                        }
                    } else {
                        self.ensure_indent();
                        self.out.push_str(&text);
                    }
                    i += 1;
                }
                Fk::Word | Fk::Num | Fk::Str => {
                    self.emit_wordlike(i);
                    i += 1;
                }
            }
        }
    }
}

pub fn format_source(src: &str) -> Result<String, String> {
    let toks = tokenize(src)?;
    let m = match_parens(&toks);
    let generic = match_generic_brackets(&toks);
    let mut e = Emitter {
        toks: &toks,
        m,
        generic,
        out: String::new(),
        indent: 0,
        start: true,
        stack: Vec::new(),
        stripped: Vec::new(),
    };
    e.run();
    while e.out.ends_with(' ') || e.out.ends_with('\t') {
        e.out.pop();
    }
    if !e.out.ends_with('\n') {
        e.out.push('\n');
    }
    Ok(e.out)
}
