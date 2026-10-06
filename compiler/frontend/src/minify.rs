use crate::token::TokenKind;
use diagnostics::{Code, Diagnostic};

#[derive(Clone, Copy, PartialEq, Eq)]
enum Frame {
    Code,
    Str,
    Interp,
}

fn is_word_byte(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

fn is_op_byte(b: u8) -> bool {
    matches!(
        b,
        b'+' | b'-'
            | b'*'
            | b'/'
            | b'%'
            | b'<'
            | b'>'
            | b'='
            | b'!'
            | b'&'
            | b'|'
            | b'?'
            | b':'
            | b'.'
            | b'^'
            | b'~'
    )
}

fn needs_space(prev: u8, next: u8) -> bool {
    (is_word_byte(prev) && is_word_byte(next)) || (is_op_byte(prev) && is_op_byte(next))
}

fn lexeme<'a>(src: &'a str, start: u32, end: u32) -> Result<&'a str, Diagnostic> {
    let (s, e) = (start as usize, end as usize);
    if s > e {
        return Err(Diagnostic::new(Code::E108, "minify hit a reversed token span"));
    }
    src.get(s..e).ok_or_else(|| {
        Diagnostic::new(Code::E108, "minify hit a token span outside the source text")
    })
}

pub fn minify_source(src: &str) -> Result<String, Diagnostic> {
    let toks = crate::lexer::lex(src)?;
    let mut out = String::new();
    let mut frames: Vec<Frame> = vec![Frame::Code];
    for tok in &toks {
        match &tok.kind {
            TokenKind::Eof | TokenKind::DocComment(_) | TokenKind::DocModule(_) => continue,
            _ => {}
        }
        let in_str_text = matches!(frames.last(), Some(Frame::Str));
        let text = lexeme(src, tok.span.start, tok.span.end)?;
        if text.is_empty() {
            return Err(Diagnostic::new(Code::E108, "minify hit an empty token"));
        }
        if !out.is_empty()
            && !in_str_text
            && let (Some(&prev), Some(&next)) = (out.as_bytes().last(), text.as_bytes().first())
            && needs_space(prev, next)
        {
            out.push(' ');
        }
        out.push_str(text);
        match tok.kind {
            TokenKind::StrOpen => frames.push(Frame::Str),
            TokenKind::StrClose => {
                frames.pop();
            }
            TokenKind::InterpOpen => frames.push(Frame::Interp),
            TokenKind::InterpClose => {
                frames.pop();
            }
            _ => {}
        }
    }
    Ok(out)
}

fn format_for_check(src: &str, rel: &str) -> Result<String, Diagnostic> {
    crate::fmt::format_source(src).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot format {rel} for the publish check: {e}"))
    })
}

fn token_signature(src: &str, rel: &str) -> Result<Vec<String>, Diagnostic> {
    let toks = crate::lexer::lex(src).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot re-lex {rel} for the publish check: {}", e.message))
    })?;
    Ok(toks
        .iter()
        .filter(|t| {
            !matches!(
                t.kind,
                TokenKind::Eof | TokenKind::DocComment(_) | TokenKind::DocModule(_)
            )
        })
        .map(|t| format!("{:?}", t.kind))
        .collect())
}

pub fn verify_and_minify(rel: &str, src: &str) -> Result<String, Diagnostic> {
    let formatted = format_for_check(src, rel)?;
    if formatted != src {
        return Err(Diagnostic::new(
            Code::E108,
            format!("refusing to publish unformatted file: {rel}"),
        )
        .with_hint("run `rnx fmt` then retry"));
    }
    let min = minify_source(&formatted).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot minify {rel}: {}", e.message))
    })?;
    let reformatted = format_for_check(&min, rel)?;
    if token_signature(&reformatted, rel)? != token_signature(&formatted, rel)? {
        return Err(Diagnostic::new(
            Code::E108,
            format!("refusing to publish {rel}: minify round-trip mismatch"),
        )
        .with_hint("this is a compiler bug: report the file and the `rnx fmt` output"));
    }
    let stable = format_for_check(&reformatted, rel)?;
    if stable != reformatted {
        return Err(Diagnostic::new(
            Code::E108,
            format!("refusing to publish {rel}: formatter unstable on minified output"),
        )
        .with_hint("this is a compiler bug: report the file and the `rnx fmt` output"));
    }
    let min_again = minify_source(&min).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot re-minify {rel}: {}", e.message))
    })?;
    if min_again != min {
        return Err(Diagnostic::new(
            Code::E108,
            format!("refusing to publish {rel}: minify is not idempotent"),
        )
        .with_hint("this is a compiler bug: report the file and the `rnx fmt` output"));
    }
    Ok(min)
}

