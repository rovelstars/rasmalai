#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use frontend::checksum::{manifest_digest_hex, DigestEntry, Sha256};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::process::Stdio;

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

fn cli_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

fn std_tops() -> Vec<String> {
    let mut tops: Vec<String> = stdlib::MODULES
        .iter()
        .map(|m| m.split('/').next().unwrap_or(m).to_string())
        .collect();
    tops.sort();
    tops.dedup();
    tops
}

fn checkout_std_file(top: &str, file: &str) -> Vec<u8> {
    let path = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src").join(top).join(file);
    std::fs::read(&path).unwrap_or_else(|_| panic!("checkout stdlib source must exist: {}", path.display()))
}

fn package_files(full: &str, version: &str, top: &str) -> Vec<(String, Vec<u8>)> {
    let config = format!(
        "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"0.0.0-published\",\n        engine: \">={version}\"\n    }},\n    entries: {{\n        main: \"src/lib.rnx\"\n    }}\n}}\n"
    );
    let mut files = vec![
        ("Project.config".to_string(), config.into_bytes()),
        ("src/lib.rnx".to_string(), checkout_std_file(top, "lib.rnx")),
    ];
    if top == "net" {
        files.push(("src/http.rnx".to_string(), checkout_std_file(top, "http.rnx")));
    }
    files
}

struct VersionFiles {
    version: String,
    manifest: String,
    pairs: Vec<(String, Vec<u8>)>,
}

fn version_files(full: &str, version: &str, top: &str) -> VersionFiles {
    let files = package_files(full, version, top);
    let mut entries = Vec::new();
    let mut pairs = Vec::new();
    for (name, bytes) in &files {
        let hash = Sha256::hexdigest(bytes);
        entries.push(DigestEntry {
            name: name.clone(),
            size: bytes.len() as u64,
            dir: false,
            chunks: vec![hash.clone()],
        });
        pairs.push((hash, bytes.clone()));
    }
    let rendered: Vec<String> = entries
        .iter()
        .map(|e| {
            format!(
                "{{\"name\": \"{}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{}\"]}}",
                e.name, e.size, e.chunks[0]
            )
        })
        .collect();
    VersionFiles {
        version: version.to_string(),
        manifest: format!("{{\"entries\": [{}]}}", rendered.join(", ")),
        pairs,
    }
}

fn node_json(full: &str, files: &VersionFiles, engine: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(&files.manifest).unwrap();
    let digest = {
        let items = value.get("entries").and_then(|v| v.as_array()).unwrap();
        let entries: Vec<DigestEntry> = items
            .iter()
            .map(|item| DigestEntry {
                name: item.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                size: item.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                dir: false,
                chunks: item
                    .get("chunks")
                    .and_then(|v| v.as_array())
                    .map(|a| a.iter().map(|h| h.as_str().unwrap_or_default().to_string()).collect())
                    .unwrap_or_default(),
            })
            .collect();
        manifest_digest_hex(&entries).unwrap()
    };
    format!(
        "{{\"full\": \"{full}\", \"version\": \"{v}\", \"path\": \"{full}@{v}/chunks\", \"integrity\": \"{d}\", \"engineRange\": \"{engine}\", \"deps\": [], \"yanked\": false}}",
        v = files.version,
        d = digest,
    )
}

fn split_pre(version: &str) -> Option<((u64, u64, u64), &str)> {
    let (core, pre) = match version.split_once('-') {
        Some((c, p)) => (c, p),
        None => (version, ""),
    };
    let mut parts = core.split('.');
    let triple = (
        parts.next()?.parse::<u64>().ok()?,
        parts.next()?.parse::<u64>().ok()?,
        parts.next()?.parse::<u64>().ok()?,
    );
    if parts.next().is_some() {
        return None;
    }
    Some((triple, pre))
}

