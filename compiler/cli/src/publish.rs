use diagnostics::{Code, Diagnostic};
use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishFile {
    pub path: String,
    pub content: String,
}

pub const PUBLISH_PROTOCOL: &str = "0";
pub const OCI_INDEX_CAP: usize = 1024 * 1024;

pub fn parse_artifact_digest(raw: &str) -> Result<String, Diagnostic> {
    let digest = raw.trim().to_string();
    let hex_ok = digest.len() == 71
        && digest.starts_with("sha256:")
        && digest.as_bytes()[7..].iter().all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b));
    if !hex_ok {
        return Err(Diagnostic::new(
            Code::E108,
            format!("invalid --artifact `{raw}` (expected `sha256:<64 lowercase hex chars>`)"),
        ));
    }
    Ok(digest)
}

pub fn artifact_repo_parts(bare: &str) -> Result<(String, String), Diagnostic> {
    frontend::project::validate_artifact_ref(bare).map_err(|e| Diagnostic::new(Code::E108, e))?;
    let slash = bare.find('/').unwrap_or(bare.len());
    Ok((bare[..slash].to_string(), bare[slash + 1..].to_string()))
}

pub fn fetch_oci_index(bare: &str) -> Result<(Vec<u8>, String), Diagnostic> {
    let (registry, repo) = artifact_repo_parts(bare)?;
    let url = format!("https://{registry}/v2/{repo}/manifests/latest");
    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(&url)
        .header(
            "Accept",
            "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json",
        )
        .call()
        .map_err(|e| {
            Diagnostic::new(Code::E108, format!("artifact origin unreachable (cannot reach `{registry}`: {e})"))
        })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("artifact origin unreachable (registry answered HTTP {status} for `{bare}`)"),
        ));
    }
    let bytes = response.body_mut().read_to_vec().map_err(|e| {
        Diagnostic::new(Code::E108, format!("artifact origin unreachable (cannot read index for `{bare}`: {e})"))
    })?;
    if bytes.len() > OCI_INDEX_CAP {
        return Err(Diagnostic::new(
            Code::E108,
            format!("artifact index exceeds the 1 MiB cap (ref `{bare}`)"),
        ));
    }
    let digest = format!("sha256:{}", frontend::checksum::Sha256::hexdigest(&bytes));
    Ok((bytes, digest))
}

pub fn index_blob_hexes(bare: &str, index_bytes: &[u8]) -> BTreeSet<String> {
    let mut hexes = BTreeSet::new();
    let doc: serde_json::Value = match serde_json::from_slice(index_bytes) {
        Ok(v) => v,
        Err(_) => return hexes,
    };
    let manifests = doc.get("manifests").and_then(|m| m.as_array()).cloned().unwrap_or_default();
    let (registry, repo) = match artifact_repo_parts(bare) {
        Ok(p) => p,
        Err(_) => return hexes,
    };
    for manifest in &manifests {
        let digest = manifest.get("digest").and_then(|d| d.as_str()).unwrap_or_default();
        if let Some(hex) = digest.strip_prefix("sha256:") {
            hexes.insert(hex.to_string());
        }
        let url = format!("https://{registry}/v2/{repo}/manifests/{digest}");
        let config = ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        let agent: ureq::Agent = config.into();
        let body = agent
            .get(&url)
            .header("Accept", "application/vnd.oci.image.manifest.v1+json")
            .call()
            .ok()
            .filter(|r| (200..300).contains(&r.status().as_u16()))
            .and_then(|mut r| r.body_mut().read_to_vec().ok())
            .filter(|b| b.len() <= OCI_INDEX_CAP);
        let Some(body) = body else { continue };
        let child: serde_json::Value = match serde_json::from_slice(&body) {
            Ok(v) => v,
            Err(_) => continue,
        };
        if let Some(hex) = child.get("config").and_then(|c| c.get("digest")).and_then(|d| d.as_str()).and_then(|d| d.strip_prefix("sha256:")) {
            hexes.insert(hex.to_string());
        }
        if let Some(layers) = child.get("layers").and_then(|l| l.as_array()) {
            for layer in layers {
                if let Some(hex) = layer.get("digest").and_then(|d| d.as_str()).and_then(|d| d.strip_prefix("sha256:")) {
                    hexes.insert(hex.to_string());
                }
            }
        }
    }
    hexes
}

