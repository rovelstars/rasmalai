use frontend::desugar::desugar;
use frontend::parser::Parser;
use frontend::semantic::check;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn errors(src: &str) -> Vec<diagnostics::Diagnostic> {
    let mut m = parse(src);
    let _ = desugar(&mut m);
    check(&m).into_iter().filter(|d| !d.code.is_warning()).collect()
}

fn codes(src: &str) -> Vec<String> {
    errors(src).iter().map(|d| d.code.as_str().to_string()).collect()
}

const PRELUDE: &str = "enum Color {\n\
    Red,\n\
    Green,\n\
    Blue\n\
    }\n\
    fn paint(c: Color): Int {\n\
    switch c {\n\
    case .Red: return 1;\n\
    case .Green: return 2;\n\
    case .Blue: return 3;\n\
    }\n\
    }\n";

#[test]
fn implicit_free_fn_arg_is_clean() {
    let got = codes(&format!("{PRELUDE}fn Main(): Int {{\n    return paint(.Green);\n    }}\n"));
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn implicit_method_static_init_args_are_clean() {
    let src = "enum Color {\n\
        Red,\n\
        Green,\n\
        Blue\n\
        }\n\
        fn shade(c: Color): Int {\n\
        switch c {\n\
        case .Red: return 1;\n\
        case .Green: return 2;\n\
        case .Blue: return 3;\n\
        }\n\
        }\n\
        class Canvas {\n\
        let mode: Int = 0;\n\
        fn setMode(c: Color): Int {\n\
        return shade(c);\n\
        }\n\
        static fn withColor(c: Color): Int {\n\
        return shade(c);\n\
        }\n\
        init(c: Color) {\n\
        this.mode = shade(c);\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        let canvas = new Canvas(.Green);\n\
        canvas.setMode(.Blue);\n\
        Canvas.withColor(.Red);\n\
        return 0;\n\
        }\n";
    let got = codes(src);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn implicit_payload_variant_arg_is_clean() {
    let src = "enum Outcome {\n\
        Ok(Int),\n\
        Err(String)\n\
        }\n\
        fn show(r: Outcome): Int {\n\
        switch r {\n\
        case .Ok(n): return n;\n\
        case .Err(e): return 0 - 1;\n\
        }\n\
        }\n\
        fn Main(): Int {\n\
        show(.Ok(1));\n\
        show(.Err(\"x\"));\n\
        return 0;\n\
        }\n";
    let got = codes(src);
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn implicit_let_annotation_still_clean() {
    let got = codes(&format!("{PRELUDE}fn Main(): Int {{\n    let a: Color = .Red;\n    return paint(a);\n    }}\n"));
    assert!(got.is_empty(), "{got:?}");
}

#[test]
fn unknown_implicit_variant_in_arg_is_e108() {
    let mut m = parse(&format!("{PRELUDE}fn Main(): Int {{\n    return paint(.NonExistent);\n    }}\n"));
    let _ = desugar(&mut m);
    let got = check(&m);
    let hit = got.iter().find(|d| d.code.as_str() == "E108" && d.message.contains("unknown variant"));
    assert!(hit.is_some(), "{got:?}");
    assert!(hit.unwrap().span.is_some(), "{got:?}");
}

#[test]
fn implicit_without_context_stays_semantically_clean() {
    let got = codes("enum Color {\n    Red,\n    Green\n    }\nfn Main(): Int {\n    let x = .Green;\n    return 0;\n    }\n");
    assert!(got.is_empty(), "{got:?}");
}
