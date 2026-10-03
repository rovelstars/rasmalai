use frontend::desugar::desugar;
use frontend::parser::Parser;

fn entry_diags(src: &str) -> Vec<diagnostics::Diagnostic> {
    let mut m = Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"));
    desugar(&mut m)
}

#[test]
fn conflicting_main_and_Main_rejected() {
    let diags = entry_diags("fn main(): Int {\n    return 1;\n}\nfn Main(): Int {\n    return 2;\n}\n");
    assert!(
        diags.iter().any(|d| d.code == diagnostics::Code::E108
            && d.message.contains("Multiple conflicting entry functions")),
        "{diags:?}"
    );
}

#[test]
fn single_lowercase_main_accepted() {
    let diags = entry_diags("fn main(): Int {\n    return 1;\n}\n");
    assert!(diags.is_empty(), "{diags:?}");
}

#[test]
fn single_uppercase_Main_accepted() {
    let diags = entry_diags("fn Main(): Int {\n    return 1;\n}\n");
    assert!(diags.is_empty(), "{diags:?}");
}
