fn lower_src(src: &str) -> Result<lir::instr::Module, diagnostics::Diagnostic> {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lir::lower::lower(&m)
}

fn reject(src: &str, from: &str, to: &str) {
    let e = lower_src(src).expect_err("scalar `as` must be rejected");
    assert_eq!(e.code.as_str(), "E108", "{e:?}");
    assert!(e.span.is_some(), "diagnostic must carry a span: {e:?}");
    assert!(
        e.message.contains(&format!("does not convert `{from}` to `{to}`")),
        "message must name both types: {}",
        e.message
    );
    assert!(
        e.message.contains("`Int(x)` or `Float(x)`"),
        "message must point at the conversions: {}",
        e.message
    );
}

#[test]
fn float_to_int_as_is_rejected() {
    reject("fn Main(): Int { let i = 2.5 as Int return i }", "Float", "Int");
}

#[test]
fn int_to_float_as_is_rejected() {
    reject("fn Main(): Int { let f = 1 as Float return 0 }", "Int", "Float");
}

#[test]
fn bool_to_int_as_is_rejected() {
    reject("fn Main(): Int { let i = true as Int return i }", "Bool", "Int");
}

#[test]
fn same_kind_scalar_as_still_lowers() {
    let m = lower_src(
        "fn A(x: Int): Int { return x as Int }
         fn B(b: Byte): Int { return b as Int }
         fn C(x: Float): Float { return x as Float }
         fn D(x: Any): Int { return x as Int }",
    )
    .unwrap_or_else(|e| panic!("{e:?}"));
    let v = lir::verify::verify(&m);
    assert!(v.is_empty(), "{v:?}");
}

#[test]
fn byte_value_still_lowers_through_as() {
    let m = lower_src("fn Main(): Int { let b = 65 as Byte print(\"b={b}\") return 0 }")
        .unwrap_or_else(|e| panic!("{e:?}"));
    let v = lir::verify::verify(&m);
    assert!(v.is_empty(), "{v:?}");
}
