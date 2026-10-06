use diagnostics::{Code, Diagnostic};
use std::path::Path;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PublishFile {
    pub path: String,
    pub size: u64,
}

#[derive(Debug)]
pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub checksum: String,
    pub description: String,
    pub keywords: Vec<String>,
    pub readme: String,
    pub docs: String,
    pub files: Vec<PublishFile>,
    pub permissions: Vec<frontend::project::PermissionDecl>,
}

// metaVersion marks the X-RNX-Meta shape: 1 means keywords, readme, and
// files are present alongside the description; 2 adds the manifest with
// the declared permissions ceiling; 3 adds the docgen JSON snapshot.
const PUBLISH_META_VERSION: u32 = 3;

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

pub fn package_meta(
    package_dir: &std::path::Path,
    cfg: &frontend::project::ProjectConfig,
    checksum: String,
    docs: String,
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
        docs,
        files,
        permissions: cfg.permissions.clone().unwrap_or_default(),
    })
}

pub fn package_meta_minified(
    package_dir: &std::path::Path,
    cfg: &frontend::project::ProjectConfig,
    checksum: String,
    docs: String,
    sizes: &[(String, u64)],
) -> Result<PackageMeta, Diagnostic> {
    let manifest =
        frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
    let names = frontend::pack::package_file_list(package_dir, &manifest)?;
    let mut files = Vec::with_capacity(names.len());
    for f in names {
        let size = sizes
            .iter()
            .find(|(n, _)| n == &f.path)
            .map(|(_, s)| *s)
            .ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("cannot publish {}: minified size missing", f.path),
                )
            })?;
        files.push(PublishFile { path: f.path, size });
    }
    Ok(PackageMeta {
        name: cfg.name.clone(),
        version: cfg.version.clone(),
        checksum,
        description: cfg.description.clone(),
        keywords: cfg.keywords.clone(),
        readme: frontend::pack::package_readme_text(package_dir, &cfg.name),
        docs,
        files,
        permissions: cfg.permissions.clone().unwrap_or_default(),
    })
}

pub fn meta_json(meta: &PackageMeta) -> String {
    let files: Vec<serde_json::Value> = meta
        .files
        .iter()
        .map(|f| serde_json::json!({"path": f.path, "size": f.size}))
        .collect();
    let permissions: Vec<serde_json::Value> = meta
        .permissions
        .iter()
        .map(|d| match &d.reason {
            Some(r) => serde_json::json!({"perm": d.perm, "reason": r}),
            None => serde_json::Value::String(d.perm.clone()),
        })
        .collect();
    let docs: serde_json::Value =
        serde_json::from_str(&meta.docs).unwrap_or(serde_json::Value::Null);
    serde_json::json!({
        "metaVersion": PUBLISH_META_VERSION,
        "description": meta.description,
        "keywords": meta.keywords,
        "readme": meta.readme,
        "docs": docs,
        "files": files,
        "manifest": { "permissions": permissions },
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
    let manifest =
        frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
    gate_formatted(dir, &manifest)?;
    let docs = package_docs(dir, &manifest)?;
    let checksum = frontend::checksum::Sha256::hexdigest(&bytes);
    Ok((bytes, package_meta(dir, cfg, checksum, docs)?))
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
    fn meta_carries_readme_keywords_and_files() {
        let dir = fixture_project("full", Some("# probe\n\nUsage notes.\n"));
        let cfg = fixture_cfg(&dir);
        let manifest =
            frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
        let docs = package_docs(&dir, &manifest).unwrap();
        let meta = package_meta(&dir, &cfg, "abc".to_string(), docs).unwrap();
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
        assert_eq!(parsed["metaVersion"], 3);
        assert_eq!(parsed["readme"], "# probe\n\nUsage notes.\n");
        assert_eq!(parsed["keywords"], serde_json::json!(["http", "cli-2"]));
        assert_eq!(parsed["description"], "Probe package");
        assert!(parsed["docs"]["modules"].is_array(), "{json}");
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
        let meta = package_meta(&dir, &cfg, "abc".to_string(), "{\"modules\":[]}".to_string()).unwrap();
        assert_eq!(meta.readme, "# probe\n");
        let json = meta_json(&meta);
        assert!(json.contains("# probe\\n"), "{json}");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn meta_manifest_carries_declared_permissions() {
        let dir = std::env::temp_dir().join(format!("rnx-meta-perm-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\"\n    },\n    permissions: [{ perm: \"fs:read:/data\", reason: \"seed\" }, \"term:write\"]\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int {\n    return 0;\n}\n").unwrap();
        let cfg = fixture_cfg(&dir);
        let meta = package_meta(&dir, &cfg, "abc".to_string(), "{\"modules\":[]}".to_string()).unwrap();
        assert_eq!(meta.permissions.len(), 2);
        let parsed: serde_json::Value = serde_json::from_str(&meta_json(&meta)).unwrap();
        assert_eq!(
            parsed["manifest"]["permissions"],
            serde_json::json!([{ "perm": "fs:read:/data", "reason": "seed" }, "term:write"])
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn gate_manifest(dir: &std::path::Path) -> frontend::project::Manifest {
        let cfg = fixture_cfg(dir);
        frontend::project::Manifest { project: Some(cfg), workspace: None }
    }

    #[test]
    fn gate_passes_on_formatted_tree() {
        let dir = fixture_project("gate-ok", None);
        let manifest = gate_manifest(&dir);
        gate_formatted(&dir, &manifest).unwrap();
        let _ = std::fs::remove_dir_all(&dir);
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
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn minified_meta_uses_minified_sizes() {
        let dir = fixture_project("min-size", None);
        std::fs::write(
            dir.join("src").join("main.rnx"),
            "// header\nfn Main(): Int {\n    return 0;\n}\n",
        )
        .unwrap();
        let cfg = fixture_cfg(&dir);
        let manifest =
            frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
        let out = std::env::temp_dir().join(format!("rnx-pub-min-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&out);
        let (_, _, sizes) =
            frontend::pack::pack_package_gz_minified(&dir, &manifest, &out).unwrap();
        let meta =
            package_meta_minified(&dir, &cfg, "abc".to_string(), "{\"modules\":[]}".to_string(), &sizes)
                .unwrap();
        let main = meta.files.iter().find(|f| f.path == "src/main.rnx").unwrap();
        assert_eq!(main.size, "fn Main():Int{return 0;}".len() as u64);
        let disk = package_meta(&dir, &cfg, "abc".to_string(), "{\"modules\":[]}".to_string()).unwrap();
        let disk_main = disk.files.iter().find(|f| f.path == "src/main.rnx").unwrap();
        assert!(main.size < disk_main.size, "minify did not shrink main.rnx");
        let config = meta.files.iter().find(|f| f.path == "Project.config").unwrap();
        let disk_config = disk.files.iter().find(|f| f.path == "Project.config").unwrap();
        assert_eq!(config.size, disk_config.size);
        let _ = std::fs::remove_dir_all(&dir);
        let _ = std::fs::remove_dir_all(&out);
    }

    #[test]
    fn minified_meta_fails_closed_on_missing_size() {
        let dir = fixture_project("min-miss", None);
        let cfg = fixture_cfg(&dir);
        let err =
            package_meta_minified(&dir, &cfg, "abc".to_string(), "{\"modules\":[]}".to_string(), &[])
                .unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
