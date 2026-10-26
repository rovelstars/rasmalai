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

#[test]
fn every_module_resolves_to_its_checkout_source() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src");
    for module in stdlib::MODULES {
        let rel = if *module == "net/http" {
            "net/http.rnx".to_string()
        } else {
            format!("{module}/lib.rnx")
        };
        let path = root.join(&rel);
        assert!(path.is_file(), "`@std/{module}` must resolve to {rel} in the checkout");
        let src = std::fs::read_to_string(&path).unwrap();
        assert!(
            !src.is_empty() && src.starts_with("//!"),
            "`@std/{module}` source must carry a `//!` module header"
        );
    }
}

#[test]
fn module_folders_carry_publishable_guides() {
    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src");
    for module in stdlib::MODULES {
        if module.contains('/') {
            continue;
        }
        let dir = root.join(module);
        assert!(dir.join("lib.rnx").is_file(), "{module} must ship lib.rnx");
        for guide in ["overview.md", "getting-started.md"] {
            let path = dir.join("docs").join(guide);
            let text = std::fs::read_to_string(&path)
                .unwrap_or_else(|_| panic!("{module} must ship docs/{guide}"));
            assert!(
                text.starts_with("# "),
                "{module} docs/{guide} must start with a Markdown title"
            );
            assert!(
                text.contains("```rnx"),
                "{module} docs/{guide} must include a runnable rnx block"
            );
        }
    }
}
