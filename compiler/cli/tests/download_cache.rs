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

struct VersionFiles {
    version: String,
    manifest: String,
    pairs: Vec<(String, Vec<u8>)>,
}

fn version_files(full: &str, version: &str) -> VersionFiles {
    version_files_with_main(full, version, b"export fn hello(): Int { return 1; }\n")
}

fn version_files_with_main(full: &str, version: &str, main: &[u8]) -> VersionFiles {
    let config = format!(
        "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"0.0.0-published\"\n    }}\n}}\n"
    );
    let main = main.to_vec();
    let files = vec![
        ("Project.config".to_string(), config.into_bytes()),
        ("src/main.rnx".to_string(), main),
    ];
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
    let digest = manifest_digest_hex(&entries).unwrap();
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

fn node_json(full: &str, files: &VersionFiles) -> String {
    format!(
        "{{\"full\": \"{full}\", \"version\": \"{v}\", \"path\": \"{full}@{v}/chunks\", \"integrity\": \"{d}\", \"engineRange\": \"\", \"deps\": [], \"yanked\": false}}",
        v = files.version,
        d = manifest_digest(&files.manifest),
    )
}

fn manifest_digest(manifest: &str) -> String {
    let value: serde_json::Value = serde_json::from_str(manifest).unwrap();
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

fn serve(
    packages: BTreeMap<String, Vec<VersionFiles>>,
) -> (
    String,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
    std::sync::Arc<std::sync::atomic::AtomicUsize>,
) {
    let packages = std::sync::Arc::new(packages);
    let resolve_posts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let prelude_posts = std::sync::Arc::new(std::sync::atomic::AtomicUsize::new(0));
    let counted = resolve_posts.clone();
    let counted_prelude = prelude_posts.clone();
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let packages = packages.clone();
            let counted = counted.clone();
            let counted_prelude = counted_prelude.clone();
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
                    counted.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    if body.windows(12).any(|w| w == b"@std/prelude") {
                        counted_prelude.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    }
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
                                            nodes.push(node_json(full, pick));
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
    (url, resolve_posts, prelude_posts)
}

fn proj(tag: &str, registry: &str) -> (crate::scratch::Scratch, std::path::PathBuf, std::path::PathBuf) {
    let dir = crate::scratch::dir(&format!("cli-dl-{tag}"));
    let root = dir.join("app");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n        widget: {{ version: \"1.3.0\" }}\n    }},\n    registry: {{ url: \"{registry}\" }}\n}}\n",
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("src").join("main.rnx"),
        "import { hello } from \"widget\";\nfn Main(): Int {\n    return hello();\n}\n",
    )
    .unwrap();
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    (dir, root, cache)
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

#[test]
fn download_range_into_global_cache_then_local() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![version_files("widget", "1.2.0"), version_files("widget", "1.3.0")],
    );
    let cli_version = env!("CARGO_PKG_VERSION").to_string();
    let prelude_src = std::fs::read(
        std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("../stdlib/src/prelude/lib.rnx"),
    )
    .expect("checkout prelude must exist");
    packages.insert(
        "@std/prelude".to_string(),
        vec![version_files_with_main("@std/prelude", &cli_version, &prelude_src)],
    );
    let (url, _resolve_posts, prelude_posts) = serve(packages);
    let (_dir, root, cache) = proj("basic", &url);
    let out = run(&root, &cache, &url, &["download", "widget@^1.0.0"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Downloaded widget@1.3.0"),
        "{}",
        combined(&out)
    );
    let out = run(&root, &cache, &url, &["lock"]);
    assert!(out.status.success(), "{}", combined(&out));
    let out = run(&root, &cache, &url, &["download", "--local", "widget@1.3.0"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Downloaded widget@1.3.0"),
        "{}",
        combined(&out)
    );
    assert!(root.join(".rnx-cache").join("registry").is_dir(), "project cache seeded");
    let out = run(&root, &cache, &url, &["download", "--local", &format!("@std/prelude@{cli_version}")]);
    assert!(out.status.success(), "{}", combined(&out));
    let prelude_before = prelude_posts.load(std::sync::atomic::Ordering::SeqCst);
    assert_eq!(prelude_before, 1, "only the explicit prelude download resolves @std/prelude");
    let cold = _dir.join("cold");
    std::fs::create_dir_all(&cold).unwrap();
    let out = run(&root, &cold, &url, &["check"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert_eq!(
        prelude_posts.load(std::sync::atomic::Ordering::SeqCst),
        prelude_before,
        "cold global cache serves @std/prelude from the project cache with no new resolve\n{}",
        combined(&out)
    );
}
