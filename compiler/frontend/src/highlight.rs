use diagnostics::theme::{AuraColor, AuraTheme};

use crate::lexer::lex;
use crate::token::TokenKind;

fn is_type_name(word: &str) -> bool {
    matches!(
        word,
        "Int"
            | "Float"
            | "FastFloat"
            | "Bool"
            | "Void"
            | "String"
            | "Any"
            | "Array"
            | "Map"
            | "Set"
            | "GenRef"
            | "Vec4f"
            | "Vec4i"
            | "Vec2"
            | "Option"
            | "Result"
    )
}

fn token_color(kind: &TokenKind) -> Option<AuraColor> {
    match kind {
        TokenKind::Fn
        | TokenKind::Let
        | TokenKind::Const
        | TokenKind::Return
        | TokenKind::Public
        | TokenKind::Private
        | TokenKind::Defer
        | TokenKind::Unsafe
        | TokenKind::Static
        | TokenKind::If
        | TokenKind::Else
        | TokenKind::For
        | TokenKind::While
        | TokenKind::In
        | TokenKind::Break
        | TokenKind::Continue
        | TokenKind::Switch
        | TokenKind::Case
        | TokenKind::Default
        | TokenKind::Throw
        | TokenKind::Throws
        | TokenKind::Try
        | TokenKind::Catch
        | TokenKind::Finally
        | TokenKind::Guard
        | TokenKind::Do
        | TokenKind::Fallthrough
        | TokenKind::Is
        | TokenKind::Import
        | TokenKind::From
        | TokenKind::As
        | TokenKind::Async
        | TokenKind::Await => Some(AuraColor::Purple),
        TokenKind::Class
        | TokenKind::Struct
        | TokenKind::Record
        | TokenKind::Trait
        | TokenKind::Enum
        | TokenKind::Init
        | TokenKind::Deinit
        | TokenKind::OnReload
        | TokenKind::Extends
        | TokenKind::With
        | TokenKind::Comptime
        | TokenKind::Native => Some(AuraColor::Purple),
        TokenKind::Int(_)
        | TokenKind::Float(_)
        | TokenKind::StrText(_)
        | TokenKind::StrOpen
        | TokenKind::StrClose
        | TokenKind::True
        | TokenKind::False
        | TokenKind::Null => Some(AuraColor::Orange),
        TokenKind::Ident(name) if is_type_name(name) => Some(AuraColor::Pink),
        TokenKind::Ident(_) | TokenKind::This => Some(AuraColor::Text),
        _ => None,
    }
}

pub fn highlight_line(theme: &AuraTheme, line: &str) -> String {
    let toks = match lex(line) {
        Ok(toks) => toks,
        Err(_) => return line.to_string(),
    };
    let mut out = String::with_capacity(line.len() + 16);
    let mut cursor = 0usize;
    for tok in &toks {
        let start = tok.span.start as usize;
        let end = tok.span.end as usize;
        if start > cursor {
            out.push_str(&line[cursor..start.min(line.len())]);
        }
        let text = &line[start.min(line.len())..end.min(line.len())];
        match token_color(&tok.kind) {
            Some(color) => out.push_str(&theme.paint(color, text)),
            None => out.push_str(text),
        }
        cursor = end.max(cursor);
    }
    if cursor < line.len() {
        out.push_str(&line[cursor..]);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keywords_types_and_literals_colored() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        let out = highlight_line(&theme, "let rate: Float = 42;");
        assert!(out.contains("\x1b[38;2;162;119;255mlet\x1b[0m"), "{out}");
        assert!(out.contains("\x1b[38;2;246;148;255mFloat\x1b[0m"), "{out}");
        assert!(out.contains("\x1b[38;2;255;202;133m42\x1b[0m"), "{out}");
    }

    #[test]
    fn plain_theme_passes_through() {
        let theme = AuraTheme::plain();
        let line = "fn add(a: Int, b: Int): Int { return a + b; }";
        assert_eq!(highlight_line(&theme, line), line);
    }

    #[test]
    fn unlexable_line_passes_through() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        let line = "let s = \"unclosed;";
        assert_eq!(highlight_line(&theme, line), line);
    }
}
