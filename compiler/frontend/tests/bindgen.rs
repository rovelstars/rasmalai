use frontend::bindgen::{verify_annotations, BindgenOptions, ClassTally};
use std::path::PathBuf;

fn sqlite_opts() -> Option<BindgenOptions> {
    let header = PathBuf::from("/usr/include/sqlite3.h");
    if !header.is_file() {
        return None;
    }
    Some(BindgenOptions {
        headers: vec![header],
        include_dirs: vec![PathBuf::from("/usr/include")],
        native_lib: "sqlite3".to_string(),
        pkg_name: "sqlite3".to_string(),
        target: "x86_64-unknown-linux-gnu".to_string(),
        prefixes: Vec::new(),
        skips: Vec::new(),
        upstream_name: None,
    })
}

fn zlib_opts() -> Option<BindgenOptions> {
    let header = PathBuf::from("/usr/include/zlib.h");
    if !header.is_file() {
        return None;
    }
    Some(BindgenOptions {
        headers: vec![header],
        include_dirs: vec![PathBuf::from("/usr/include")],
        native_lib: "z".to_string(),
        pkg_name: "zlib".to_string(),
        target: "x86_64-unknown-linux-gnu".to_string(),
        prefixes: Vec::new(),
        skips: Vec::new(),
        upstream_name: None,
    })
}

fn tally_of(opts: &BindgenOptions) -> (ClassTally, frontend::bindgen::BindgenOutput) {
    let out = frontend::bindgen::generate(opts).unwrap_or_else(|e| panic!("{e}"));
    eprintln!(
        "tally={:?} emitted={} skipped={}",
        out.tally,
        out.emitted,
        out.skipped.len()
    );
    (out.tally.clone(), out)
}

#[test]
fn sqlite3_tally_matches_reference_classifier() {
    let Some(opts) = sqlite_opts() else {
        return;
    };
    let (tally, _) = tally_of(&opts);
    assert_eq!(
        tally,
        ClassTally {
            proxyable: 18,
            marshalable: 33,
            handletable: 138,
            not_proxyable: 109,
        },
        "{tally:?}"
    );
}

#[test]
fn zlib_tally_matches_reference_classifier() {
    let Some(opts) = zlib_opts() else {
        return;
    };
    let (tally, _) = tally_of(&opts);
    assert_eq!(
        tally,
        ClassTally {
            proxyable: 7,
            marshalable: 75,
            handletable: 0,
            not_proxyable: 6,
        },
        "{tally:?}"
    );
}

#[test]
fn sqlite3_key_symbols_emitted_or_warned() {
    let Some(opts) = sqlite_opts() else {
        return;
    };
    let (_, out) = tally_of(&opts);
    assert!(
        out.rnx.contains("unsafe fn sqlite3_open("),
        "handle-creating out-param must emit"
    );
    assert!(out.rnx.contains("unsafe fn sqlite3_libversion("));
    assert!(!out.rnx.contains("unsafe fn sqlite3_exec("), "callback surface warns");
    assert!(!out.rnx.contains("unsafe fn sqlite3_malloc("), "voidptr surface warns");
    assert!(!out.rnx.contains("unsafe fn sqlite3_mprintf("), "variadic surface warns");
    let reasons: std::collections::BTreeMap<&str, &str> =
        out.skipped.iter().map(|(n, r)| (n.as_str(), r.as_str())).collect();
    assert_eq!(reasons.get("sqlite3_exec"), Some(&"callback"));
    assert_eq!(reasons.get("sqlite3_mprintf"), Some(&"variadic"));
}

#[test]
fn zlib_key_symbols_emitted_or_warned() {
    let Some(opts) = zlib_opts() else {
        return;
    };
    let (_, out) = tally_of(&opts);
    assert!(out.rnx.contains("unsafe fn compressBound("));
    assert!(out.rnx.contains("unsafe fn deflate("));
    assert!(!out.rnx.contains("unsafe fn inflateBack("), "callback surface warns");
    assert!(!out.rnx.contains("unsafe fn gzprintf("), "variadic surface warns");
}

#[test]
fn generated_bindings_parse_and_verify() {
    for opts in [sqlite_opts(), zlib_opts()].into_iter().flatten() {
        let out = frontend::bindgen::generate(&opts).unwrap_or_else(|e| panic!("{e}"));
        frontend::parser::Parser::parse_module(&out.rnx)
            .unwrap_or_else(|e| panic!("generated bindings do not parse: {e}"));
        verify_annotations(&out.annotations).unwrap_or_else(|e| panic!("{e}"));
        assert!(
            out.rnx.lines().next().unwrap_or("").starts_with("// @generated rnx-bindgen "),
            "missing generator header"
        );
    }
}