fn satisfies(version: &str, range: &str) -> bool {
    let Some((v, vpre)) = split_pre(version) else {
        return false;
    };
    let r = range.trim();
    if r == "*" || r == "latest" {
        return vpre.is_empty();
    }
    if let Some(rest) = r.strip_prefix('^') {
        let Some((b, _)) = split_pre(rest.trim()) else {
            return false;
        };
        if v < b {
            return false;
        }
        if b.0 > 0 {
            return v.0 == b.0;
        }
        if b.1 > 0 {
            return v.0 == 0 && v.1 == b.1;
        }
        return v == b;
    }
    if let Some(rest) = r.strip_prefix('~') {
        let Some((b, _)) = split_pre(rest.trim()) else {
            return false;
        };
        return v >= b && v.0 == b.0 && v.1 == b.1;
    }
    if let Some(rest) = r.strip_prefix(">=") {
        let Some((b, _)) = split_pre(rest.trim()) else {
            return false;
        };
        return v >= b;
    }
    split_pre(r).is_some_and(|(b, _)| v == b)
}

fn serve(packages: BTreeMap<String, Vec<VersionFiles>>) -> String {
    let engine = format!(">={}", cli_version());
    let packages = std::sync::Arc::new(packages);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let packages = packages.clone();
            let engine = engine.clone();
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(30)));
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(0) => return,
                        Ok(_) => head.push(byte[0]),
                        Err(_) => return,
                    }
                    if head.len() > 65536 {
                        return;
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let mut lines = text.lines();
                let request_line = lines.next().unwrap_or_default().to_string();
                let mut parts = request_line.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let mut length = 0usize;
                for line in lines {
                    if let Some(rest) = line.to_ascii_lowercase().strip_prefix("content-length:") {
                        length = rest.trim().parse::<usize>().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; length.min(16 * 1024 * 1024)];
                let mut read = 0usize;
                while read < body.len() {
                    match stream.read(&mut body[read..]) {
                        Ok(0) => break,
                        Ok(n) => read += n,
                        Err(_) => break,
                    }
                }
                body.truncate(read);
                let route = path.split('?').next().unwrap_or_default().to_string();
                let mut respond = |status: u16, text: &str, body: &[u8]| {
                    let head = format!(
                        "HTTP/1.1 {status} {text}\r\nContent-Type: application/json\r\nrnx-registry-spec: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = stream.write_all(head.as_bytes());
                    let _ = stream.write_all(body);
                };
                if method == "GET" && route == "/api/version" {
                    respond(
                        200,
                        "OK",
                        br#"{"spec": 0, "capabilities": ["tombstones"], "registry": "test"}"#,
                    );
                } else if method == "POST" && route == "/api/v0/resolve" {
                    let value: serde_json::Value =
                        serde_json::from_slice(&body).unwrap_or(serde_json::Value::Null);
                    let mut resolved = BTreeMap::new();
                    let mut nodes = Vec::new();
                    let mut bad = None;
                    if let Some(reqs) = value.get("requirements").and_then(|v| v.as_object()) {
                        for (full, range) in reqs {
                            let range = range.as_str().unwrap_or_default();
                            match packages.get(full) {
                                None => {
                                    bad = Some(format!("package {full} does not exist"));
                                    break;
                                }
                                Some(versions) => {
                                    let mut best: Option<&VersionFiles> = None;
                                    for cand in versions {
                                        if !satisfies(&cand.version, range) {
                                            continue;
                                        }
                                        let take = match &best {
                                            None => true,
                                            Some(b) => {
                                                split_pre(&cand.version).map(|(t, _)| t)
                                                    > split_pre(&b.version).map(|(t, _)| t)
                                            }
                                        };
                                        if take {
                                            best = Some(cand);
                                        }
                                    }
                                    match best {
                                        None => {
                                            bad = Some(format!(
                                                "no version of {full} satisfies {range}"
                                            ));
                                            break;
                                        }
                                        Some(pick) => {
                                            resolved.insert(full.clone(), pick.version.clone());
                                            nodes.push(node_json(full, pick, &engine));
                                        }
                                    }
                                }
                            }
                        }
                    }
                    match bad {
                        Some(message) => {
                            let body =
                                format!("{{\"code\": \"not-found\", \"message\": \"{message}\"}}");
                            respond(404, "Not Found", body.as_bytes());
                        }
                        None => {
                            let body = format!(
                                "{{\"resolved\": {{{}}}, \"base\": \"\", \"levels\": [[{}]]}}",
                                resolved
                                    .iter()
                                    .map(|(k, v)| format!("\"{k}\": \"{v}\""))
                                    .collect::<Vec<_>>()
                                    .join(", "),
                                nodes.join(", ")
                            );
                            respond(200, "OK", body.as_bytes());
                        }
                    }
                } else if method == "GET" && route.ends_with("/chunks") {
                    let hit = packages
                        .iter()
                        .flat_map(|(full, versions)| {
                            versions
                                .iter()
                                .map(move |v| (format!("{full}@{}", v.version), v))
                        })
                        .find(|(key, _)| route.contains(key.as_str()));
                    match hit {
                        Some((_, files)) => respond(200, "OK", files.manifest.as_bytes()),
                        None => respond(404, "Not Found", b"{\"code\": \"not-found\"}"),
                    }
                } else if method == "GET" && route.contains("/chunk/") {
                    let hash = route.rsplit('/').next().unwrap_or_default().to_string();
                    let mut hit: Option<Vec<u8>> = None;
                    for versions in packages.values() {
                        for files in versions {
                            for (h, b) in &files.pairs {
                                if *h == hash {
                                    hit = Some(b.clone());
                                    break;
                                }
                            }
                        }
                    }
                    match hit {
                        Some(bytes) => {
                            let head = format!(
                                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nrnx-registry-spec: 0\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                                bytes.len()
                            );
                            let _ = stream.write_all(head.as_bytes());
                            let _ = stream.write_all(&bytes);
                        }
                        None => respond(404, "Not Found", b"{\"code\": \"not-found\"}"),
                    }
                } else {
                    respond(404, "Not Found", b"{\"code\": \"not-found\"}");
                }
            });
        }
    });
    url
}

