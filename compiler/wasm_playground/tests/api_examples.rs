fn api_path() -> std::path::PathBuf {
    std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../website/static/data/api.json")
}

fn examples() -> Vec<(String, String)> {
    let text = match std::fs::read_to_string(api_path()) {
        Ok(t) => t,
        Err(_) => return Vec::new(),
    };
    let doc: serde_json::Value = serde_json::from_str(&text).expect("api.json parses");
    let mut out = Vec::new();
    let modules = doc.get("modules").and_then(|m| m.as_array()).cloned().unwrap_or_default();
    for m in &modules {
        let mname = m.get("name").and_then(|n| n.as_str()).unwrap_or("?");
        let no_arr = Vec::new();
        let mut items: Vec<(String, &serde_json::Value)> = Vec::new();
        for key in ["functions", "constants"] {
            let arr = m.get(key).and_then(|a| a.as_array()).unwrap_or(&no_arr);
            for it in arr {
                items.push((key.to_string(), it));
            }
        }
        let classes = m.get("classes").and_then(|a| a.as_array()).unwrap_or(&no_arr);
        for c in classes {
            let cname = c.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let methods = c.get("methods").and_then(|a| a.as_array()).unwrap_or(&no_arr);
            for it in methods {
                items.push((format!("{mname}.{cname}"), it));
            }
        }
        for (ctx, it) in &items {
            let name = it.get("name").and_then(|n| n.as_str()).unwrap_or("?");
            let empty = Vec::new();
            let tags = it
                .get("docs")
                .and_then(|d| d.get("tags"))
                .and_then(|t| t.as_array())
                .unwrap_or(&empty);
            for t in tags {
                if t.get("kind").and_then(|k| k.as_str()) == Some("example")
                    && let Some(code) = t.get("text").and_then(|t| t.as_str())
                {
                    out.push((format!("{ctx}/{name}"), code.to_string()));
                }
            }
        }
    }
    out
}

#[test]
fn api_examples_run_clean() {
    let cases = examples();
    if cases.is_empty() {
        eprintln!("api.json absent, skipping doc snippet run");
        return;
    }
    for (ctx, code) in &cases {
        if code.contains("exit(") {
            continue;
        }
        let o = wasm_playground::run(code);
        assert!(!o.contains('┌'), "{ctx} failed:\n{o}");
        assert!(o.contains("=>"), "{ctx} produced no value:\n{o}");
    }
}
