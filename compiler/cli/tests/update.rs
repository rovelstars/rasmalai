#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use frontend::checksum::{DigestEntry, Sha256, manifest_digest_hex};
use std::collections::BTreeMap;
use std::io::{Read, Write};
use std::sync::{Arc, Mutex};

fn rnx() -> String {
    env!("CARGO_BIN_EXE_rnx").to_string()
}

struct VersionFiles {
    version: String,
    manifest: String,
    pairs: Vec<(String, Vec<u8>)>,
    digest: String,
}

fn version_files(full: &str, version: &str, marker: u32) -> VersionFiles {
    let config = format!(
        "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"0.0.0-published\"\n    }}\n}}\n"
    );
    let main = format!("export fn hello(): Int {{ return {marker}; }}\n").into_bytes();
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
    let manifest = format!("{{\"entries\": [{}]}}", rendered.join(", "));
    VersionFiles {
        version: version.to_string(),
        manifest,
        pairs,
        digest,
    }
}

fn node_json(full: &str, files: &VersionFiles) -> String {
    format!(
        "{{\"full\": \"{full}\", \"version\": \"{v}\", \"path\": \"{full}@{v}/chunks\", \"integrity\": \"{d}\", \"engineRange\": \"\", \"deps\": [], \"yanked\": false}}",
        v = files.version,
        d = files.digest
    )
}

fn parse_triple(version: &str) -> Option<(u64, u64, u64)> {
    let mut parts = version.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch))
}

fn satisfies(version: &str, range: &str) -> bool {
    let v = match parse_triple(version) {
        Some(v) => v,
        None => return false,
    };
    let r = range.trim();
    if r == "*" || r == "latest" {
        return true;
    }
    if let Some(rest) = r.strip_prefix('^') {
        let b = match parse_triple(rest.trim()) {
            Some(b) => b,
            None => return false,
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
        let b = match parse_triple(rest.trim()) {
            Some(b) => b,
            None => return false,
        };
        return v >= b && v.0 == b.0 && v.1 == b.1;
    }
    if let Some(rest) = r.strip_prefix(">=") {
        return match parse_triple(rest.trim()) {
            Some(b) => v >= b,
            None => false,
        };
    }
    parse_triple(r).is_some_and(|b| v == b)
}

fn serve(packages: BTreeMap<String, Vec<VersionFiles>>) -> String {
    let packages = Arc::new(packages);
    let seen: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let packages = packages.clone();
            let seen = seen.clone();
            std::thread::spawn(move || {
                let _ = stream.set_read_timeout(Some(std::time::Duration::from_secs(10)));
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
                seen.lock().unwrap().push(format!("{method} {path}"));
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
                                                parse_triple(&cand.version)
                                                    > parse_triple(&b.version)
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
    url
}

fn clear_env(cmd: &mut std::process::Command) {
    cmd.env_remove("HTTP_PROXY");
    cmd.env_remove("HTTPS_PROXY");
    cmd.env_remove("ALL_PROXY");
    cmd.env_remove("http_proxy");
    cmd.env_remove("https_proxy");
    cmd.env_remove("all_proxy");
    cmd.env_remove("RNX_PREFER_NATIVE");
}

fn proj(
    tag: &str,
    deps: &str,
    extra: &str,
    registry: &str,
) -> (
    crate::scratch::Scratch,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let dir = crate::scratch::dir(&format!("cli-update-{tag}"));
    let root = dir.join("app");
    std::fs::create_dir_all(root.join("src")).unwrap();
    write_manifest(&root, deps, extra, registry);
    std::fs::write(
        root.join("src").join("main.rnx"),
        "fn Main(): Int { return 0; }\n",
    )
    .unwrap();
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    (dir, root, cache)
}

fn write_manifest(root: &std::path::Path, deps: &str, extra: &str, registry: &str) {
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n{deps}    }}{extra},\n    registry: {{ url: \"{registry}\" }}\n}}\n"
        ),
    )
    .unwrap();
}

fn run(root: &std::path::Path, cache: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(rnx());
    cmd.args(args)
        .current_dir(root)
        .env("RNX_CACHE_HOME", cache);
    clear_env(&mut cmd);
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
fn update_all_reports_old_to_new() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "@acme/widget".to_string(),
        vec![
            version_files("@acme/widget", "1.2.0", 120),
            version_files("@acme/widget", "1.3.0", 130),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj(
        "all",
        "        \"@acme/widget\": { version: \"1.2.0\" }\n",
        "",
        &url,
    );
    let out = run(&root, &cache, &["lock"]);
    assert!(out.status.success(), "{}", combined(&out));
    write_manifest(
        &root,
        "        \"@acme/widget\": { version: \"^1.0.0\" }\n",
        "",
        &url,
    );
    let out = run(&root, &cache, &["update"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("updated @acme/widget 1.2.0 -> 1.3.0"),
        "{}",
        combined(&out)
    );
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("version: \"1.3.0\""), "{text}");
    assert!(
        text.contains("source: \"registry:@acme/widget@1.3.0\""),
        "{text}"
    );
}

#[test]
fn update_named_only_moves_named() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "@acme/widget".to_string(),
        vec![
            version_files("@acme/widget", "1.2.0", 120),
            version_files("@acme/widget", "1.3.0", 130),
        ],
    );
    packages.insert(
        "@acme/gadget".to_string(),
        vec![version_files("@acme/gadget", "2.0.0", 200)],
    );
    let url = serve(packages);
    let deps = "        \"@acme/widget\": { version: \"1.2.0\" },\n        \"@acme/gadget\": { version: \"2.0.0\" }\n";
    let (_dir, root, cache) = proj("named", deps, "", &url);
    let out = run(&root, &cache, &["lock"]);
    assert!(out.status.success(), "{}", combined(&out));
    let deps = "        \"@acme/widget\": { version: \"^1.0.0\" },\n        \"@acme/gadget\": { version: \"^2.0.0\" }\n";
    write_manifest(&root, deps, "", &url);
    let out = run(&root, &cache, &["update", "@acme/widget"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("updated @acme/widget 1.2.0 -> 1.3.0"),
        "{}",
        combined(&out)
    );
    assert!(
        !combined(&out).contains("@acme/gadget"),
        "{}",
        combined(&out)
    );
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("version: \"2.0.0\""), "{text}");
    assert!(
        text.contains("source: \"registry:@acme/gadget@2.0.0\""),
        "{text}"
    );
}

