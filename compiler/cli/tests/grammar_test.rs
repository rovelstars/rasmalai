use std::path::{Path, PathBuf};

fn repo_root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
}

fn grammar_path() -> PathBuf {
    repo_root()
        .join("editors")
        .join("vscode")
        .join("syntaxes")
        .join("rnx.tmLanguage.json")
}

fn config_path() -> PathBuf {
    repo_root()
        .join("editors")
        .join("vscode")
        .join("language-configuration.json")
}

fn read_json(path: &Path) -> serde_json::Value {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    serde_json::from_str(&text).unwrap_or_else(|e| panic!("parse {}: {e}", path.display()))
}

fn collect_patterns(value: &serde_json::Value, out: &mut Vec<String>) {
    match value {
        serde_json::Value::Object(map) => {
            for (key, val) in map {
                if key == "match" || key == "begin" || key == "end" {
                    if let serde_json::Value::String(s) = val {
                        out.push(s.clone());
                    }
                } else {
                    collect_patterns(val, out);
                }
            }
        }
        serde_json::Value::Array(items) => {
            for item in items {
                collect_patterns(item, out);
            }
        }
        _ => {}
    }
}

fn grammar_patterns() -> Vec<String> {
    let grammar = read_json(&grammar_path());
    let mut out = Vec::new();
    collect_patterns(&grammar, &mut out);
    out
}

fn compile_all(patterns: &[String]) {
    for pattern in patterns {
        fancy_regex::Regex::new(pattern)
            .unwrap_or_else(|e| panic!("pattern fails to compile {pattern:?}: {e}"));
    }
}

fn assert_covered(patterns: &[String], probe: &str) {
    let hit = patterns.iter().any(|pattern| {
        fancy_regex::Regex::new(pattern)
            .map(|re| re.is_match(probe).unwrap_or(false))
            .unwrap_or(false)
    });
    assert!(hit, "no grammar pattern matches probe {probe:?}");
}

fn collect_rnx(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            collect_rnx(&path, out);
        } else if path.extension().is_some_and(|ext| ext == "rnx") {
            out.push(path);
        }
    }
}

fn rnx_corpus() -> Vec<PathBuf> {
    let root = repo_root();
    let mut named = Vec::new();
    let nbody = root.join("programs").join("nbody").join("nbody.rnx");
    if nbody.is_file() {
        named.push(nbody);
    }
    let mut starfall = Vec::new();
    collect_rnx(&root, &mut starfall);
    starfall.retain(|p| {
        p.to_string_lossy().to_lowercase().contains("starfall")
    });
    named.extend(starfall);
    if named.is_empty() {
        let mut all = Vec::new();
        collect_rnx(&root, &mut all);
        named = all;
    }
    named.sort();
    named
}

#[test]
fn test_textmate_grammar_validity() {
    let grammar = read_json(&grammar_path());
    assert_eq!(grammar["name"], serde_json::Value::String("Rasmalai".to_string()));
    assert_eq!(
        grammar["scopeName"],
        serde_json::Value::String("source.rnx".to_string())
    );
    let file_types = grammar["fileTypes"]
        .as_array()
        .unwrap_or_else(|| panic!("grammar missing fileTypes array"));
    assert!(file_types.contains(&serde_json::Value::String("rnx".to_string())));
    assert!(grammar["patterns"].as_array().is_some());
    let repository = grammar["repository"]
        .as_object()
        .unwrap_or_else(|| panic!("grammar missing repository object"));
    for section in ["comments", "strings", "constants", "keywords", "types", "functions"] {
        assert!(repository.contains_key(section), "repository missing {section}");
    }

    let mut patterns = Vec::new();
    collect_patterns(&grammar, &mut patterns);
    assert!(!patterns.is_empty());
    compile_all(&patterns);

    let config = read_json(&config_path());
    assert_eq!(config["comments"]["lineComment"], serde_json::json!("//"));
    assert_eq!(config["comments"]["blockComment"], serde_json::json!(["/*", "*/"]));
    assert!(config["brackets"].as_array().is_some());
    assert!(config["autoClosingPairs"].as_array().is_some());
    assert!(config["surroundingPairs"].as_array().is_some());
    let increase = config["indentationRules"]["increaseIndentPattern"]
        .as_str()
        .unwrap_or_else(|| panic!("config missing increaseIndentPattern"));
    let decrease = config["indentationRules"]["decreaseIndentPattern"]
        .as_str()
        .unwrap_or_else(|| panic!("config missing decreaseIndentPattern"));
    compile_all(&[increase.to_string(), decrease.to_string()]);
}

#[test]
fn test_grammar_coverage_of_language_features() {
    let sources = rnx_corpus();
    assert!(!sources.is_empty(), "no .rnx sources found under {}", repo_root().display());
    let mut corpus = String::new();
    for source in &sources {
        corpus.push_str(&std::fs::read_to_string(source).unwrap_or_default());
        corpus.push('\n');
    }
    assert!(corpus.contains("Vec4f"), "corpus lacks Vec4f coverage");
    assert!(corpus.contains("Vec4i"), "corpus lacks Vec4i coverage");
    assert!(corpus.contains("fn ") || corpus.contains("fn("), "corpus lacks fn coverage");

    let patterns = grammar_patterns();
    let keywords = [
        "pub", "public", "fn", "let", "const", "class", "struct", "record", "trait", "enum",
        "with", "if", "else", "while", "for", "in", "switch", "case", "default", "break",
        "continue", "return", "async", "await", "defer", "throw", "throws", "try", "catch",
        "import", "export", "from", "test", "bench", "Vec4f", "Vec4i", "Int", "Float",
        "Bool", "Void", "String", "Array", "Map", "Set", "Option", "Result", "GenRef",
        "Mutex", "RwLock", "Condvar", "Barrier", "Channel", "ThreadPool", "init",
        "deinit", "onReload", "this", "true", "false", "null", "nil", "none",
    ];
    for keyword in keywords {
        assert_covered(&patterns, keyword);
    }
    let probes = [
        "0x1F", "0b101", "0o17", "1.5", "42", "// note", "/// docs", "//! module",
        "\"text\"", "${name}", "$name", "fn name", "name(", "CustomType",
        "/** docs */", "@param", "{Int}", "=>", "x =>", "(x: Int): Int =>",
    ];
    for probe in probes {
        assert_covered(&patterns, probe);
    }
}