pub fn is_minified_rel(rel: &str) -> bool {
    rel.ends_with(".rnx")
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIXTURE: &str = "// leading comment\n/** Doc for add. */\n//! module docs\npub fn add_one(x: Int): Int {\n    // increment\n    return x + 1; /* trailing */\n}\n";

    #[test]
    fn minify_exact_output() {
        let formatted = crate::fmt::format_source(FIXTURE).unwrap();
        let min = minify_source(&formatted).unwrap();
        assert_eq!(min, "pub fn add_one(x:Int):Int{return x+1;}");
    }

    #[test]
    fn minify_drops_all_comment_shapes() {
        let src = "//! module note\n/** Doc for f. */\n// plain line\n/* plain block */\nfn f(): Int {\n    return 1; // trailing\n}\n";
        let min = minify_source(src).unwrap();
        assert_eq!(min, "fn f():Int{return 1;}");
        assert!(!min.contains("//"));
        assert!(!min.contains("/*"));
        assert!(!min.contains("*/"));
        assert!(!min.contains("Doc for"));
        assert!(!min.contains("module note"));
    }

    #[test]
    fn minify_keeps_comment_like_string_text() {
        let src = "fn f(): String {\n    return \"/* not a comment //\";\n}\n";
        let min = minify_source(src).unwrap();
        assert!(min.contains("\"/* not a comment //\""), "{min}");
    }

    #[test]
    fn minify_keeps_string_interpolation_exact() {
        let src = "fn f(name: String): String {\n    return \"hello ${name}!\";\n}\n";
        let min = minify_source(src).unwrap();
        assert_eq!(min, "fn f(name:String):String{return\"hello ${name}!\";}");
        assert_eq!(minify_source(&min).unwrap(), min);
    }

    #[test]
    fn minify_needs_space_between_word_tokens() {
        let min = minify_source("let x = 1;\n").unwrap();
        assert_eq!(min, "let x=1;");
    }

    #[test]
    fn minify_never_joins_slashes_into_a_comment() {
        let min = minify_source("fn f(x: Int): Int {\n    return x / 2;\n}\n").unwrap();
        assert_eq!(min, "fn f(x:Int):Int{return x/2;}");
        let toks_before = crate::lexer::lex("fn f(x: Int): Int { return x / 2; }").unwrap();
        let toks_after = crate::lexer::lex(&min).unwrap();
        let kinds_before: Vec<String> =
            toks_before.iter().map(|t| format!("{:?}", t.kind)).collect();
        let kinds_after: Vec<String> =
            toks_after.iter().map(|t| format!("{:?}", t.kind)).collect();
        assert_eq!(kinds_before, kinds_after);
    }

    #[test]
    fn minify_never_joins_dashes_into_dec() {
        let src = "fn f(x: Int): Int {\n    return x - -1;\n}\n";
        let min = minify_source(src).unwrap();
        assert!(min.contains("- -1"), "{min}");
        let kinds_before: Vec<String> = crate::lexer::lex(src)
            .unwrap()
            .iter()
            .map(|t| format!("{:?}", t.kind))
            .collect();
        let kinds_after: Vec<String> = crate::lexer::lex(&min)
            .unwrap()
            .iter()
            .map(|t| format!("{:?}", t.kind))
            .collect();
        assert_eq!(kinds_before, kinds_after);
    }

    #[test]
    fn verify_round_trip_on_fixture() {
        let formatted = crate::fmt::format_source(FIXTURE).unwrap();
        let min = verify_and_minify("src/main.rnx", &formatted).unwrap();
        assert_eq!(min, "pub fn add_one(x:Int):Int{return x+1;}");
    }

    #[test]
    fn verify_refuses_unformatted_input() {
        let err = verify_and_minify("src/main.rnx", "fn Main():Int{return 0;}").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("src/main.rnx"), "{}", err.message);
    }

    #[test]
    fn round_trip_holds_on_stdlib_sources() {
        let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("..")
            .join("stdlib")
            .join("src");
        let mut files = Vec::new();
        let mut dirs = vec![root];
        while let Some(dir) = dirs.pop() {
            for entry in std::fs::read_dir(&dir).unwrap() {
                let path = entry.unwrap().path();
                if path.is_dir() {
                    dirs.push(path);
                } else if path.extension().is_some_and(|e| e == "rnx") {
                    files.push(path);
                }
            }
        }
        assert!(!files.is_empty(), "no stdlib sources found");
        files.sort();
        for path in &files {
            let src = std::fs::read_to_string(path).unwrap();
            let formatted = format_for_check(&src, &path.display().to_string()).unwrap();
            verify_and_minify(&path.display().to_string(), &formatted).unwrap();
        }
    }
}
