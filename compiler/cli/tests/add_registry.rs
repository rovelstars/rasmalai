#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use frontend::checksum::{DigestEntry, Sha256, manifest_digest_hex};
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
    digest: String,
    engine: String,
}

fn version_files(full: &str, version: &str, marker: u32) -> VersionFiles {
    version_files_engine(full, version, marker, "")
}

fn version_files_engine(full: &str, version: &str, marker: u32, engine: &str) -> VersionFiles {
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
        engine: engine.to_string(),
    }
}

fn node_json(full: &str, files: &VersionFiles) -> String {
    format!(
        "{{\"full\": \"{full}\", \"version\": \"{v}\", \"path\": \"{full}@{v}/chunks\", \"integrity\": \"{d}\", \"engineRange\": \"{e}\", \"deps\": [], \"yanked\": false}}",
        v = files.version,
        d = files.digest,
        e = files.engine,
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
    if core.split('.').any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    Some((triple, pre))
}

fn range_base(range: &str) -> Option<((u64, u64, u64), String)> {
    let r = range.trim();
    let rest = r
        .strip_prefix('^')
        .or_else(|| r.strip_prefix('~'))
        .or_else(|| r.strip_prefix(">="))
        .unwrap_or(r);
    let ((a, b, c), pre) = split_pre(rest.trim())?;
    Some(((a, b, c), pre.to_string()))
}

fn satisfies(version: &str, range: &str) -> bool {
    let Some((v, vpre)) = split_pre(version) else {
        return false;
    };
    let r = range.trim();
    if r == "*" || r == "latest" {
        return vpre.is_empty();
    }
    let base = match range_base(r) {
        Some(base) => base,
        None => return false,
    };
    if vpre.is_empty() != base.1.is_empty() {
        if vpre.is_empty() {
            return false;
        }
        if base.1.is_empty() || v != base.0 {
            return false;
        }
    }
    if r.starts_with('^') {
        let b = base.0;
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
    if r.starts_with('~') {
        let b = base.0;
        return v >= b && v.0 == b.0 && v.1 == b.1;
    }
    if let Some(rest) = r.strip_prefix(">=") {
        let _ = rest;
        return v >= base.0;
    }
    v == base.0
}

fn serve(packages: BTreeMap<String, Vec<VersionFiles>>) -> String {
    let packages = std::sync::Arc::new(packages);
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    let url = format!("http://{}", listener.local_addr().unwrap());
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else {
                continue;
            };
            let packages = packages.clone();
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
    registry: &str,
) -> (
    crate::scratch::Scratch,
    std::path::PathBuf,
    std::path::PathBuf,
) {
    let dir = crate::scratch::dir(&format!("cli-add-{tag}"));
    let root = dir.join("app");
    std::fs::create_dir_all(root.join("src")).unwrap();
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n{deps}    }},\n    registry: {{ url: \"{registry}\" }}\n}}\n"
        ),
    )
    .unwrap();
    std::fs::write(
        root.join("src").join("main.rnx"),
        "fn Main(): Int { return 0; }\n",
    )
    .unwrap();
    let cache = dir.join("cache");
    std::fs::create_dir_all(&cache).unwrap();
    (dir, root, cache)
}