pub fn local_artifact_hashes(package_dir: &std::path::Path) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    let root = package_dir.join(".artifacts");
    let mut stack = vec![root.clone()];
    while let Some(dir) = stack.pop() {
        let entries = std::fs::read_dir(&dir).map(|r| r.collect::<Vec<_>>()).unwrap_or_default();
        for entry in entries {
            let Ok(entry) = entry else { continue };
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !path.is_file() {
                continue;
            }
            let rel = path.strip_prefix(&root).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let bytes = std::fs::read(&path).unwrap_or_default();
            out.insert(rel, frontend::checksum::Sha256::hexdigest(&bytes));
        }
    }
    out
}

pub fn cross_check_local_artifacts(package_dir: &std::path::Path, blob_hexes: &BTreeSet<String>) -> Result<usize, Diagnostic> {
    let mut checked = 0;
    for (rel, hash) in local_artifact_hashes(package_dir) {
        checked += 1;
        if !blob_hexes.contains(&hash) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("local artifact `.artifacts/{rel}` is not in the asserted index (stale image or stale --artifact digest)"),
            ));
        }
    }
    Ok(checked)
}

pub fn gate_formatted(
    package_dir: &Path,
    manifest: &frontend::project::Manifest,
) -> Result<(), Diagnostic> {
    let files = frontend::pack::package_file_list(package_dir, manifest)?;
    let mut paths = Vec::new();
    for f in &files {
        let abs = package_dir.join(&f.path);
        if abs.is_file() {
            paths.push(abs);
        }
    }
    let outcome = crate::fmt::run_fmt(&paths, true, false);
    if outcome.code == 0 {
        return Ok(());
    }
    let mut unformatted: Vec<String> = outcome
        .stderr
        .lines()
        .filter_map(|l| l.strip_prefix("unformatted: ").map(str::to_string))
        .collect();
    unformatted.sort();
    unformatted.dedup();
    if unformatted.is_empty() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("refusing to publish: format gate failed:\n{}", outcome.stderr.trim()),
        )
        .with_hint("run `rnx fmt` then retry"));
    }
    Err(Diagnostic::new(
        Code::E108,
        format!(
            "refusing to publish {} unformatted file(s): {}",
            unformatted.len(),
            unformatted.join(", ")
        ),
    )
    .with_hint("run `rnx fmt` then retry"))
}

pub fn package_docs(
    package_dir: &Path,
    manifest: &frontend::project::Manifest,
) -> Result<String, Diagnostic> {
    frontend::pack::package_doc_json(package_dir, manifest).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot extract package docs: {}", e.message))
    })
}

// The source tree the server canonicalizes: every file the pack would
// ship, as UTF-8 text. The server derives the stored bytes itself, so the
// client never supplies stored bytes under any path.
pub fn collect_publish_files(
    package_dir: &Path,
    manifest: &frontend::project::Manifest,
) -> Result<Vec<PublishFile>, Diagnostic> {
    let mut out = Vec::new();
    for f in frontend::pack::package_file_list(package_dir, manifest)? {
        let bytes = std::fs::read(package_dir.join(&f.path)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read {}: {e}", f.path))
        })?;
        let content = String::from_utf8(bytes).map_err(|_| {
            Diagnostic::new(Code::E108, format!("cannot publish {}: not valid UTF-8", f.path))
        })?;
        out.push(PublishFile { path: f.path, content });
    }
    Ok(out)
}

pub fn publish_frame(
    cfg: &frontend::project::ProjectConfig,
    readme: &str,
    files: &[PublishFile],
    docs: &str,
    artifact_digest: Option<&str>,
) -> serde_json::Value {
    let file_values: Vec<serde_json::Value> = files
        .iter()
        .map(|f| serde_json::json!({"path": f.path, "content": f.content}))
        .collect();
    let docs_value: serde_json::Value =
        serde_json::from_str(docs).unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "name": cfg.name,
        "version": cfg.version,
        "files": file_values,
        "description": cfg.description,
        "readme": readme,
        "docs": docs_value,
        "engineRange": cfg.engine,
        "keywords": cfg.keywords,
        "tags": [],
        "license": "MIT",
        "artifactDigest": artifact_digest,
    })
}

pub enum PublishOutcome {
    Published { url: String, digest: String, artifact_digest: Option<String> },
    Rejected { status: u16, message: String },
}