fn run(
    root: &std::path::Path,
    cache: &std::path::Path,
    registry: &str,
    args: &[&str],
) -> std::process::Output {
    let mut cmd = std::process::Command::new(rnx());
    cmd.args(args)
        .current_dir(root)
        .env("RNX_CACHE_HOME", cache)
        .env("RNX_REGISTRY", registry)
        .env_remove("RNX_VENDOR_DIR")
        .stdin(Stdio::null());
    for key in [
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "ALL_PROXY",
        "http_proxy",
        "https_proxy",
        "all_proxy",
    ] {
        cmd.env_remove(key);
    }
    cmd.output().unwrap()
}

fn combined(out: &std::process::Output) -> String {
    format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    )
}

fn mock_std_registry() -> (String, Vec<String>) {
    let version = cli_version();
    let tops = std_tops();
    let mut packages = BTreeMap::new();
    for top in &tops {
        let full = format!("@std/{top}");
        packages.insert(full.clone(), vec![version_files(&full, &version, top)]);
    }
    (serve(packages), tops)
}

fn seed_fresh_cache(tag: &str) -> (scratch::Scratch, std::path::PathBuf, std::path::PathBuf, Vec<String>) {
    let (url, tops) = mock_std_registry();
    let dir = scratch::dir(&format!("cli-seed-{tag}"));
    let root = dir.join("app");
    std::fs::create_dir_all(&root).unwrap();
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    let out = run(&root, &cache, &url, &["download"]);
    assert!(out.status.success(), "bare download seeds the cache\n{}", combined(&out));
    (dir, root, cache, tops)
}

fn cached_bytes(cache: &std::path::Path) -> Vec<Vec<u8>> {
    let mut found = Vec::new();
    let mut stack = vec![cache.join("store")];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                if let Ok(bytes) = std::fs::read(&path) {
                    found.push(bytes);
                }
            }
        }
    }
    found
}

