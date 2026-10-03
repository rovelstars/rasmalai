#[test]
fn all_stdlib_modules_registered_and_resolvable() {
    assert!(!stdlib::MODULES.is_empty(), "MODULES must not be empty");
    let mut seen = std::collections::HashSet::new();
    for module in stdlib::MODULES {
        assert!(
            seen.insert(*module),
            "duplicate entry `{module}` in stdlib::MODULES"
        );
        assert!(
            stdlib::source(module).is_some(),
            "standard module `@std/{module}` is registered in MODULES but missing implementation"
        );
    }
}
