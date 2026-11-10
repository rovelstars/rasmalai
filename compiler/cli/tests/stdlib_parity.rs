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
            let body = text
                .strip_prefix("---\n")
                .and_then(|rest| rest.split_once("\n---\n").map(|(_, body)| body))
                .unwrap_or(&text);
            assert!(
                body.trim_start().starts_with("# "),
                "{module} docs/{guide} must start with a Markdown title"
            );
            assert!(
                text.contains("```rnx"),
                "{module} docs/{guide} must include a runnable rnx block"
            );
        }
    }
}

#[test]
fn every_stdlib_guide_carries_frontmatter() {
    fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
        let Ok(entries) = std::fs::read_dir(dir) else {
            return;
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                walk(&path, out);
            } else if path.extension().is_some_and(|e| e == "md") {
                out.push(path);
            }
        }
    }

    let root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src");
    for module in stdlib::MODULES {
        if module.contains('/') {
            continue;
        }
        let docs = root.join(module).join("docs");
        let mut files = Vec::new();
        walk(&docs, &mut files);
        for path in files {
            let text = std::fs::read_to_string(&path).unwrap();
            let rest = text
                .strip_prefix("---\n")
                .unwrap_or_else(|| panic!("{} must start with frontmatter", path.display()));
            let (front, body) = rest
                .split_once("\n---\n")
                .unwrap_or_else(|| panic!("{} must close its frontmatter", path.display()));
            let field = |key: &str| {
                front
                    .lines()
                    .find_map(|line| line.strip_prefix(&format!("{key}:")))
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
            };
            assert!(field("title").is_some(), "{} needs a title", path.display());
            assert!(field("description").is_some(), "{} needs a description", path.display());
            assert!(
                body.trim_start().starts_with("# "),
                "{} needs a Markdown heading after frontmatter",
                path.display()
            );
            let rel = path.strip_prefix(&docs).unwrap().to_string_lossy();
            if rel.starts_with("guides/") {
                assert!(field("section").is_some(), "{} needs a section", path.display());
                assert!(field("order").is_some(), "{} needs an order", path.display());
            }
            assert!(
                !text.contains("\u{2014}") && !text.contains("\u{2013}"),
                "{} uses a dash instead of punctuation",
                path.display()
            );
        }
    }
}
