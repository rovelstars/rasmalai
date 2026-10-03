use diagnostics::{ALL, Code, Diagnostic, Span};

#[test]
fn registry_covers_spec_codes() {
    let mut got: Vec<&str> = ALL.iter().map(|c| c.as_str()).collect();
    got.sort();
    assert_eq!(
        got,
        vec![
            "E005", "E105", "E107", "E108", "E109", "E110", "E111", "E112", "E201", "E202", "E203", "E204", "E205", "E206", "E302", "E303", "E304", "E305", "E402", "E501", "S101", "S102", "S201", "S301", "S401", "S501", "W104", "W108", "W109", "W201", "W204"
        ]
    );
}

#[test]
fn code_round_trips_through_str() {
    for code in ALL {
        assert_eq!(Code::from_str(code.as_str()), Some(*code));
    }
    assert_eq!(Code::from_str("E999"), None);
}

#[test]
fn warnings_are_warnings_only() {
    for code in ALL {
        let expect_warning = matches!(
            code,
            Code::W104 | Code::W108 | Code::W109 | Code::W201 | Code::W204
        );
        assert_eq!(code.is_warning(), expect_warning, "{}", code);
    }
}

#[test]
fn span_rejects_inverted_range() {
    assert!(Span::new(4, 2).is_none());
    assert_eq!(Span::new(2, 2), Some(Span { start: 2, end: 2 }));
}

#[test]
fn display_carries_code_and_hint() {
    let d = Diagnostic::new(Code::E110, "for (i = 0;;)")
        .with_span(Span { start: 0, end: 12 })
        .with_hint("use for i in 0..n or while");
    let s = d.to_string();
    assert!(s.contains("error[E110]"), "{s}");
    assert!(s.contains("0..12"), "{s}");
    assert!(s.contains("while"), "{s}");
}
