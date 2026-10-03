use crate::token::{Token, TokenKind, keyword};
use diagnostics::{Code, Diagnostic, Span};

enum Frame {
    Code,
    Interp(usize),
    Str,
}

pub fn lex(src: &str) -> Result<Vec<Token>, Diagnostic> {
    let bytes = src.as_bytes();
    let mut toks: Vec<Token> = Vec::new();
    let mut frames: Vec<Frame> = vec![Frame::Code];
    let mut i = 0usize;
    let mut text = String::new();
    let mut text_start = 0usize;

    let span = |s: usize, e: usize| Span {
        start: s as u32,
        end: e as u32,
    };
    let push = |toks: &mut Vec<Token>, kind: TokenKind, s: usize, e: usize| {
        toks.push(Token {
            kind,
            span: span(s, e),
        })
    };
    let flush_text = |toks: &mut Vec<Token>, text: &mut String, s: usize, e: usize| {
        if !text.is_empty() {
            let t = std::mem::take(text);
            toks.push(Token {
                kind: TokenKind::StrText(t),
                span: span(s, e),
            });
        }
    };

    while i < bytes.len() {
        let c = bytes[i] as char;
        let in_str = matches!(frames.last(), Some(Frame::Str));
        if in_str {
            // Decode one full UTF-8 scalar. Stepping byte-by-byte here
            // would split multi-byte sequences, storing each raw byte
            // as U+00xx and double-encoding the string.
            let ch = src[i..].chars().next().unwrap();
            let ch_len = ch.len_utf8();
            match ch {
                '"' => {
                    flush_text(&mut toks, &mut text, text_start, i);
                    push(&mut toks, TokenKind::StrClose, i, i + 1);
                    frames.pop();
                    i += 1;
                }
                '{' => {
                    flush_text(&mut toks, &mut text, text_start, i);
                    push(&mut toks, TokenKind::InterpOpen, i, i + 1);
                    frames.push(Frame::Interp(0));
                    i += 1;
                }
                '$' => {
                    if bytes.get(i + 1) == Some(&b'{') {
                        flush_text(&mut toks, &mut text, text_start, i);
                        push(&mut toks, TokenKind::InterpOpen, i, i + 2);
                        frames.push(Frame::Interp(0));
                        i += 2;
                    } else {
                        if text.is_empty() {
                            text_start = i;
                        }
                        text.push(ch);
                        i += ch_len;
                    }
                }
                '\\' => {
                    if i + 1 >= bytes.len() {
                        return Err(Diagnostic::new(Code::E108, "dangling backslash")
                            .with_span(span(i, i + 1)));
                    }
                    let e = src[i + 1..].chars().next().unwrap();
                    match e {
                        'n' => text.push('\n'),
                        't' => text.push('\t'),
                        '"' => text.push('"'),
                        '\\' => text.push('\\'),
                        '{' => text.push('{'),
                        'u' => {
                            let hex_start = i + 2;
                            if bytes.get(hex_start) != Some(&b'{') {
                                return Err(Diagnostic::new(
                                    Code::E108,
                                    "bad escape \\u (expected \\u{...})",
                                )
                                .with_span(span(i, i + 2)));
                            }
                            let mut j = hex_start + 1;
                            while j < bytes.len() && bytes[j].is_ascii_hexdigit() {
                                j += 1;
                            }
                            if j == hex_start + 1
                                || j - (hex_start + 1) > 6
                                || bytes.get(j) != Some(&b'}')
                            {
                                return Err(Diagnostic::new(
                                    Code::E108,
                                    "bad unicode escape (expected \\u{1-6 hex digits})",
                                )
                                .with_span(span(i, j + 1)));
                            }
                            let cp = u32::from_str_radix(&src[hex_start + 1..j], 16)
                                .map_err(|_| {
                                    Diagnostic::new(Code::E108, "bad unicode escape")
                                        .with_span(span(i, j + 1))
                                })?;
                            let decoded = char::from_u32(cp).ok_or_else(|| {
                                Diagnostic::new(Code::E108, "invalid unicode scalar")
                                    .with_span(span(i, j + 1))
                            })?;
                            text.push(decoded);
                            i = j + 1;
                            continue;
                        }
                        _ => {
                            return Err(Diagnostic::new(
                                Code::E108,
                                format!("bad escape \\{e}"),
                            )
                            .with_span(span(i, i + 1 + e.len_utf8())));
                        }
                    }
                    i += 2;
                }
                _ => {
                    if text.is_empty() {
                        text_start = i;
                    }
                    text.push(ch);
                    i += ch_len;
                }
            }
            continue;
        }
        match c {
            ' ' | '\t' | '\r' | '\n' => {
                i += 1;
            }
            '/' if bytes.get(i + 1) == Some(&b'*') => {
                let doc = bytes.get(i + 2) == Some(&b'*') && bytes.get(i + 3) != Some(&b'/');
                let content_start = if doc { i + 3 } else { i + 2 };
                let mut j = content_start;
                while j + 1 < bytes.len() && !(bytes[j] == b'*' && bytes[j + 1] == b'/') {
                    j += 1;
                }
                if j + 1 >= bytes.len() {
                    return Err(Diagnostic::new(Code::E108, "unterminated block comment")
                        .with_span(span(i, bytes.len())));
                }
                if doc {
                    let raw = String::from_utf8_lossy(&bytes[content_start..j]).into_owned();
                    let mut lines = Vec::new();
                    for (n, line) in raw.split('\n').enumerate() {
                        let trimmed = line.trim_end();
                        if n == 0 && trimmed.trim().is_empty() {
                            continue;
                        }
                        let no_star = trimmed.trim_start();
                        let no_star = no_star.strip_prefix('*').unwrap_or(no_star);
                        lines.push(no_star.strip_prefix(' ').unwrap_or(no_star).to_string());
                    }
                    while lines.last().is_some_and(|l| l.is_empty()) {
                        lines.pop();
                    }
                    push(&mut toks, TokenKind::DocComment(lines.join("\n")), i, j + 2);
                }
                i = j + 2;
            }
            '/' if bytes.get(i + 1) == Some(&b'/') => {
                if bytes.get(i + 2) == Some(&b'!') {
                    let mut parts = Vec::new();
                    loop {
                        let mut j = i + 3;
                        while j < bytes.len() && bytes[j] != b'\n' {
                            j += 1;
                        }
                        let mut line = bytes[i + 3..j].to_vec();
                        if line.first() == Some(&b' ') {
                            line.remove(0);
                        }
                        parts.push(String::from_utf8_lossy(&line).into_owned());
                        i = j;
                        if i < bytes.len() && bytes[i] == b'\n' {
                            i += 1;
                        }
                        if !(i + 2 < bytes.len()
                            && bytes[i] == b'/'
                            && bytes[i + 1] == b'/'
                            && bytes[i + 2] == b'!')
                        {
                            break;
                        }
                    }
                    push(&mut toks, TokenKind::DocModule(parts.join("\n")), i, i);
                } else {
                    while i < bytes.len() && bytes[i] != b'\n' {
                        i += 1;
                    }
                }
            }
            '"' => {
                push(&mut toks, TokenKind::StrOpen, i, i + 1);
                frames.push(Frame::Str);
                text.clear();
                text_start = i + 1;
                i += 1;
            }
            '{' => {
                if let Some(Frame::Interp(d)) = frames.last_mut() {
                    *d += 1;
                }
                push(&mut toks, TokenKind::LBrace, i, i + 1);
                i += 1;
            }
            '}' => {
                match frames.last_mut() {
                    Some(Frame::Interp(0)) => {
                        frames.pop();
                        push(&mut toks, TokenKind::InterpClose, i, i + 1);
                    }
                    Some(Frame::Interp(d)) => {
                        *d -= 1;
                        push(&mut toks, TokenKind::RBrace, i, i + 1);
                    }
                    _ => push(&mut toks, TokenKind::RBrace, i, i + 1),
                }
                i += 1;
            }
            '(' => {
                push(&mut toks, TokenKind::LParen, i, i + 1);
                i += 1;
            }
            ')' => {
                push(&mut toks, TokenKind::RParen, i, i + 1);
                i += 1;
            }
            '[' => {
                push(&mut toks, TokenKind::LBracket, i, i + 1);
                i += 1;
            }
            ']' => {
                push(&mut toks, TokenKind::RBracket, i, i + 1);
                i += 1;
            }
            ',' => {
                push(&mut toks, TokenKind::Comma, i, i + 1);
                i += 1;
            }
            ';' => {
                push(&mut toks, TokenKind::Semi, i, i + 1);
                i += 1;
            }
            '?' => {
                if bytes.get(i + 1) == Some(&b'?') {
                    push(&mut toks, TokenKind::QuestionQuestion, i, i + 2);
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'.') {
                    push(&mut toks, TokenKind::QuestionDot, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Question, i, i + 1);
                    i += 1;
                }
            }
            '#' => {
                push(&mut toks, TokenKind::Hash, i, i + 1);
                i += 1;
            }
            '.' => {
                if bytes.get(i + 1) == Some(&b'.') {
                    if bytes.get(i + 2) == Some(&b'.') {
                        push(&mut toks, TokenKind::Ellipsis, i, i + 3);
                        i += 3;
                    } else if bytes.get(i + 2) == Some(&b'=') {
                        push(&mut toks, TokenKind::DotDotEq, i, i + 3);
                        i += 3;
                    } else {
                        push(&mut toks, TokenKind::DotDot, i, i + 2);
                        i += 2;
                    }
                } else {
                    push(&mut toks, TokenKind::Dot, i, i + 1);
                    i += 1;
                }
            }
            ':' => {
                if bytes.get(i + 1) == Some(&b':') {
                    return Err(Diagnostic::new(
                        Code::E005,
                        "'::' is not valid syntax in Rasmalai; use '.' for namespacing and member access",
                    )
                    .with_span(span(i, i + 2)));
                } else {
                    push(&mut toks, TokenKind::Colon, i, i + 1);
                    i += 1;
                }
            }
            '=' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::EqEq, i, i + 2);
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'>') {
                    push(&mut toks, TokenKind::FatArrow, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Eq, i, i + 1);
                    i += 1;
                }
            }
            '!' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::BangEq, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Bang, i, i + 1);
                    i += 1;
                }
            }
            '<' => {
                if bytes.get(i + 1) == Some(&b'<') {
                    if bytes.get(i + 2) == Some(&b'=') {
                        push(&mut toks, TokenKind::ShlEq, i, i + 3);
                        i += 3;
                    } else {
                        push(&mut toks, TokenKind::Shl, i, i + 2);
                        i += 2;
                    }
                } else if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::LtEq, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Lt, i, i + 1);
                    i += 1;
                }
            }
            '>' => {
                if bytes.get(i + 1) == Some(&b'>') {
                    if bytes.get(i + 2) == Some(&b'>') {
                        if bytes.get(i + 3) == Some(&b'=') {
                            push(&mut toks, TokenKind::ZshrEq, i, i + 4);
                            i += 4;
                        } else {
                            push(&mut toks, TokenKind::Zshr, i, i + 3);
                            i += 3;
                        }
                    } else if bytes.get(i + 2) == Some(&b'=') {
                        push(&mut toks, TokenKind::ShrEq, i, i + 3);
                        i += 3;
                    } else {
                        push(&mut toks, TokenKind::Shr, i, i + 2);
                        i += 2;
                    }
                } else if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::GtEq, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Gt, i, i + 1);
                    i += 1;
                }
            }
            '+' => {
                if bytes.get(i + 1) == Some(&b'+') {
                    push(&mut toks, TokenKind::PlusPlus, i, i + 2);
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::PlusEq, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Plus, i, i + 1);
                    i += 1;
                }
            }
            '-' => {
                if bytes.get(i + 1) == Some(&b'-') {
                    push(&mut toks, TokenKind::MinusMinus, i, i + 2);
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::MinusEq, i, i + 2);
                    i += 2;
                } else if bytes.get(i + 1) == Some(&b'>') {
                    push(&mut toks, TokenKind::Arrow, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Minus, i, i + 1);
                    i += 1;
                }
            }
            '*' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::StarEq, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Star, i, i + 1);
                    i += 1;
                }
            }
            '/' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::SlashEq, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Slash, i, i + 1);
                    i += 1;
                }
            }
            '%' => {
                if bytes.get(i + 1) == Some(&b'=') {
                    push(&mut toks, TokenKind::PercentEq, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Percent, i, i + 1);
                    i += 1;
                }
            }
            '&' => {
                if bytes.get(i + 1) == Some(&b'&') {
                    push(&mut toks, TokenKind::AmpAmp, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Amp, i, i + 1);
                    i += 1;
                }
            }
            '|' => {
                if bytes.get(i + 1) == Some(&b'|') {
                    push(&mut toks, TokenKind::PipePipe, i, i + 2);
                    i += 2;
                } else {
                    push(&mut toks, TokenKind::Pipe, i, i + 1);
                    i += 1;
                }
            }
            '^' => {
                push(&mut toks, TokenKind::Caret, i, i + 1);
                i += 1;
            }
            '~' => {
                push(&mut toks, TokenKind::Tilde, i, i + 1);
                i += 1;
            }
            '0'..='9' => {
                let s = i;
                if c == '0' && matches!(bytes.get(i + 1), Some(b'x') | Some(b'X')) {
                    i += 2;
                    let d = i;
                    while i < bytes.len()
                        && (bytes[i].is_ascii_hexdigit() || bytes[i] == b'_')
                    {
                        i += 1;
                    }
                    let raw: String = src[d..i].bytes().filter(|&b| b != b'_').map(|b| b as char).collect();
                    if raw.is_empty() {
                        return Err(Diagnostic::new(Code::E108, "bad hex literal").with_span(span(s, i)));
                    }
                    let v = u64::from_str_radix(&raw, 16).map_err(|_| {
                        Diagnostic::new(Code::E108, "bad hex literal").with_span(span(s, i))
                    })? as i64;
                    push(&mut toks, TokenKind::Int(v), s, i);
                    continue;
                }
                if c == '0' && matches!(bytes.get(i + 1), Some(b'b') | Some(b'B')) {
                    i += 2;
                    let d = i;
                    while i < bytes.len()
                        && (bytes[i] == b'0' || bytes[i] == b'1' || bytes[i] == b'_')
                    {
                        i += 1;
                    }
                    let raw: String = src[d..i].bytes().filter(|&b| b != b'_').map(|b| b as char).collect();
                    if raw.is_empty() {
                        return Err(Diagnostic::new(Code::E108, "bad binary literal").with_span(span(s, i)));
                    }
                    let v = u64::from_str_radix(&raw, 2).map_err(|_| {
                        Diagnostic::new(Code::E108, "bad binary literal").with_span(span(s, i))
                    })? as i64;
                    push(&mut toks, TokenKind::Int(v), s, i);
                    continue;
                }
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'_') {
                    i += 1;
                }
                let is_float_dot = bytes.get(i) == Some(&b'.')
                    && !matches!(bytes.get(i + 1), Some(b'.'))
                    && matches!(bytes.get(i + 1), Some(b'0'..=b'9'));
                let mut j = i;
                let mut frac_end = i;
                if is_float_dot {
                    j += 1;
                    while j < bytes.len() && (bytes[j].is_ascii_digit() || bytes[j] == b'_') {
                        j += 1;
                    }
                    frac_end = j;
                }
                let mut exp_end: Option<usize> = None;
                if matches!(bytes.get(j), Some(b'e') | Some(b'E')) {
                    let mut k = j + 1;
                    if matches!(bytes.get(k), Some(b'+') | Some(b'-')) {
                        k += 1;
                    }
                    let d = k;
                    while k < bytes.len() && (bytes[k].is_ascii_digit() || bytes[k] == b'_') {
                        k += 1;
                    }
                    if k > d {
                        exp_end = Some(k);
                    }
                }
                if is_float_dot || exp_end.is_some() {
                    i = exp_end.unwrap_or(frac_end);
                    let raw: String = src[s..i].bytes().filter(|&b| b != b'_').map(|b| b as char).collect();
                    let v: f64 = raw.parse().map_err(|_| {
                        Diagnostic::new(Code::E108, "bad float literal").with_span(span(s, i))
                    })?;
                    push(&mut toks, TokenKind::Float(v), s, i);
                } else {
                    let raw: String = src[s..i].bytes().filter(|&b| b != b'_').map(|b| b as char).collect();
                    let v: i64 = raw.parse().map_err(|_| {
                        Diagnostic::new(Code::E108, "bad int literal").with_span(span(s, i))
                    })?;
                    push(&mut toks, TokenKind::Int(v), s, i);
                }
            }
            'A'..='Z' | 'a'..='z' | '_' => {
                let s = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                let word = &src[s..i];
                match keyword(word) {
                    Some(k) => push(&mut toks, k, s, i),
                    None => push(&mut toks, TokenKind::Ident(word.to_string()), s, i),
                }
            }
            _ => {
                let in_interp = frames.iter().any(|f| matches!(f, Frame::Str));
                let mut d = Diagnostic::new(
                    Code::E108,
                    if in_interp {
                        format!("unexpected character `{c}` in `{{...}}` interpolation")
                    } else {
                        format!("unexpected character `{c}`")
                    },
                )
                .with_span(span(i, i + 1));
                if in_interp {
                    d = d.with_hint("use `\\{` for a literal brace, or `${...}` for interpolation");
                }
                return Err(d);
            }
        }
    }
    if !matches!(frames.last(), Some(Frame::Code)) {
        return Err(Diagnostic::new(Code::E108, "unterminated string")
            .with_span(span(src.len().saturating_sub(1), src.len())));
    }
    push(&mut toks, TokenKind::Eof, src.len(), src.len());
    Ok(toks)
}
