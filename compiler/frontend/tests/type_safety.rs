use diagnostics::Code;
use frontend::check::check_source;

fn codes(src: &str) -> Vec<Code> {
    match check_source(src) {
        Ok(_) => Vec::new(),
        Err(errs) => errs.iter().map(|d| d.code).collect(),
    }
}

fn has(src: &str, code: Code) -> bool {
    codes(src).contains(&code)
}

#[test]
fn inferred_primitive_reassign_mismatches() {
    let src = "fn Main(): Int {\n    let a = 2;\n    a = \"hello\";\n    return 0;\n}\n";
    assert!(has(src, Code::E205), "{:?}", codes(src));
}

#[test]
fn annotated_primitive_reassign_mismatches() {
    let src = "fn Main(): Int {\n    let a: Int = 2;\n    a = \"hello\";\n    return 0;\n}\n";
    assert!(has(src, Code::E205), "{:?}", codes(src));
}

#[test]
fn inferred_class_reassign_mismatches() {
    let src = "class Meter {\n    let v: Int = 0;\n    init(v: Int) { this.v = v; }\n}\nfn Main(): Int {\n    let a = new Meter(10);\n    a = 42;\n    return 0;\n}\n";
    assert!(has(src, Code::E205), "{:?}", codes(src));
}

#[test]
fn explicit_any_reassign_passes() {
    let src = "fn Main(): Int {\n    let a: Any = 2;\n    a = \"hello\";\n    return 0;\n}\n";
    assert!(!has(src, Code::E205), "{:?}", codes(src));
    assert!(!has(src, Code::E206), "{:?}", codes(src));
}

#[test]
fn any_dynamic_boundary_flows() {
    let src = "fn Main(): Int {\n    let a: Any = \"hello\";\n    a = 42;\n    let b: Int = a + 10;\n    return b;\n}\n";
    assert!(!has(src, Code::E205), "{:?}", codes(src));
}

#[test]
fn unannotated_binding_without_initializer_defaults_to_null() {
    let src = "fn Main(): Int {\n    let a;\n    return 0;\n}\n";
    assert!(!has(src, Code::E206), "{:?}", codes(src));
}

#[test]
fn annotated_declaration_mismatch_fails() {
    let src = "fn Main(): Int {\n    let a: Int = \"hello\";\n    return 0;\n}\n";
    assert!(has(src, Code::E205), "{:?}", codes(src));
}

#[test]
fn same_type_reassign_passes() {
    let src = "fn Main(): Int {\n    let a = 2;\n    a = 3;\n    let s = \"x\";\n    s = \"y\";\n    return a;\n}\n";
    assert!(codes(src).is_empty(), "{:?}", codes(src));
}

#[test]
fn int_to_float_promotion_passes() {
    let src = "fn Main(): Int {\n    let f: Float = 1;\n    f = 2;\n    return 0;\n}\n";
    assert!(!has(src, Code::E205), "{:?}", codes(src));
}

#[test]
fn const_reassign_fails() {
    let src = "fn Main(): Int {\n    const a = 2;\n    a = 3;\n    return a;\n}\n";
    assert!(!codes(src).is_empty(), "const reassign must fail");
}
