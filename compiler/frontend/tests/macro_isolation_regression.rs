use frontend::modules::ModuleGraph;

fn isolation(src: &str) -> Vec<diagnostics::Diagnostic> {
    ModuleGraph::from_source_graph(src)
        .unwrap_or_else(|e| panic!("{e}"))
        .isolation_errors()
}

#[test]
fn vec_macro_is_not_an_unresolved_identifier() {
    let diags = isolation("fn Main(): Int { let v = vec![1, 2, 3]; return v[0]; }");
    assert!(
        diags.iter().all(|d| d.code.is_warning()),
        "vec! is owned by the syntax expansion pass and must not fail import isolation: {diags:?}"
    );
}

#[test]
fn embed_bytes_macro_is_not_an_unresolved_identifier() {
    let diags = isolation("fn Main(): Int { let b = EmbedBytes!(\"blob.bin\"); return b.len(); }");
    assert!(
        diags.iter().all(|d| d.code.is_warning()),
        "EmbedBytes! is owned by the syntax expansion pass and must not fail import isolation: {diags:?}"
    );
}

#[test]
fn static_assert_macro_is_not_an_unresolved_identifier() {
    let diags = isolation(
        "fn Main(): Int { StaticAssert!(12 * 4 == 48, \"packed\"); return 0; }",
    );
    assert!(
        diags.iter().all(|d| d.code.is_warning()),
        "StaticAssert! is owned by the comptime expansion pass and must not fail import isolation: {diags:?}"
    );
}

#[test]
fn unknown_macro_still_fails_isolation() {
    let diags = isolation("fn Main(): Int { let v = BogusMacro![1]; return 0; }");
    assert!(
        diags.iter().any(|d| d.code.as_str() == "E303"),
        "names no pass owns must stay unresolved: {diags:?}"
    );
}

#[test]
fn unresolved_sibling_name_still_fails_isolation() {
    let diags = isolation("fn Main(): Int { return Widget(); }");
    assert!(
        diags.iter().any(|d| d.code.as_str() == "E303"),
        "a plain unresolved identifier must keep failing: {diags:?}"
    );
}
