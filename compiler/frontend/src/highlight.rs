use diagnostics::theme::{AuraColor, AuraTheme};

use crate::lexer::lex;
use crate::token::TokenKind;

fn is_type_name(word: &str) -> bool {
    // Option stays: user-declared types highlight even though std.prelude omits it.
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

const DOC_TAGS: &[&str] = &[
    "returns", "return", "param", "throws", "error", "example", "see", "since", "deprecated",
];

fn paint_doc(theme: &AuraTheme, text: &str) -> String {
    let bytes = text.as_bytes();
    let mut out = String::with_capacity(text.len() + 16);
    let mut start = 0usize;
    let mut i = 0usize;
    while i < bytes.len() {
        let tag_len = if bytes[i] == b'@' {
            let rest = &text[i + 1..];
            DOC_TAGS
                .iter()
                .find(|tag| {
                    rest.starts_with(*tag)
                        && rest[tag.len()..]
                            .chars()
                            .next()
                            .map_or(true, |c| !c.is_alphanumeric() && c != '_')
                })
                .map(|tag| tag.len() + 1)
        } else {
            None
        };
        match tag_len {
            Some(len) => {
                out.push_str(&theme.paint(AuraColor::Green, &text[start..i]));
                out.push_str(&theme.paint(AuraColor::Cyan, &text[i..i + len]));
                i += len;
                start = i;
            }
            None => {
                i += 1;
            }
        }
    }
    out.push_str(&theme.paint(AuraColor::Green, &text[start..]));
    out
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
        | TokenKind::Await
        | TokenKind::Export
        | TokenKind::Pass => Some(AuraColor::Purple),
        TokenKind::Class
        | TokenKind::Struct
        | TokenKind::Record
        | TokenKind::Trait
        | TokenKind::Interface
        | TokenKind::Extension
        | TokenKind::New
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
        TokenKind::Ident(_) | TokenKind::This | TokenKind::Super => Some(AuraColor::Text),
        _ => None,
    }
}

pub fn highlight_line(theme: &AuraTheme, line: &str) -> String {
    let toks = match lex(line) {
        Ok(toks) => toks,
        Err(_) => return line.to_string(),
    };
    if toks.iter().any(|t| matches!(t.kind, TokenKind::DocModule(_))) {
        if let Some(idx) = line.find("//!") {
            let (head, tail) = line.split_at(idx);
            let mut out = highlight_line(theme, head);
            out.push_str(&paint_doc(theme, tail));
            return out;
        }
    }
    let mut out = String::with_capacity(line.len() + 16);
    let mut cursor = 0usize;
    for tok in &toks {
        let start = tok.span.start as usize;
        let end = tok.span.end as usize;
        if start > cursor {
            out.push_str(&line[cursor..start.min(line.len())]);
        }
        let text = &line[start.min(line.len())..end.min(line.len())];
        if matches!(tok.kind, TokenKind::DocComment(_)) {
            out.push_str(&paint_doc(theme, text));
        } else {
            match token_color(&tok.kind) {
                Some(color) => out.push_str(&theme.paint(color, text)),
                None => out.push_str(text),
            }
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

    #[test]
    fn doc_comment_painted_green_with_cyan_tags() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        let out = highlight_line(&theme, "/** @param x the value */");
        assert!(out.contains("\x1b[38;2;130;226;255m@param\x1b[0m"), "{out}");
        assert!(out.contains("\x1b[38;2;97;255;202m"), "{out}");
    }

    #[test]
    fn module_doc_line_painted_green() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        assert_eq!(
            highlight_line(&theme, "//! Module docs."),
            "\x1b[38;2;97;255;202m//! Module docs.\x1b[0m"
        );
    }

    #[test]
    fn triple_slash_stays_plain() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        let line = "/// Subtracts one.";
        assert_eq!(highlight_line(&theme, line), line);
    }

    #[test]
    fn oracle_keywords_share_keyword_color() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        for word in ["export", "pass", "interface", "extension", "new"] {
            let out = highlight_line(&theme, word);
            assert!(
                out.contains(&format!("\x1b[38;2;162;119;255m{word}\x1b[0m")),
                "{word}: {out}"
            );
        }
        let out = highlight_line(&theme, "super");
        assert!(out.contains("\x1b[38;2;237;236;238msuper\x1b[0m"), "{out}");
    }

    #[test]
    fn option_highlighted_as_type() {
        let theme = AuraTheme {
            enabled: true,
            truecolor: true,
        };
        let out = highlight_line(&theme, "let o: Option = 0;");
        assert!(out.contains("\x1b[38;2;246;148;255mOption\x1b[0m"), "{out}");
    }
}
