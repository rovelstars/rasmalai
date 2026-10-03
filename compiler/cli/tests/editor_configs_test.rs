use std::path::PathBuf;

fn editors_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("editors")
}

fn read_file(rel: &str) -> String {
    let path = editors_dir().join(rel);
    std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()))
}

#[test]
fn test_vscode_manifest_schema_and_links() {
    let text = read_file("vscode/package.json");
    let manifest: serde_json::Value =
        serde_json::from_str(&text).unwrap_or_else(|e| panic!("invalid package.json: {e}"));
    assert_eq!(manifest["name"], serde_json::Value::String("rasmalai".to_string()));
    let activation = manifest["activationEvents"]
        .as_array()
        .unwrap_or_else(|| panic!("missing activationEvents"));
    assert!(activation.contains(&serde_json::Value::String("onLanguage:rasmalai".to_string())));
    let languages = manifest["contributes"]["languages"]
        .as_array()
        .unwrap_or_else(|| panic!("missing contributes.languages"));
    assert!(languages.iter().any(|l| l["id"] == serde_json::Value::String("rasmalai".to_string())));
    let grammars = manifest["contributes"]["grammars"]
        .as_array()
        .unwrap_or_else(|| panic!("missing contributes.grammars"));
    assert!(grammars.iter().any(|g| g["scopeName"] == serde_json::Value::String("source.rnx".to_string())));
    for rel in ["vscode/language-configuration.json", "vscode/syntaxes/rnx.tmLanguage.json", "vscode/src/extension.ts"] {
        let path = editors_dir().join(rel);
        assert!(path.is_file(), "missing linked file {rel}");
        assert!(!std::fs::read_to_string(&path).unwrap().trim().is_empty(), "empty file {rel}");
    }
    let client = read_file("vscode/src/extension.ts");
    assert!(client.contains("\"lsp\""), "client must spawn `rnx lsp`");
    assert!(client.contains("rasmalai"), "client must target rasmalai");
}

#[test]
fn test_editor_configs_exist_and_reference_rnx_lsp() {
    let helix = read_file("helix/languages.toml");
    assert!(helix.contains("command = \"rnx\""), "{helix}");
    assert!(helix.contains("args = [\"lsp\"]"), "{helix}");
    assert!(helix.contains("rasmalai"), "{helix}");

    let neovim = read_file("neovim/rasmalai.lua");
    assert!(neovim.contains("rnx"), "{neovim}");
    assert!(neovim.contains("lsp"), "{neovim}");
    assert!(neovim.contains("filetype"), "{neovim}");

    let zed = read_file("zed/languages/rasmalai/config.toml");
    assert!(!zed.trim().is_empty());
    assert!(zed.contains("rnx"), "{zed}");

    let manifest = read_file("zed/extension.toml");
    assert!(manifest.contains("[grammars.rasmalai]"), "{manifest}");
    assert!(manifest.contains("rev = "), "{manifest}");
    assert!(manifest.contains("[language_servers.rnx-lsp]"), "{manifest}");

    let cargo = read_file("zed/Cargo.toml");
    assert!(cargo.contains("zed_extension_api"), "{cargo}");
    assert!(cargo.contains("cdylib"), "{cargo}");
    let lib = read_file("zed/src/lib.rs");
    assert!(lib.contains("language_server_command"), "{lib}");
    assert!(lib.contains("register_extension!"), "{lib}");
    assert!(lib.contains("\"lsp\""), "{lib}");
    for query in ["highlights.scm", "locals.scm", "folds.scm", "outline.scm"] {
        let path = editors_dir().join("zed/languages/rasmalai").join(query);
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        assert!(!text.trim().is_empty(), "empty {query}");
        let (mut depth, mut in_string) = (0i64, false);
        for ch in text.chars() {
            match ch {
                '"' => in_string = !in_string,
                '(' if !in_string => depth += 1,
                ')' if !in_string => depth -= 1,
                _ => {}
            }
        }
        assert_eq!(depth, 0, "unbalanced {query}");
    }
}