#[test]
fn update_latest_selector_pins_digest() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "@acme/widget".to_string(),
        vec![
            version_files("@acme/widget", "1.2.0", 120),
            version_files("@acme/widget", "1.3.0", 130),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj(
        "latest",
        "        \"@acme/widget\": { version: \"1.2.0\" }\n",
        "",
        &url,
    );
    let out = run(&root, &cache, &["lock"]);
    assert!(out.status.success(), "{}", combined(&out));
    let out = run(&root, &cache, &["update", "@acme/widget@latest"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("updated @acme/widget 1.2.0 -> 1.3.0"),
        "{}",
        combined(&out)
    );
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(!text.contains("@latest"), "{text}");
    assert!(
        text.contains("source: \"registry:@acme/widget@1.3.0\""),
        "{text}"
    );
}

#[test]
fn update_noop_reports_up_to_date() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "@acme/widget".to_string(),
        vec![version_files("@acme/widget", "1.2.0", 120)],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj(
        "noop",
        "        \"@acme/widget\": { version: \"1.2.0\" }\n",
        "",
        &url,
    );
    let out = run(&root, &cache, &["lock"]);
    assert!(out.status.success(), "{}", combined(&out));
    let before = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    let out = run(&root, &cache, &["update"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Already up to date."),
        "{}",
        combined(&out)
    );
    let after = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert_eq!(before, after);
}

#[test]
fn update_keeps_consumer_native_override() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "@acme/widget".to_string(),
        vec![
            version_files("@acme/widget", "1.2.0", 120),
            version_files("@acme/widget", "1.3.0", 130),
        ],
    );
    let url = serve(packages);
    let deps = "        \"@acme/widget\": { version: \"1.2.0\" },\n        zlib: { native: \"z\", system: true }\n";
    let extra = ",\n    native: { zlib: { system: false } }";
    let (_dir, root, cache) = proj("native", deps, extra, &url);
    let out = run(&root, &cache, &["lock"]);
    assert!(out.status.success(), "{}", combined(&out));
    let deps = "        \"@acme/widget\": { version: \"^1.0.0\" },\n        zlib: { native: \"z\", system: true }\n";
    write_manifest(&root, deps, extra, &url);
    let out = run(&root, &cache, &["update"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("updated @acme/widget 1.2.0 -> 1.3.0"),
        "{}",
        combined(&out)
    );
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("source: \"native:z\""), "{text}");
}
