fn api_path() -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!(
        "rnx-api-examples-{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_nanos())
            .unwrap_or(0)
    ));
    let _ = std::fs::remove_dir_all(&dir);
    docgen::generate_stdlib_docs_json_local(&dir)
        .unwrap_or_else(|e| panic!("cannot generate the stdlib catalog: {e}"))
}

fn host_skip(module: &str) -> Option<String> {
    let top = module.split('/').next().unwrap_or(module);
    let spec = format!("@std/{top}");
    let active = sysroot::target::active_target();
    if active.prelude != spec
        && sysroot::target::Target::all()
            .iter()
            .any(|t| t.prelude == spec)
    {
        return Some(format!(
            "`{spec}` is the prelude for another target, not `{}`",
            active.triple
        ));
    }
    match frontend::modules::load_std_module_source(module) {
        Ok(_) => None,
        Err(e) if e.message.contains("requires capability") => Some(e.message),
        Err(e) => panic!("@std/{module} source did not load: {}", e.message),
    }
}

fn examples() -> Vec<(String, String)> {
    let path = api_path();
    let text = std::fs::read_to_string(&path)
        .unwrap_or_else(|e| panic!("cannot read {}: {e}", path.display()));
    if let Some(parent) = path.parent() {
        let _ = std::fs::remove_dir_all(parent);
    }
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
        if items.is_empty() {
            continue;
        }
        if let Some(reason) = host_skip(mname) {
            eprintln!("skipping @std/{mname} examples: {reason}");
            continue;
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
    assert!(
        !cases.is_empty(),
        "the stdlib catalog carries no documentation examples"
    );
    for (ctx, code) in &cases {
        if code.contains("exit(") {
            continue;
        }
        let o = wasm_playground::run(code);
        assert!(!o.contains('┌'), "{ctx} failed:\n{o}");
        assert!(o.contains("=>"), "{ctx} produced no value:\n{o}");
    }
}
