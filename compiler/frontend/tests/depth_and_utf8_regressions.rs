use diagnostics::Code;
use frontend::check::check_source;
use frontend::lexer::lex;
use frontend::parser::Parser;

fn nesting_error(errs: &[diagnostics::Diagnostic]) -> bool {
    errs.iter().any(|d| {
        d.code == Code::E108
            && d.message.contains("expression nesting is too deep")
            && d.span.is_some()
    })
}

#[test]
fn walker_bounds_member_index_and_cast_chains() {
    // Long enough to exceed the walker budget, short enough that the
    // unrelated taint pass, which has its own recursion, is not the subject.
    let chains = [
        format!("fn Main(): Int {{ let a = 1\n    return a{}\n}}", ".x".repeat(100)),
        format!("fn Main(): Int {{ let a = 1\n    return a{}\n}}", "[0]".repeat(100)),
        format!("fn Main(): Int {{ return 1{} }}", " as Int".repeat(100)),
    ];
    for src in chains {
        let errs = match check_source(&src) {
            Ok(_) => panic!("expected a nesting diagnostic"),
            Err(errs) => errs,
        };
        assert!(nesting_error(&errs), "{errs:?}");
    }
}

#[test]
fn walker_survives_the_reported_chain_lengths() {
    let chains = [
        format!("fn Main(): Int {{ let a = 1\n    return a{}\n}}", ".x".repeat(1500)),
        format!("fn Main(): Int {{ let a = 1\n    return a{}\n}}", "[0]".repeat(1500)),
        format!("fn Main(): Int {{ return 1{} }}", " as Int".repeat(1500)),
    ];
    for src in chains {
        let module = Parser::parse_module(&src).unwrap_or_else(|e| panic!("{e}"));
        let _ = frontend::semantic::prelude_uses(&module);
    }
}

#[test]
fn parser_bounds_every_prefix_operator() {
    let prefixes = ["! ", "- ", "* ", "& ", "~ ", "await ", "++ ", "-- "];
    for prefix in prefixes {
        let src = format!("fn F() {{ return {}x }}", prefix.repeat(1000));
        let err = Parser::parse_module(&src)
            .expect_err(&format!("{prefix:?} chain must be rejected"));
        assert_eq!(err.code, Code::E108, "{prefix:?}: {err}");
        assert!(
            err.message.contains("expression nesting is too deep"),
            "{prefix:?}: {err}"
        );
        assert!(err.span.is_some(), "{prefix:?}: {err}");
    }
}

#[test]
fn parser_still_accepts_shallow_prefix_chains() {
    for prefix in ["! ", "- ", "* ", "& ", "~ ", "await ", "++ ", "-- "] {
        let src = format!("fn F() {{ return {}x }}", prefix.repeat(8));
        Parser::parse_module(&src).unwrap_or_else(|e| panic!("{prefix:?} chain: {e}"));
    }
}

#[test]
fn lexer_reports_the_real_non_ascii_scalar() {
    let err = lex("let h\u{e9}llo").expect_err("non-ASCII identifier must be rejected");
    assert_eq!(err.code, Code::E108);
    assert!(err.message.contains('\u{e9}'), "{}", err.message);
    assert!(!err.message.contains('\u{c3}'), "{}", err.message);
    let span = err.span.expect("diagnostic must carry a span");
    assert_eq!((span.start, span.end), (5, 7));

    let err = lex("fn \u{1F600}").expect_err("non-ASCII identifier must be rejected");
    assert!(err.message.contains('\u{1F600}'), "{}", err.message);
    let span = err.span.expect("diagnostic must carry a span");
    assert_eq!((span.start, span.end), (3, 7));
}

#[test]
fn lexer_ascii_diagnostics_keep_single_byte_spans() {
    let err = lex("let x = @").expect_err("stray ASCII character must be rejected");
    assert_eq!(err.code, Code::E108);
    assert!(err.message.contains('@'), "{}", err.message);
    let span = err.span.expect("diagnostic must carry a span");
    assert_eq!((span.start, span.end), (8, 9));
}
