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
            !module.is_empty() && !module.starts_with('/') && !module.ends_with('/'),
            "malformed entry `{module}` in stdlib::MODULES"
        );
    }
    assert!(
        stdlib::MODULES.contains(&"prelude"),
        "MODULES must list the ambient `prelude`"
    );
}
