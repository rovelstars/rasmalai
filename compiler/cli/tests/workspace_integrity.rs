fn crates_dir() -> std::path::PathBuf {
    let manifest = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest.join("..")
}

fn dep_lines(text: &str) -> Vec<(usize, String)> {
    let mut out = Vec::new();
    let mut in_deps = false;
    for (i, line) in text.lines().enumerate() {
        let t = line.trim();
        if t.starts_with('[') {
            in_deps = t == "[dependencies]" || t == "[dev-dependencies]";
            continue;
        }
        if in_deps && !t.is_empty() && !t.starts_with('#') && t.contains('=') {
            out.push((i + 1, t.to_string()));
        }
    }
    out
}

#[test]
fn test_zero_external_dependencies_in_core() {
    let root = crates_dir();
    // Allowed third-party crates per crate. Each entry needs an explicit
    // justification recorded here; everything else must stay path-only so
    // the core compiler has no supply-chain surface beyond its lockfile.
    // - runtime/mio: Phase-1 async reactor (epoll/kqueue/IOCP event loop
    //   backing @std/net non-blocking sockets).
    // - runtime/serde_json: @std/json document parser (RFC 8259 grammar,
    //   offset-tagged errors, no build scripts; pure Rust, wasm-safe).
    // - runtime/serde: staging adapter for the serde_json lexer used by the
    //   streaming @std/json parser (Visitor traits only; same author and
    //   lockfile entry as serde_json, no new supply-chain surface).
    // - frontend/sha2: plan-mandated SHA-256 content fingerprints for the
    //   build cache (pure Rust, no build scripts).
    let allowed: &[(&str, &str)] = &[("runtime", "mio"), ("runtime", "serde_json"), ("runtime", "serde"), ("frontend", "sha2")];
    for krate in ["frontend", "lir", "runtime", "diagnostics", "stdlib"] {
        let path = root.join(krate).join("Cargo.toml");
        assert!(path.is_file(), "missing {}", path.display());
        let text = std::fs::read_to_string(&path).unwrap();
        for (line_no, line) in dep_lines(&text) {
            let exempt = allowed
                .iter()
                .any(|(k, dep)| *k == krate && line.starts_with(dep));
            assert!(
                line.contains("path") || exempt,
                "{krate} Cargo.toml:{line_no} has external dep: {line}"
            );
        }
    }
}

fn all_codes() -> Vec<&'static str> {
    diagnostics::ALL.iter().map(|c| c.as_str()).collect()
}

fn rs_files(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    let read = std::fs::read_dir(dir).unwrap();
    for entry in read {
        let path = entry.unwrap().path();
        if path.is_dir() {
            let name = path.file_name().map(|s| s.to_string_lossy().into_owned()).unwrap_or_default();
            if name == "target" || name.starts_with('.') {
                continue;
            }
            rs_files(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}

#[test]
fn test_diagnostic_registry_coverage() {
    let codes = all_codes();
    for code in &codes {
        let parsed = diagnostics::Code::from_str(code);
        assert!(parsed.is_some(), "{code} missing from Code enum");
        let c = parsed.unwrap();
        assert_eq!(c.as_str(), *code, "{code} round-trip");
        assert!(!c.title().is_empty(), "{code} has no title");
        let fix = cli::explain(code);
        assert!(fix.is_some(), "{code} has no explain fix");
        assert!(!fix.unwrap().1.is_empty(), "{code} has empty fix");
        if code.starts_with('W') {
            assert!(c.is_warning(), "{code} should be a warning");
        } else {
            assert!(!c.is_warning(), "{code} should be an error");
        }
    }
    let root = crates_dir();
    let mut files = Vec::new();
    rs_files(&root, &mut files);
    assert!(!files.is_empty(), "no source files found");
    let mut bodies: Vec<String> = Vec::new();
    for f in &files {
        if let Ok(text) = std::fs::read_to_string(f) {
            bodies.push(text);
        }
    }
    for code in &codes {
        let hits = bodies.iter().filter(|b| b.contains(code)).count();
        assert!(hits > 0, "{code} never referenced in compiler sources or tests");
    }
}
