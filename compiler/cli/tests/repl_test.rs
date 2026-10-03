use cli::repl::{ReplOut, ReplSession};

fn lines_of(session: &mut ReplSession, input: &str) -> Vec<String> {
    match session.eval(input) {
        ReplOut::Lines(lines) => lines,
        ReplOut::Exit => panic!("unexpected exit for {input:?}"),
    }
}

fn contains(lines: &[String], want: &str) -> bool {
    lines.iter().any(|l| l.contains(want))
}

#[test]
fn test_expr_eval() {
    let mut s = ReplSession::new();
    let out = lines_of(&mut s, "1 + 2");
    assert!(contains(&out, "3"), "{out:?}");
}

#[test]
fn test_fn_define_and_call() {
    let mut s = ReplSession::new();
    let out = lines_of(&mut s, "fn square(x: Int): Int { return x * x; }");
    assert!(contains(&out, "square"), "{out:?}");
    let out = lines_of(&mut s, "square(8)");
    assert!(contains(&out, "64"), "{out:?}");
}

#[test]
fn test_var_retention() {
    let mut s = ReplSession::new();
    lines_of(&mut s, "let a = 15;");
    lines_of(&mut s, "let b = 25;");
    let out = lines_of(&mut s, "a + b");
    assert!(contains(&out, "40"), "{out:?}");
}

#[test]
fn test_error_resilience() {
    let mut s = ReplSession::new();
    lines_of(&mut s, "let valid = 100;");
    let out = lines_of(&mut s, "this is bad syntax {");
    assert!(!out.is_empty(), "diagnostic expected");
    let out = lines_of(&mut s, "valid");
    assert!(contains(&out, "100"), "{out:?}");
}

#[test]
fn test_assign_and_reset() {
    let mut s = ReplSession::new();
    lines_of(&mut s, "let n = 1;");
    lines_of(&mut s, "n = 41;");
    let out = lines_of(&mut s, "n + 1");
    assert!(contains(&out, "42"), "{out:?}");
    lines_of(&mut s, "n += 8;");
    let out = lines_of(&mut s, "n");
    assert!(contains(&out, "49"), "{out:?}");
    let out = lines_of(&mut s, ":type n");
    assert!(!out.is_empty(), "type line expected");
    let out = lines_of(&mut s, ":reset");
    assert!(contains(&out, "reset"), "{out:?}");
    let out = lines_of(&mut s, "7 * 6");
    assert!(contains(&out, "42"), "{out:?}");
}

#[test]
fn test_needs_more() {
    assert!(cli::repl::needs_more("fn f(): Int {"));
    assert!(cli::repl::needs_more("1 +"));
    assert!(!cli::repl::needs_more("1 + 2"));
    assert!(!cli::repl::needs_more("fn f(): Int { return 1; }"));
}
