use frontend::lexer::lex;
use frontend::parser::Parser;
use frontend::token::TokenKind;

fn doc_texts(src: &str) -> Vec<String> {
    lex(src)
        .unwrap_or_else(|e| panic!("lex failed: {e}"))
        .into_iter()
        .filter_map(|t| match t.kind {
            TokenKind::DocComment(text) => Some(text),
            _ => None,
        })
        .collect()
}

#[test]
fn block_comments_are_skipped() {
    let src = "/* header */\nfn Main(): Int {\n    /* inline */ let x = 1;\n    return x;\n}\n";
    let toks = lex(src).unwrap_or_else(|e| panic!("lex failed: {e}"));
    assert!(!toks.iter().any(|t| matches!(
        t.kind,
        TokenKind::Slash | TokenKind::Star | TokenKind::SlashEq | TokenKind::StarEq
    )));
    Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
}

#[test]
fn multiline_block_comment_is_skipped() {
    let src = "/*\nline one\nline * two\n*/\nfn Main(): Int {\n    return 0;\n}\n";
    Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
}

#[test]
fn unterminated_block_comment_errors() {
    let src = "/* never ends\nfn Main(): Int {\n    return 0;\n}\n";
    assert!(lex(src).is_err());
}

#[test]
fn jsdoc_block_attaches_as_doc() {
    let src = "/** Adds one. */\nfn inc(x: Int): Int {\n    return x;\n}\n";
    let docs = doc_texts(src);
    assert_eq!(docs, vec!["Adds one.".to_string()]);
    let module = Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
    match &module.decls[0].node {
        frontend::ast::Decl::Fn(f) => assert_eq!(f.docs, "Adds one."),
        other => panic!("unexpected decl {other:?}"),
    }
}

#[test]
fn jsdoc_multiline_strips_stars() {
    let src = "/**\n * Adds one.\n * Second line.\n */\nfn inc(x: Int): Int {\n    return x;\n}\n";
    let docs = doc_texts(src);
    assert_eq!(docs, vec!["Adds one.\nSecond line.".to_string()]);
}

#[test]
fn empty_block_is_not_doc() {
    let src = "/**/\nfn Main(): Int {\n    return 0;\n}\n";
    assert!(doc_texts(src).is_empty());
    Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
}

#[test]
fn triple_slash_is_a_plain_comment() {
    let src = "/// Subtracts one.\nfn dec(x: Int): Int {\n    return x;\n}\n";
    assert!(doc_texts(src).is_empty());
    let module = Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
    match &module.decls[0].node {
        frontend::ast::Decl::Fn(f) => assert!(f.docs.is_empty(), "triple slash attached: {:?}", f.docs),
        other => panic!("unexpected decl {other:?}"),
    }
}

#[test]
fn jsdoc_block_attaches_to_interface_and_members() {
    let src = "/** Drawable shape. */\ninterface Shape {\n    /** Draw it. */\n    fn draw(): Int;\n}\n";
    let module = Parser::parse_module(src).unwrap_or_else(|e| panic!("parse failed: {e}"));
    match &module.decls[0].node {
        frontend::ast::Decl::Interface { docs, members, .. } => {
            assert_eq!(docs, "Drawable shape.");
            match &members[0].node {
                frontend::ast::ClassMember::Method(f) => assert_eq!(f.docs, "Draw it."),
                other => panic!("unexpected member {other:?}"),
            }
        }
        other => panic!("unexpected decl {other:?}"),
    }
}
