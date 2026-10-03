use std::path::{Path, PathBuf};

fn grammar_dir() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..")
        .join("editors")
        .join("tree-sitter-rasmalai")
}

fn read_file(dir: &Path, name: &str) -> String {
    let path = dir.join(name);
    let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
    assert!(!text.trim().is_empty(), "{} is empty", path.display());
    text
}

fn check_balanced(text: &str, origin: &str) {
    let mut depth = 0i64;
    let mut chars = text.chars().peekable();
    while let Some(ch) = chars.next() {
        match ch {
            ';' => {
                for tail in chars.by_ref() {
                    if tail == '\n' {
                        break;
                    }
                }
            }
            '"' => {
                let mut closed = false;
                while let Some(inner) = chars.next() {
                    if inner == '\\' {
                        chars.next();
                    } else if inner == '"' {
                        closed = true;
                        break;
                    } else if inner == '\n' {
                        break;
                    }
                }
                assert!(closed, "{origin}: unmatched quote");
            }
            '(' => depth += 1,
            ')' => {
                depth -= 1;
                assert!(depth >= 0, "{origin}: unmatched closing parenthesis");
            }
            '@' => {
                let mut name = String::new();
                while let Some(&next) = chars.peek() {
                    if next.is_alphanumeric() || next == '_' || next == '.' || next == '-' {
                        name.push(next);
                        chars.next();
                    } else {
                        break;
                    }
                }
                assert!(!name.is_empty(), "{origin}: unclosed capture after @");
            }
            _ => {}
        }
    }
    assert_eq!(depth, 0, "{origin}: unbalanced parentheses");
}

#[test]
fn test_tree_sitter_files_exist_and_non_empty() {
    let dir = grammar_dir();
    for name in [
        "package.json",
        "grammar.js",
        "queries/highlights.scm",
        "queries/locals.scm",
        "queries/folds.scm",
    ] {
        read_file(&dir, name);
    }
}

#[test]
fn test_query_files_balanced_parentheses() {
    let dir = grammar_dir();
    let queries = dir.join("queries");
    let mut seen = 0;
    let mut entries: Vec<_> = std::fs::read_dir(&queries)
        .unwrap_or_else(|e| panic!("read {}: {e}", queries.display()))
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "scm"))
        .collect();
    entries.sort();
    for path in entries {
        let text = std::fs::read_to_string(&path).unwrap_or_else(|e| panic!("read {}: {e}", path.display()));
        check_balanced(&text, &path.display().to_string());
        seen += 1;
    }
    assert!(seen >= 3, "expected at least 3 query files, found {seen}");
}

#[test]
fn test_grammar_keyword_consistency() {
    let dir = grammar_dir();
    let grammar = read_file(&dir, "grammar.js");
    assert!(grammar.contains("name: 'rasmalai'"));
    let keywords = [
        "pub", "public", "fn", "let", "const", "class", "struct", "record", "trait", "enum",
        "with", "if", "else", "while", "for", "in", "switch", "case", "default", "break",
        "continue", "return", "async", "await", "defer", "throw", "throws", "try", "catch",
        "import", "test", "bench", "Vec4f", "Vec4i", "Int", "Float", "Bool", "Void", "String",
        "init", "deinit", "onReload", "this", "true", "false",
    ];
    for keyword in keywords {
        let quoted = format!("'{keyword}'");
        assert!(grammar.contains(&quoted), "grammar.js missing keyword {keyword}");
    }
}
