use std::path::PathBuf;

const LIB_SRC: &str = "/** Adds two numbers together. */\npub fn add(a: Int, b: Int): Int { return a + b; }\n\n/** Represents a 2D coordinate. */\npub class Point {\n    let x: Float;\n    let y: Float;\n    init(x: Float, y: Float) { this.x = x; this.y = y; }\n}\n\nfn privateHelper(): Void {}\n";

fn manifest(name: &str, _entry: &str) -> String {
    format!(
        "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\",\n        description: \"Docs for {name}.\"\n    }}\n}}\n"
    )
}

fn write_project(tag: &str, name: &str, src: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("rnx-doc-{tag}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("src")).unwrap();
    std::fs::write(dir.join("Project.config"), manifest(name, "src/main.rnx")).unwrap();
    std::fs::write(dir.join("src").join("main.rnx"), src).unwrap();
    dir
}

fn run_doc(dir: &std::path::Path, extra: &[&str]) -> std::process::Output {
    let rnx = env!("CARGO_BIN_EXE_rnx");
    let mut cmd = std::process::Command::new(rnx);
    cmd.arg("doc").current_dir(dir);
    for a in extra {
        cmd.arg(a);
    }
    cmd.output().unwrap()
}

fn sha256(path: &std::path::Path) -> String {
    let out = std::process::Command::new("sha256sum").arg(path).output().unwrap();
    assert!(out.status.success(), "sha256sum failed");
    String::from_utf8(out.stdout).unwrap().split_whitespace().next().unwrap().to_string()
}

#[test]
fn test_doc_generation_structure() {
    let dir = write_project("struct", "calc", LIB_SRC);
    let out = run_doc(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc = dir.join("target").join("doc");
    assert!(doc.join("index.html").is_file(), "index missing");
    assert!(doc.join("style.css").is_file(), "css missing");
    let index = std::fs::read_to_string(doc.join("index.html")).unwrap();
    assert!(index.contains("calc"), "package:\n{index}");
    let pages: Vec<PathBuf> = std::fs::read_dir(&doc)
        .unwrap()
        .map(|e| e.unwrap().path())
        .filter(|p| p.extension().is_some_and(|e| e == "html") && p.file_name().unwrap() != "index.html")
        .collect();
    assert!(!pages.is_empty(), "no module pages");
    let mut found_add = false;
    let mut found_point = false;
    for page in &pages {
        let text = std::fs::read_to_string(page).unwrap();
        if text.contains("fn add(a: Int, b: Int): Int") && text.contains("Adds two numbers together.") {
            found_add = true;
        }
        if text.contains("class Point") && text.contains("Represents a 2D coordinate.") {
            found_point = true;
        }
    }
    assert!(found_add, "add undocumented");
    assert!(found_point, "Point undocumented");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_private_items_excluded() {
    let dir = write_project("priv", "calc", LIB_SRC);
    let out = run_doc(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc = dir.join("target").join("doc");
    let mut all = String::new();
    for entry in std::fs::read_dir(&doc).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "html") {
            all.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    assert!(!all.contains("privateHelper"), "private item leaked");
    let _ = std::fs::remove_dir_all(&dir);
}

fn doc_tree_hashes(root: &std::path::Path) -> Vec<(String, String)> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap() {
            let path = entry.unwrap().path();
            if path.is_dir() {
                stack.push(path);
            } else {
                let rel = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                out.push((rel, sha256(&path)));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn test_doc_output_determinism() {
    let base = std::env::temp_dir().join(format!("rnx-docdet-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&base);
    for tag in ["a", "b"] {
        let dir = base.join(tag);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Project.config"), manifest("calc", "src/main.rnx")).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), LIB_SRC).unwrap();
    }
    for tag in ["a", "b"] {
        let out = run_doc(&base.join(tag), &[]);
        assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    }
    let ha = doc_tree_hashes(&base.join("a").join("target").join("doc"));
    let hb = doc_tree_hashes(&base.join("b").join("target").join("doc"));
    assert_eq!(ha, hb, "doc trees differ");
    assert!(!ha.is_empty(), "empty doc tree");
    let _ = std::fs::remove_dir_all(&base);
}
#[test]
fn test_workspace_doc_generation() {

    let dir = std::env::temp_dir().join(format!("rnx-docws-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).unwrap();
    std::fs::write(
        dir.join("Project.config"),
        "export default {\n    workspace: {\n        members: [\"alpha\", \"beta\"]\n    }\n}\n",
    )
    .unwrap();
    for (member, src) in [
        ("alpha", "/** Alpha helpers. */\npub fn ahalf(x: Int): Int { return x; }\n"),
        ("beta", "/** Beta helpers. */\npub fn bhalf(x: Int): Int { return x; }\n"),
    ] {
        let mdir = dir.join(member);
        std::fs::create_dir_all(mdir.join("src")).unwrap();
        std::fs::write(mdir.join("Project.config"), manifest(member, "src/main.rnx")).unwrap();
        std::fs::write(mdir.join("src").join("main.rnx"), src).unwrap();
    }
    let out = run_doc(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let index = std::fs::read_to_string(dir.join("target").join("doc").join("index.html")).unwrap();
    assert!(index.contains("alpha"), "alpha missing:\n{index}");
    assert!(index.contains("beta"), "beta missing:\n{index}");
    assert!(dir.join("target").join("doc").join("alpha").join("index.html").is_file());
    assert!(dir.join("target").join("doc").join("beta").join("index.html").is_file());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_jsdoc_tags_render() {
    let src = "/**\n * Quote for one unit of ore.\n *\n * @param base price per unit before pressure.\n * @param demand buyer pressure count.\n * @returns pressure-scaled quote.\n * @example\n * quote(2.0, 3, 1)\n */\npub fn quote(base: Float, demand: Int): Float { return base; }\n";
    let dir = write_project("tags", "calc", src);
    let out = run_doc(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc = dir.join("target").join("doc");
    let mut all = String::new();
    for entry in std::fs::read_dir(&doc).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "html") && path.file_name().unwrap() != "index.html" {
            all.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    assert!(all.contains("Quote for one unit of ore."), "desc missing");
    assert!(all.contains("<dt><code>base</code></dt><dd>price per unit before pressure.</dd>"), "param missing:\n{all}");
    assert!(all.contains("<dt><code>demand</code></dt><dd>buyer pressure count.</dd>"), "param missing");
    assert!(all.contains("<dt>Returns</dt><dd>pressure-scaled quote.</dd>"), "returns missing");
    assert!(all.contains("<pre><code>quote(2.0, 3, 1)</code></pre>"), "example missing");
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn test_doc_all_includes_private_items() {
    let src = "/** Internal helper. */\nfn internal(x: Int): Int { return x; }\n\n/** Private helper. */\nprivate fn hidden(): Void {}\n";
    let dir = write_project("all", "calc", src);
    let out = run_doc(&dir, &[]);
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    let doc = dir.join("target").join("doc");
    let mut plain = String::new();
    for entry in std::fs::read_dir(&doc).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "html") && path.file_name().unwrap() != "index.html" {
            plain.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    assert!(!plain.contains("Internal helper."), "private leaked into default docs");
    let out_all = run_doc(&dir, &["--all"]);
    assert!(out_all.status.success(), "{}", String::from_utf8_lossy(&out_all.stderr));
    let mut all = String::new();
    for entry in std::fs::read_dir(&doc).unwrap() {
        let path = entry.unwrap().path();
        if path.extension().is_some_and(|e| e == "html") && path.file_name().unwrap() != "index.html" {
            all.push_str(&std::fs::read_to_string(&path).unwrap());
        }
    }
    assert!(all.contains("Internal helper."), "plain fn missing from --all docs:\n{all}");
    assert!(all.contains("Private helper."), "private fn missing from --all docs");
    let _ = std::fs::remove_dir_all(&dir);
}