fn run(root: &std::path::Path, cache: &std::path::Path, args: &[&str]) -> std::process::Output {
    let mut cmd = std::process::Command::new(rnx());
    cmd.args(args)
        .current_dir(root)
        .env("RNX_CACHE_HOME", cache)
        .stdin(Stdio::null());
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
fn add_bare_name_pins_latest_stable() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![
            version_files("widget", "1.2.0", 120),
            version_files("widget", "1.3.0", 130),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj("bare", "", &url);
    let out = run(&root, &cache, &["add", "widget"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Added widget v1.3.0"),
        "{}",
        combined(&out)
    );
    assert!(combined(&out).contains("checksum: "), "{}", combined(&out));
    let manifest = std::fs::read_to_string(root.join("Project.config")).unwrap();
    assert!(manifest.contains("widget: \"^1.3.0\""), "{manifest}");
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("source: \"registry:widget@1.3.0\""), "{text}");
    assert!(text.contains("version: \"1.3.0\""), "{text}");
}

#[test]
fn add_exact_range_pins_exact() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![
            version_files("widget", "1.2.0", 120),
            version_files("widget", "1.3.0", 130),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj("exact", "", &url);
    let out = run(&root, &cache, &["add", "widget@1.2.0"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Added widget v1.2.0"),
        "{}",
        combined(&out)
    );
    let manifest = std::fs::read_to_string(root.join("Project.config")).unwrap();
    assert!(manifest.contains("widget: \"1.2.0\""), "{manifest}");
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("source: \"registry:widget@1.2.0\""), "{text}");
}

#[test]
fn add_caret_range_resolves_max() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![
            version_files("widget", "1.2.0", 120),
            version_files("widget", "1.3.0", 130),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj("caret", "", &url);
    let out = run(&root, &cache, &["add", "widget@^1.0.0"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Added widget v1.3.0"),
        "{}",
        combined(&out)
    );
    let manifest = std::fs::read_to_string(root.join("Project.config")).unwrap();
    assert!(manifest.contains("widget: \"^1.0.0\""), "{manifest}");
}

#[test]
fn add_bare_name_skips_prereleases() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![
            version_files("widget", "1.3.0", 130),
            version_files("widget", "1.4.0-alpha", 140),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj("pre-skip", "", &url);
    let out = run(&root, &cache, &["add", "widget"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Added widget v1.3.0"),
        "{}",
        combined(&out)
    );
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("source: \"registry:widget@1.3.0\""), "{text}");
}

#[test]
fn add_explicit_prerelease_range_is_honored() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![
            version_files("widget", "1.3.0", 130),
            version_files("widget", "1.4.0-alpha", 140),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj("pre-exact", "", &url);
    let out = run(&root, &cache, &["add", "widget@1.4.0-alpha"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Added widget v1.4.0-alpha"),
        "{}",
        combined(&out)
    );
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(
        text.contains("source: \"registry:widget@1.4.0-alpha\""),
        "{text}"
    );
}

#[test]
fn add_unknown_package_fails_actionably() {
    let packages = BTreeMap::new();
    let url = serve(packages);
    let (_dir, root, cache) = proj("missing", "", &url);
    let out = run(&root, &cache, &["add", "nope"]);
    assert!(!out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("does not exist"),
        "{}",
        combined(&out)
    );
    assert!(!root.join("Project.deplock").exists());
}

#[test]
fn add_dist_tag_is_rejected_without_pinning() {
    let mut packages = BTreeMap::new();
    packages.insert("widget".to_string(), vec![version_files("widget", "1.3.0", 130)]);
    let url = serve(packages);
    let (_dir, root, cache) = proj("tag", "", &url);
    let out = run(&root, &cache, &["add", "widget@beta"]);
    assert!(!out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("not a valid range"),
        "{}",
        combined(&out)
    );
    assert!(combined(&out).contains("dist-tags"), "{}", combined(&out));
    assert!(!root.join("Project.deplock").exists());
}

#[test]
fn add_rejects_conflicting_local_source() {
    let mut packages = BTreeMap::new();
    packages.insert("widget".to_string(), vec![version_files("widget", "1.3.0", 130)]);
    let url = serve(packages);
    let (dir, root, cache) = proj(
        "conflict",
        "        widget: { path: \"../local-widget\" }\n",
        &url,
    );
    let local = dir.join("local-widget");
    std::fs::create_dir_all(local.join("src")).unwrap();
    std::fs::write(
        local.join("Project.config"),
        "export default {\n    project: {\n        name: \"widget\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(local.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
    let out = run(&root, &cache, &["add", "widget"]);
    assert!(!out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("already declared with a different source"),
        "{}",
        combined(&out)
    );
}

#[test]
fn add_engine_mismatch_fails_closed() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![version_files_engine("widget", "2.0.0", 200, ">=999.0.0")],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj("engine", "", &url);
    let out = run(&root, &cache, &["add", "widget"]);
    assert!(!out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("needs engine"),
        "{}",
        combined(&out)
    );
}

#[test]
fn add_is_idempotent() {
    let mut packages = BTreeMap::new();
    packages.insert(
        "widget".to_string(),
        vec![
            version_files("widget", "1.2.0", 120),
            version_files("widget", "1.3.0", 130),
        ],
    );
    let url = serve(packages);
    let (_dir, root, cache) = proj("idem", "", &url);
    let out = run(&root, &cache, &["add", "widget"]);
    assert!(out.status.success(), "{}", combined(&out));
    let before = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    let manifest_before = std::fs::read_to_string(root.join("Project.config")).unwrap();
    let out = run(&root, &cache, &["add", "widget"]);
    assert!(out.status.success(), "{}", combined(&out));
    let after = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    let manifest_after = std::fs::read_to_string(root.join("Project.config")).unwrap();
    assert_eq!(before, after);
    assert_eq!(manifest_before, manifest_after);
}

#[test]
fn add_path_flow_is_unchanged() {
    let packages = BTreeMap::new();
    let url = serve(packages);
    let (dir, root, cache) = proj("path", "", &url);
    let local = dir.join("local-widget");
    std::fs::create_dir_all(local.join("src")).unwrap();
    std::fs::write(
        local.join("Project.config"),
        "export default {\n    project: {\n        name: \"widget\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    std::fs::write(
        local.join("src").join("main.rnx"),
        "fn Main(): Int { return 0; }\n",
    )
    .unwrap();
    let out = run(&root, &cache, &["add", "widget", "--path", "../local-widget"]);
    assert!(out.status.success(), "{}", combined(&out));
    assert!(
        combined(&out).contains("Added widget v0.1.0"),
        "{}",
        combined(&out)
    );
    let text = std::fs::read_to_string(root.join("Project.deplock")).unwrap();
    assert!(text.contains("source: \"path:"), "{text}");
}