pub fn post_frame(
    registry: &str,
    token: &str,
    frame: &serde_json::Value,
    name: &str,
) -> Result<PublishOutcome, Diagnostic> {
    let mut base = registry.trim().trim_end_matches('/').to_string();
    for suffix in ["/api/v0/packages", "/api/packages"] {
        if let Some(root) = base.strip_suffix(suffix) {
            base = root.trim_end_matches('/').to_string();
            break;
        }
    }
    let url = format!("{base}/api/v0/packages");
    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let body = serde_json::to_string(frame).map_err(|e| {
        Diagnostic::new(Code::E108, format!("publish failed: cannot encode request: {e}"))
    })?;
    let res = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/json")
        .header("X-RNX-Protocol", PUBLISH_PROTOCOL)
        .send(&body);
    match res {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let body = response.body_mut().read_to_string().unwrap_or_default();
            if status == 200 || status == 201 {
                let pkg_url = package_url(&url, name, &body);
                let digest = extract_json_string(&body, "manifestDigest").unwrap_or_default();
                let artifact_digest = extract_json_string(&body, "artifactDigest");
                Ok(PublishOutcome::Published { url: pkg_url, digest, artifact_digest })
            } else {
                let message = extract_json_string(&body, "error").unwrap_or_else(|| short_body(&body));
                Ok(PublishOutcome::Rejected { status, message })
            }
        }
        Err(ureq::Error::StatusCode(status)) => {
            Ok(PublishOutcome::Rejected { status, message: format!("HTTP {status}") })
        }
        Err(ureq::Error::Timeout(_)) => Err(Diagnostic::new(
            Code::E108,
            "publish failed: registry timed out after 30s".to_string(),
        )),
        Err(e) => Err(Diagnostic::new(Code::E108, format!("publish failed: {e}"))),
    }
}

pub fn fetch_bytes(url: &str) -> Result<Vec<u8>, Diagnostic> {
    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let mut response = agent
        .get(url)
        .header("Accept-Encoding", "br, gzip")
        .call()
        .map_err(|e| Diagnostic::new(Code::E108, format!("fetch failed: {e}")))?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Diagnostic::new(Code::E108, format!("fetch failed: HTTP {status}")));
    }
    response
        .body_mut()
        .read_to_vec()
        .map_err(|e| Diagnostic::new(Code::E108, format!("fetch failed: {e}")))
}

fn package_url(registry: &str, name: &str, body: &str) -> String {
    if let Some(found) = extract_json_string(body, "url") {
        if found.starts_with("http") {
            return found;
        }
        if let Some(root) = registry.split("/api/").next() {
            return format!("{root}{found}");
        }
        return found;
    }
    if let Some(root) = registry.split("/api/").next() {
        return format!("{root}/packages/{name}");
    }
    format!("https://rasmalai.rovelstars.com/packages/{name}")
}

fn extract_json_string(body: &str, key: &str) -> Option<String> {
    let needle = format!("\"{key}\"");
    let at = body.find(&needle)?;
    let rest = body[at + needle.len()..].trim_start();
    let rest = rest.strip_prefix(':')?.trim_start();
    let rest = rest.strip_prefix('"')?;
    let end = rest.find('"')?;
    Some(rest[..end].to_string())
}

fn short_body(body: &str) -> String {
    let one_line: String = body.split_whitespace().collect::<Vec<_>>().join(" ");
    if one_line.len() > 200 {
        format!("{}...", &one_line[..200])
    } else if one_line.is_empty() {
        "empty response".to_string()
    } else {
        one_line
    }
}