#[test]
fn fresh_install_seed_is_byte_identical() {
    let (_dir, _root, cache, tops) = seed_fresh_cache("identical");
    let pin_path = cache.join("std-pin.json");
    let pin = std::fs::read(&pin_path).unwrap_or_else(|_| panic!("seed writes {}", pin_path.display()));
    let pin_text = String::from_utf8_lossy(&pin).into_owned();
    for top in &tops {
        assert!(
            pin_text.contains(&format!("@std/{top}")),
            "pin names @std/{top}\n{pin_text}"
        );
    }
    let blobs = cached_bytes(&cache);
    assert!(!blobs.is_empty(), "seeded store holds package trees");
    for top in &tops {
        let checkout = checkout_std_file(top, "lib.rnx");
        assert!(
            blobs.iter().any(|b| *b == checkout),
            "seeded @std/{top} is byte-identical to the checkout source"
        );
    }
    let net_http = checkout_std_file("net", "http.rnx");
    assert!(
        blobs.iter().any(|b| *b == net_http),
        "seeded @std/net submodule is byte-identical to the checkout source"
    );
}

fn bmh_find(haystack: &[u8], needle: &[u8]) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }
    let mut skip = [0usize; 256];
    for s in skip.iter_mut() {
        *s = needle.len();
    }
    for (i, &b) in needle[..needle.len() - 1].iter().enumerate() {
        skip[b as usize] = needle.len() - 1 - i;
    }
    let mut i = 0usize;
    while i + needle.len() <= haystack.len() {
        let mut j = needle.len();
        while j > 0 && haystack[i + j - 1] == needle[j - 1] {
            j -= 1;
        }
        if j == 0 {
            return true;
        }
        i += skip[haystack[i + needle.len() - 1] as usize].max(1);
    }
    false
}

#[test]
fn shipped_binary_carries_no_stdlib_sources() {
    let binary = std::fs::read(rnx()).expect("test harness exposes the rnx binary");
    assert!(!binary.is_empty(), "rnx binary is non-empty");
    let std_root = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src");
    let mut sources = Vec::new();
    let mut stack = vec![std_root];
    while let Some(dir) = stack.pop() {
        for entry in std::fs::read_dir(&dir).unwrap().flatten() {
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().and_then(|e| e.to_str()) == Some("rnx") {
                sources.push(path);
            }
        }
    }
    assert!(!sources.is_empty(), "checkout carries stdlib sources to check against");
    sources.sort();
    for path in &sources {
        let text = std::fs::read_to_string(path).unwrap();
        let sentinel = text.lines().max_by_key(|l| l.len()).unwrap_or_default();
        let sentinel = sentinel.trim_end();
        assert!(
            sentinel.len() >= 40,
            "stdlib source has a distinctive line to scan for: {}",
            path.display()
        );
        assert!(
            !bmh_find(&binary, sentinel.as_bytes()),
            "rnx binary embeds stdlib source line from {}",
            path.display()
        );
    }
}

#[test]
fn seeded_cache_builds_offline_against_dead_registry() {
    let (_dir, root, cache, _tops) = seed_fresh_cache("offline");
    let dead = "http://127.0.0.1:9";
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: { name: \"probe\", version: \"0.1.0\" }\n}\n",
    )
    .unwrap();
    std::fs::write(
        root.join("src").join("main.rnx"),
        "import { Clock } from \"@std/time\";\n\nfn Main(): Int {\n    let clock = Clock.virtual(0);\n    clock.tick(2500000);\n    return clock.now().toMillis();\n}\n",
    )
    .unwrap();
    let out = run(&root, &cache, dead, &["check"]);
    assert!(out.status.success(), "seeded cache checks with no network\n{}", combined(&out));
    let out = run(&root, &cache, dead, &["build", "src/main.rnx"]);
    assert!(out.status.success(), "seeded cache builds with no network\n{}", combined(&out));
    let exe = root.join(".rnx-cache").join("build").join("dev").join("probe");
    assert!(exe.is_file(), "build writes .rnx-cache/build/dev/probe");
    let status = std::process::Command::new(&exe).status().unwrap();
    assert_eq!(status.code(), Some(2), "probe binary runs and returns the clock reading");
}
