use frontend::lexer::lex;
use frontend::token::TokenKind;

fn str_texts(src: &str) -> Vec<String> {
    lex(src)
        .unwrap_or_else(|d| panic!("lex failed: {d}"))
        .into_iter()
        .filter_map(|t| match t.kind {
            TokenKind::StrText(s) => Some(s),
            _ => None,
        })
        .collect()
}

fn lex_str(src: &str) -> String {
    let texts = str_texts(src);
    assert_eq!(texts.len(), 1, "expected one StrText in {src:?}");
    texts.into_iter().next().unwrap()
}

#[test]
fn accented_latin_literals() {
    assert_eq!(lex_str("\"café\""), "café");
    assert_eq!(lex_str("\"naïve\""), "naïve");
    assert_eq!(lex_str("\"résumé\""), "résumé");
}

#[test]
fn multibyte_east_asian_and_devanagari() {
    assert_eq!(lex_str("\"日本語\""), "日本語");
    assert_eq!(lex_str("\"Rasmalai: रसमलाई\""), "Rasmalai: रसमलाई");
}

#[test]
fn four_byte_emoji_literals() {
    assert_eq!(lex_str("\"🚀\""), "🚀");
    assert_eq!(lex_str("\"✨🎉\""), "✨🎉");
}

#[test]
fn url_and_header_strings() {
    assert_eq!(
        lex_str("\"http://127.0.0.1:8080/api/v1/café\""),
        "http://127.0.0.1:8080/api/v1/café"
    );
    assert_eq!(lex_str("\"x-custom-header: 🚀\""), "x-custom-header: 🚀");
}

#[test]
fn ascii_escapes_still_decode() {
    assert_eq!(lex_str("\"a\\nb\\tc\\\"d\\\\e\\{f\""), "a\nb\tc\"d\\e{f");
}

#[test]
fn unicode_escapes_decode_to_scalars() {
    assert_eq!(lex_str("\"\\u{e9}\""), "é");
    assert_eq!(lex_str("\"\\u{1F680}\""), "🚀");
    assert_eq!(lex_str("\"A\\u{41}B\""), "AAB");
}

#[test]
fn bad_unicode_escapes_rejected() {
    for src in [
        "\"\\u{D800}\"",
        "\"\\u{110000}\"",
        "\"\\u{}\"",
        "\"\\u{1234567}\"",
        "\"\\u{e9\"",
        "\"\\ux\"",
    ] {
        assert!(lex(src).is_err(), "expected error for {src:?}");
    }
}

#[test]
fn interpolation_around_unicode() {
    let toks = lex("\"café${x}!\"").unwrap();
    let kinds: Vec<&TokenKind> = toks.iter().map(|t| &t.kind).collect();
    assert!(matches!(kinds[0], TokenKind::StrOpen));
    assert!(matches!(kinds[1], TokenKind::StrText(s) if s == "café"));
    assert!(matches!(kinds[2], TokenKind::InterpOpen));
}

fn float_of(src: &str) -> f64 {
    let toks = lex(src).unwrap_or_else(|d| panic!("lex failed: {d}"));
    assert_eq!(toks.len(), 2, "expected one token plus Eof in {src:?}");
    match toks[0].kind {
        TokenKind::Float(v) => v,
        ref other => panic!("expected Float in {src:?}, got {other:?}"),
    }
}

#[test]
fn scientific_exponent_lexes_as_float() {
    assert_eq!(float_of("1e6"), 1e6);
    assert_eq!(float_of("1E+16"), 1e16);
    assert_eq!(float_of("1e16"), 10000000000000000.0);
    assert_eq!(float_of("2E+8"), 200000000.0);
    assert_eq!(float_of("5e-3"), 0.005);
    assert_eq!(float_of("1.5e3"), 1500.0);
    assert_eq!(float_of("1.5e-4"), 0.00015);
}

#[test]
fn bare_e_without_digits_stays_int_then_ident() {
    let toks = lex("1e").unwrap();
    assert!(matches!(toks[0].kind, TokenKind::Int(1)), "{:?}", toks[0].kind);
    assert!(matches!(toks[1].kind, TokenKind::Ident(_)), "{:?}", toks[1].kind);
}