pub fn missing_token_diagnostic() -> Diagnostic {
    Diagnostic::new(Code::E501, "missing publisher token".to_string())
        .with_hint("pass --token <token> or set RNX_TOKEN in your environment")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Read, Write};

    const PLAIN: &[u8] = b"hello brotli registry payload";
    const BROTLI_BODY: &[u8] = &[
        27, 28, 0, 248, 197, 109, 108, 93, 247, 213, 39, 8, 69, 116, 204, 65, 84, 169, 37, 47,
        81, 230, 28, 59, 215, 205, 199, 168, 86, 44, 11,
    ];

    fn serve_once(body: Vec<u8>, encoding: &'static str) -> (String, std::sync::mpsc::Receiver<Vec<u8>>) {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        request.extend_from_slice(&chunk[..n]);
                        if request.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(request);
            let head = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Encoding: {encoding}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(&body).unwrap();
        });
        (format!("http://{addr}/pkg"), rx)
    }

    #[test]
    fn fetch_decodes_brotli_when_advertised() {
        let (url, seen) = serve_once(BROTLI_BODY.to_vec(), "br");
        let bytes = fetch_bytes(&url).unwrap();
        assert_eq!(bytes, PLAIN);
        let request = String::from_utf8_lossy(&seen.recv().unwrap()).into_owned();
        assert!(request.contains("br"), "{request}");
    }

    #[test]
    fn fetch_decodes_gzip_when_advertised() {
        let gz = frontend::gzip::compress_gzip(PLAIN);
        assert!(!gz.is_empty());
        let (url, seen) = serve_once(gz, "gzip");
        let bytes = fetch_bytes(&url).unwrap();
        assert_eq!(bytes, PLAIN);
        let request = String::from_utf8_lossy(&seen.recv().unwrap()).into_owned();
        assert!(request.contains("gzip"), "{request}");
    }

    fn fixture_project(tag: &str, readme: Option<&str>) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("frame-{tag}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\",\n        description: \"Probe package\"\n    },\n    keywords: [\"http\", \"cli-2\"]\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int {\n    return 0;\n}\n").unwrap();
        if let Some(text) = readme {
            std::fs::write(dir.join("README.md"), text).unwrap();
        }
        dir
    }

    fn fixture_cfg(dir: &std::path::Path) -> frontend::project::ProjectConfig {
        frontend::project::ProjectConfig::load_from_dir(dir).unwrap().expect("manifest")
    }

    #[test]
    fn frame_carries_name_version_files_and_docs() {
        let dir = fixture_project("full", Some("# probe\n\nUsage notes.\n"));
        let cfg = fixture_cfg(&dir);
        let manifest =
            frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
        let docs = package_docs(&dir, &manifest).unwrap();
        let files = collect_publish_files(&dir, &manifest).unwrap();
        let readme = frontend::pack::package_readme_text(&dir, &cfg.name);
        assert_eq!(readme, "# probe\n\nUsage notes.\n");
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
        assert!(paths.contains(&"src/main.rnx"));
        assert!(paths.contains(&"Project.config"));
        assert!(paths.contains(&"README.md"));
        let frame = publish_frame(&cfg, &readme, &files, &docs, None);
        assert_eq!(frame["name"], "probe");
        assert_eq!(frame["version"], "1.2.3");
        assert_eq!(frame["description"], "Probe package");
        assert_eq!(frame["keywords"], serde_json::json!(["http", "cli-2"]));
        let file_paths: Vec<&str> = frame["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(file_paths.contains(&"src/main.rnx"));
        let main = frame["files"].as_array().unwrap().iter().find(|f| f["path"] == "src/main.rnx").unwrap();
        assert!(main["content"].as_str().unwrap().contains("return 0"));
    }

    #[test]
    fn frame_posts_to_v0_with_protocol_zero() {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let (mut stream, _) = listener.accept().unwrap();
            let mut request = Vec::new();
            let mut chunk = [0u8; 512];
            loop {
                match stream.read(&mut chunk) {
                    Ok(0) => break,
                    Ok(n) => {
                        request.extend_from_slice(&chunk[..n]);
                        if request.windows(4).any(|w| w == b"\r\n\r\n") {
                            break;
                        }
                    }
                    Err(_) => break,
                }
            }
            let _ = tx.send(request);
            let body = "{\"success\": true, \"url\": \"/packages/probe\", \"version\": \"1.2.3\", \"manifestDigest\": \"ab\"}";
            let head = format!(
                "HTTP/1.1 201 Created\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                body.len()
            );
            stream.write_all(head.as_bytes()).unwrap();
            stream.write_all(body.as_bytes()).unwrap();
        });
        let frame = serde_json::json!({"name": "probe", "version": "1.2.3", "files": []});
        let outcome = post_frame(&format!("http://{addr}"), "tok", &frame, "probe").unwrap();
        let request = String::from_utf8_lossy(&rx.recv().unwrap()).into_owned();
        let head = request.split("\r\n\r\n").next().unwrap_or("").to_string();
        assert!(head.contains("POST /api/v0/packages "), "{head}");
        assert!(head.to_ascii_lowercase().contains("x-rnx-protocol: 0"), "{head}");
        match outcome {
            PublishOutcome::Published { url, digest, .. } => {
                assert!(url.contains("/packages/probe"), "{url}");
                assert_eq!(digest, "ab");
            }
            PublishOutcome::Rejected { status, message } => panic!("rejected {status}: {message}"),
        }
    }

    #[test]
    fn collect_refuses_non_utf8_files() {
        let dir = fixture_project("nonutf", None);
        std::fs::write(dir.join("src").join("blob.rnx"), [0xff, 0xfe, 0x00]).unwrap();
        let cfg = fixture_cfg(&dir);
        let manifest =
            frontend::project::Manifest { project: Some(cfg), workspace: None };
        let err = collect_publish_files(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("not valid UTF-8"), "{}", err.message);
    }

    #[test]
    fn gate_passes_on_formatted_tree() {
        let dir = fixture_project("gate-ok", None);
        let manifest = gate_manifest(&dir);
        gate_formatted(&dir, &manifest).unwrap();
    }

    fn gate_manifest(dir: &std::path::Path) -> frontend::project::Manifest {
        let cfg = fixture_cfg(dir);
        frontend::project::Manifest { project: Some(cfg), workspace: None }
    }

    #[test]
    fn gate_refuses_unformatted_input() {
        let dir = fixture_project("gate-bad", None);
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main():Int{return 0;}\n").unwrap();
        let manifest = gate_manifest(&dir);
        let err = gate_formatted(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("unformatted"), "{}", err.message);
        assert!(err.message.contains("main.rnx"), "{}", err.message);
    }

    #[test]
    fn artifact_digest_accepts_lowercase_hex_only() {
        let good = format!("sha256:{}", "ab".repeat(32));
        assert_eq!(parse_artifact_digest(&good).unwrap(), good);
        for bad in [
            "sha256:ABCDEF".to_string() + &"ab".repeat(29),
            "sha256:xyz".to_string(),
            "ab".repeat(32),
            String::new(),
        ] {
            let err = parse_artifact_digest(&bad).unwrap_err();
            assert_eq!(err.code, diagnostics::Code::E108);
        }
    }

    #[test]
    fn artifact_frame_carries_digest_or_null() {
        let dir = fixture_project("frame-art", None);
        let cfg = fixture_cfg(&dir);
        let frame = publish_frame(&cfg, "# probe\n", &[], "{}", None);
        assert!(frame["artifactDigest"].is_null(), "{}", frame["artifactDigest"]);
        let digest = format!("sha256:{}", "cd".repeat(32));
        let frame = publish_frame(&cfg, "# probe\n", &[], "{}", Some(&digest));
        assert_eq!(frame["artifactDigest"], digest);
    }

    #[test]
    fn artifact_parts_split_registry_from_repo() {
        let (registry, repo) = artifact_repo_parts("ghcr.io/org/sqlite3").unwrap();
        assert_eq!(registry, "ghcr.io");
        assert_eq!(repo, "org/sqlite3");
        let err = artifact_repo_parts("ghcr.io/org/sqlite3:1.0.0").unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
    }

    #[test]
    fn artifact_cross_check_rejects_files_missing_from_index() {
        let dir = fixture_project("xcheck", None);
        std::fs::create_dir_all(dir.join(".artifacts").join("runtime")).unwrap();
        let blob = b"fake native bytes";
        std::fs::write(dir.join(".artifacts").join("runtime").join("libfoo.so"), blob).unwrap();
        let hash = frontend::checksum::Sha256::hexdigest(blob);
        let mut hexes = BTreeSet::new();
        hexes.insert(hash.clone());
        assert_eq!(cross_check_local_artifacts(&dir, &hexes).unwrap(), 1);
        let empty = BTreeSet::new();
        let err = cross_check_local_artifacts(&dir, &empty).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("runtime/libfoo.so"), "{}", err.message);
        std::fs::remove_dir_all(dir.join(".artifacts")).unwrap();
        assert_eq!(cross_check_local_artifacts(&dir, &empty).unwrap(), 0);
    }

    #[test]
    fn artifact_index_hexes_collect_manifest_digests_when_origin_unreachable() {
        let hex = "aa".repeat(32);
        let index = serde_json::json!({
            "schemaVersion": 2,
            "manifests": [
                { "digest": format!("sha256:{hex}"), "annotations": {} },
                { "digest": "not-a-digest", "annotations": {} }
            ]
        });
        let bytes = serde_json::to_vec(&index).unwrap();
        let hexes = index_blob_hexes("example.invalid/org/x", &bytes);
        assert_eq!(hexes.len(), 1, "{hexes:?}");
        assert!(hexes.contains(&hex), "{hexes:?}");
    }
}
