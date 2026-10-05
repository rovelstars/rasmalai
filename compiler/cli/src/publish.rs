use diagnostics::{Code, Diagnostic};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishFile {
    pub path: String,
    pub size: u64,
}

pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub checksum: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub readme: String,
    pub files: Vec<PublishFile>,
}

// metaVersion marks the X-RNX-Meta shape: 1 means keywords, readme, and
// files are present alongside the description.
const PUBLISH_META_VERSION: u32 = 1;

pub fn package_meta(
    package_dir: &std::path::Path,
    cfg: &frontend::project::ProjectConfig,
    checksum: String,
) -> Result<PackageMeta, Diagnostic> {
    let manifest =
        frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
    let files = frontend::pack::package_file_list(package_dir, &manifest)?
        .into_iter()
        .map(|f| PublishFile { path: f.path, size: f.size })
        .collect();
    Ok(PackageMeta {
        name: cfg.name.clone(),
        version: cfg.version.clone(),
        checksum,
        description: cfg.description.clone(),
        keywords: cfg.keywords.clone(),
        readme: frontend::pack::package_readme_text(package_dir, &cfg.name),
        files,
    })
}

pub fn meta_json(meta: &PackageMeta) -> String {
    let files: Vec<serde_json::Value> = meta
        .files
        .iter()
        .map(|f| serde_json::json!({"path": f.path, "size": f.size}))
        .collect();
    serde_json::json!({
        "metaVersion": PUBLISH_META_VERSION,
        "description": meta.description,
        "keywords": meta.keywords,
        "readme": meta.readme,
        "files": files,
    })
    .to_string()
}

pub fn read_tarball_meta(tarball: &std::path::Path, cwd: &std::path::Path) -> Result<(Vec<u8>, PackageMeta), Diagnostic> {
    let bytes = std::fs::read(tarball).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot read {}: {e}", tarball.display()))
    })?;
    let targets = crate::resolve_pack_targets(cwd, None)?;
    let (_, dir, cfg) = targets.packages.first().ok_or_else(|| {
        Diagnostic::new(Code::E108, "no package found; run `rnx publish` inside a project".to_string())
    })?;
    let checksum = frontend::checksum::Sha256::hexdigest(&bytes);
    Ok((bytes, package_meta(dir, cfg, checksum)?))
}

pub enum PublishOutcome {
    Published { url: String },
    Rejected { status: u16, message: String },
}

pub fn post_package(
    registry: &str,
    token: &str,
    bytes: &[u8],
    meta: &PackageMeta,
) -> Result<PublishOutcome, Diagnostic> {
    let url = registry.trim_end_matches('/').to_string();
    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(30)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    let res = agent
        .post(&url)
        .header("Authorization", &format!("Bearer {token}"))
        .header("Content-Type", "application/octet-stream")
        .header("X-RNX-Package-Name", &meta.name)
        .header("X-RNX-Package-Version", &meta.version)
        .header("X-RNX-Checksum", &meta.checksum)
        .header("X-RNX-Meta", &meta_json(meta))
        .send(bytes);
    match res {
        Ok(mut response) => {
            let status = response.status().as_u16();
            let body = response.body_mut().read_to_string().unwrap_or_default();
            if status == 200 || status == 201 {
                let pkg_url = package_url(&url, &meta.name, &body);
                Ok(PublishOutcome::Published { url: pkg_url })
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
    if status < 200 || status > 299 {
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
    format!("https://rnx.dev/packages/{name}")
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

    fn fixture_project(tag: &str, readme: Option<&str>) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-meta-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\",\n        description: \"Probe package\"\n    },\n    keywords: [\"http\", \"cli-2\"]\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        if let Some(text) = readme {
            std::fs::write(dir.join("README.md"), text).unwrap();
        }
        dir
    }

    fn fixture_cfg(dir: &std::path::Path) -> frontend::project::ProjectConfig {
        frontend::project::ProjectConfig::load_from_dir(dir).unwrap().expect("manifest")
    }

    #[test]
    fn meta_carries_readme_keywords_and_files() {
        let dir = fixture_project("full", Some("# probe\n\nUsage notes.\n"));
        let cfg = fixture_cfg(&dir);
        let meta = package_meta(&dir, &cfg, "abc".to_string()).unwrap();
        assert_eq!(meta.name, "probe");
        assert_eq!(meta.version, "1.2.3");
        assert_eq!(meta.description, "Probe package");
        assert_eq!(meta.keywords, vec!["http".to_string(), "cli-2".to_string()]);
        assert_eq!(meta.readme, "# probe\n\nUsage notes.\n");
        let paths: Vec<&str> = meta.files.iter().map(|f| f.path.as_str()).collect();
        let mut sorted = paths.clone();
        sorted.sort();
        assert_eq!(paths, sorted);
        assert!(paths.contains(&"src/main.rnx"));
        assert!(paths.contains(&"Project.config"));
        assert!(paths.contains(&"README.md"));
        for f in &meta.files {
            assert!(!f.path.contains('\\'), "{}", f.path);
        }
        let json = meta_json(&meta);
        let parsed: serde_json::Value = serde_json::from_str(&json).unwrap();
        assert_eq!(parsed["metaVersion"], 1);
        assert_eq!(parsed["readme"], "# probe\n\nUsage notes.\n");
        assert_eq!(parsed["keywords"], serde_json::json!(["http", "cli-2"]));
        assert_eq!(parsed["description"], "Probe package");
        let file_paths: Vec<&str> = parsed["files"]
            .as_array()
            .unwrap()
            .iter()
            .map(|f| f["path"].as_str().unwrap())
            .collect();
        assert!(file_paths.contains(&"src/main.rnx"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn meta_readme_falls_back_to_default_heading() {
        let dir = fixture_project("bare", None);
        let cfg = fixture_cfg(&dir);
        let meta = package_meta(&dir, &cfg, "abc".to_string()).unwrap();
        assert_eq!(meta.readme, "# probe\n");
        let json = meta_json(&meta);
        assert!(json.contains("# probe\\n"), "{json}");
        let _ = std::fs::remove_dir_all(&dir);
    }
}
