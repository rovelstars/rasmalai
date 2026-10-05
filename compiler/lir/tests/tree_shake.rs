use lir::{lower, reach, verify};

fn compile(src: &str) -> lir::instr::Module {
    let mut m = frontend::parser::Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    lower::lower(&m).unwrap_or_else(|e| panic!("{e}"))
}

fn shaken(src: &str) -> lir::instr::Module {
    let mut m = compile(src);
    reach::shake_module(&mut m, "Main", false);
    let v = verify::verify(&m);
    assert!(v.is_empty(), "{v:?}");
    m
}

#[test]
fn unreachable_fn_dropped_from_output() {
    let m = shaken("fn Main(): Int { return 0 } fn Dead(): Int { return 1 }");
    let names: Vec<&str> = m.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Main"), "{names:?}");
    assert!(!names.contains(&"Dead"), "{names:?}");
    assert!(m.fn_index.get("Dead").is_none());
}

#[test]
fn reachable_through_interface_kept() {
    let m = shaken(
        "interface HasX {\n\
        fn get(): Int;\n\
        }\n\
        class Base with HasX {\n\
        let x: Int;\n\
        init(x: Int) { this.x = x; }\n\
        fn get(): Int { return this.x * 2; }\n\
        }\n\
        fn useIt(h: HasX): Int {\n\
        return h.get();\n\
        }\n\
        fn Main(): Int {\n\
        let b = new Base(21);\n\
        return useIt(b);\n\
        }\n",
    );
    let names: Vec<&str> = m.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Base.get"), "{names:?}");
    assert!(names.contains(&"useIt"), "{names:?}");
    assert!(names.contains(&"Main"), "{names:?}");
}

#[test]
fn pub_export_kept_in_app_mode() {
    let m = shaken("pub fn helper(): Int { return 1 } fn Main(): Int { return 0 }");
    let names: Vec<&str> = m.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"helper"), "{names:?}");
    assert!(names.contains(&"Main"), "{names:?}");
}

#[test]
fn value_call_keeps_only_address_taken_callees() {
    let m = shaken(
        "fn Dead(): Int { return 1 }\n\
        fn Main(): Int {\n\
        let f = (x: Int): Int => x + 1;\n\
        return f(41);\n\
        }\n",
    );
    let names: Vec<&str> = m.functions.iter().map(|f| f.name.as_str()).collect();
    assert!(names.contains(&"Main"), "{names:?}");
    assert!(!names.contains(&"Dead"), "{names:?}");
    assert!(names.iter().any(|n| n.contains("closure")), "{names:?}");
    assert!(m.fn_index.get("Dead").is_none());
}

#[test]
fn unreachable_fn_still_typechecked() {
    let mut m =
        frontend::parser::Parser::parse_module("fn Main(): Int { return 0 } fn Dead(): Int { let x = 1 }")
            .unwrap_or_else(|e| panic!("{e}"));
    let d = frontend::desugar::desugar(&mut m);
    assert!(d.is_empty(), "{d:?}");
    let e = lower::lower(&m).expect_err("unreachable body must still be checked");
    assert_eq!(e.code, diagnostics::Code::E108);
}
