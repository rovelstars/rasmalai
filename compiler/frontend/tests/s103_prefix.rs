use diagnostics::Code;
use frontend::parser::Parser;
use frontend::security::s103_frame;
use std::collections::BTreeSet;
use std::path::Path;

fn parse(src: &str) -> frontend::ast::Module {
    Parser::parse_module(src).unwrap_or_else(|e| panic!("{e}"))
}

fn s103(diags: &[diagnostics::Diagnostic]) -> Vec<&diagnostics::Diagnostic> {
    diags.iter().filter(|d| d.code == Code::S103).collect()
}

fn scan(src: &str) -> Vec<diagnostics::Diagnostic> {
    let m = parse(src);
    s103_frame(&[(Path::new(""), &m)])
}

#[test]
fn bare_rnx_call_is_s103() {
    let diags = scan("fn Main(): Int {\n    return __rnx_clock_mono();\n}\n");
    let hits = s103(&diags);
    assert_eq!(hits.len(), 1, "{diags:?}");
    assert!(hits[0].message.contains("__rnx_clock_mono"), "{}", hits[0].message);
    assert!(
        hits[0].hint.as_deref().unwrap_or_default().contains("@std"),
        "{:?}",
        hits[0].hint
    );
    assert!(hits[0].file.is_none(), "{hits:?}");
}

#[test]
fn bare_rnx_ident_use_is_s103() {
    let diags = scan("fn Main(): Int {\n    let f = __rnx_clock_mono;\n    return f();\n}\n");
    assert_eq!(s103(&diags).len(), 1, "{diags:?}");
}

#[test]
fn declared_native_alias_is_not_bare() {
    let diags = scan("import { fn clock(): Int as __rnx_clock_mono } from native \"rt\"\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n");
    assert!(s103(&diags).is_empty(), "{diags:?}");
}

#[test]
fn locally_declared_rnx_name_is_not_bare() {
    let diags = scan("fn __rnx_helper(): Int {\n    return 1;\n}\nfn Main(): Int {\n    return __rnx_helper();\n}\n");
    assert!(s103(&diags).is_empty(), "{diags:?}");
}

#[test]
fn bare_ffi_name_is_a_scope_error_not_s103() {
    let m = parse("fn Main(): Int {\n    return __ffi_clock_mono();\n}\n");
    let diags = frontend::semantic::check(&m);
    assert!(s103(&diags).is_empty(), "{diags:?}");
    assert!(diags.iter().any(|d| d.code == Code::E303), "{diags:?}");
    let framed = s103_frame(&[(Path::new(""), &m)]);
    assert!(s103(&framed).is_empty(), "{framed:?}");
}

#[test]
fn frame_declared_names_cover_cross_file_uses() {
    let src = "fn Main(): Int {\n    return __rnx_clock_mono();\n}\n";
    let m = parse(src);
    let mut extra = BTreeSet::new();
    extra.insert("__rnx_clock_mono".to_string());
    let diags =
        frontend::semantic::check_with_files_and_globals(&m, &diagnostics::DeclFiles::new(), &extra);
    assert!(s103(&diags).is_empty(), "{diags:?}");
    let bare =
        frontend::semantic::check_with_files_and_globals(&m, &diagnostics::DeclFiles::new(), &BTreeSet::new());
    assert_eq!(s103(&bare).len(), 1, "{bare:?}");
}

#[test]
fn driver_tags_real_paths_and_leaves_snippets_untagged() {
    let m = parse("fn Main(): Int {\n    return __rnx_clock_mono();\n}\n");
    let tagged = s103_frame(&[(Path::new("src/main.rnx"), &m)]);
    assert_eq!(tagged.len(), 1, "{tagged:?}");
    assert_eq!(
        tagged[0].file.as_deref(),
        Some(Path::new("src/main.rnx")),
        "{tagged:?}"
    );
    let untagged = s103_frame(&[(Path::new(""), &m)]);
    assert_eq!(untagged.len(), 1, "{untagged:?}");
    assert!(untagged[0].file.is_none(), "{untagged:?}");
}

#[test]
fn std_package_paths_are_exempt_by_prefix() {
    use std::collections::BTreeMap;
    let mut cache = BTreeMap::new();
    assert!(frontend::security::file_in_std_package(
        std::path::Path::new("@std/time/src/lib.rnx"),
        &mut cache
    ));
    assert!(!frontend::security::file_in_std_package(
        std::path::Path::new("src/main.rnx"),
        &mut cache
    ));
}
