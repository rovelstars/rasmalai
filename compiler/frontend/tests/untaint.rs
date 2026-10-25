use diagnostics;
use frontend::parser::Parser;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn error_codes(src: &str) -> Vec<String> {
    frontend::unsafe_taint::check_module(&parse(src))
        .iter()
        .map(|d| d.code.as_str().to_string())
        .collect()
}

fn check_source_split(src: &str) -> (Vec<String>, Vec<String>) {
    match frontend::check::check_source(src) {
        Ok(warnings) => (
            Vec::new(),
            warnings.iter().map(|d| d.code.as_str().to_string()).collect(),
        ),
        Err(errors) => (
            errors.iter().map(|d| d.code.as_str().to_string()).collect(),
            Vec::new(),
        ),
    }
}

const FFI_IMPORT: &str =
    "import { fn compressBound(sourceLen: Int): Int } from native \"z\"\n";

#[test]
fn direct_ffi_call_outside_unsafe_is_e201() {
    let src = format!("{FFI_IMPORT}fn Main(): Int {{\n    return compressBound(100);\n}}\n");
    let codes = error_codes(&src);
    assert!(codes.iter().any(|c| c == "E201"), "{codes:?}");
    let diags = frontend::unsafe_taint::check_module(&parse(&src));
    let e201 = diags.iter().find(|d| d.code == diagnostics::Code::E201).unwrap();
    assert!(e201.hint.as_deref().unwrap_or("").contains("unsafe"), "{e201:?}");
}

#[test]
fn contained_wrapper_passes_with_justified_boundary() {
    let src = format!(
        "{FFI_IMPORT}fn bound(n: Int): Int {{\n    unsafe {{\n        return compressBound(n);\n    }}\n}}\nfn Main(): Int {{\n    return bound(100);\n}}\n"
    );
    let codes = error_codes(&src);
    assert!(codes.is_empty(), "{codes:?}");
    let map = frontend::unsafe_taint::boundary_map(&parse(&src));
    assert_eq!(map.len(), 2, "{map:?}");
    let bound = map.iter().find(|b| b.name == "bound").unwrap();
    assert_eq!(bound.direct_seeds, 1);
    assert!(bound.contained);
    let main = map.iter().find(|b| b.name == "Main").unwrap();
    assert_eq!(main.direct_seeds, 0);
    assert!(main.contained);
    assert!(map.windows(2).all(|w| w[0].name <= w[1].name));
}

#[test]
fn ffi_inside_unsafe_fn_body_is_contained() {
    let src = format!(
        "{FFI_IMPORT}unsafe fn bound(n: Int): Int {{\n    return compressBound(n);\n}}\nfn Main(): Int {{\n    unsafe {{\n        return bound(1);\n    }}\n}}\n"
    );
    let codes = error_codes(&src);
    assert!(codes.is_empty(), "{codes:?}");
}

#[test]
fn empty_unsafe_block_lints_w202() {
    let src = "fn Main(): Int {\n    unsafe {\n        return 1;\n    }\n}\n";
    let codes = error_codes(src);
    assert!(codes.iter().any(|c| c == "W202"), "{codes:?}");
}

#[test]
fn unsafe_block_with_pure_call_lints_but_tainted_call_justifies() {
    let pure = "fn helper(): Int {\n    return 1;\n}\nfn Main(): Int {\n    unsafe {\n        return helper();\n    }\n}\n";
    let codes = error_codes(pure);
    assert!(codes.iter().any(|c| c == "W202"), "{codes:?}");
    let src = format!(
        "{FFI_IMPORT}fn helper(): Int {{\n    unsafe {{\n        return compressBound(1);\n    }}\n}}\nfn Main(): Int {{\n    unsafe {{\n        return helper();\n    }}\n}}\n"
    );
    let codes = error_codes(&src);
    assert!(!codes.iter().any(|c| c == "W202"), "{codes:?}");
}

#[test]
fn method_call_on_unknown_receiver_suppresses_lint() {
    let src = "fn Main(): Int {\n    let p = 0;\n    unsafe {\n        p.write(99);\n    }\n    return 0;\n}\n";
    let codes = error_codes(src);
    assert!(!codes.iter().any(|c| c == "W202"), "{codes:?}");
}

#[test]
fn check_source_surfaces_both_diagnostics() {
    let src = format!(
        "{FFI_IMPORT}fn Main(): Int {{\n    let b = compressBound(10);\n    unsafe {{\n        return b;\n    }}\n}}\n"
    );
    let (errors, _) = check_source_split(&src);
    assert!(errors.iter().any(|c| c == "E201"), "{errors:?}");
    let (errors, warnings) = check_source_split("fn Main(): Int {\n    unsafe {\n        return 1;\n    }\n}\n");
    assert!(errors.is_empty(), "{errors:?}");
    assert!(warnings.iter().any(|c| c == "W202"), "{warnings:?}");
}
