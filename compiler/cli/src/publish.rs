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
    let root = frontend::oci::project_artifact_cache(package_dir);
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
            if rel.starts_with("blobs/") || rel == "blobs" {
                continue;
            }
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
                format!("local artifact `{rel}` is not in the asserted index (stale image or stale --artifact digest)"),
            ));
        }
    }
    Ok(checked)
}

pub const PUBLISH_STAGE_DIR: &str = ".rnx-cache/publish";

pub fn push_plan() -> Option<(String, Option<String>)> {
    let base = std::env::var(crate::task::PUSH_URL_ENV).ok()?;
    if base.trim().is_empty() {
        return None;
    }
    let token = std::env::var(crate::task::PUSH_TOKEN_ENV).ok();
    Some((base, token))
}

fn staged_role_kind(role: &str) -> Option<&'static str> {
    match role {
        "runtime" | "wasm" => Some("binary"),
        "bindgen" => Some("bindings"),
        "media" => Some("media"),
        _ => None,
    }
}

pub fn collect_pipeline_inputs(
    package_dir: &std::path::Path,
    cfg: &frontend::project::ProjectConfig,
) -> Result<Vec<frontend::oci::AssembleInput>, Diagnostic> {
    let mut inputs = Vec::new();
    if let Some(driver) = cfg.bindgen.as_ref() {
        let mut upstream_bytes = Vec::new();
        let mut sorted_headers = driver.headers.clone();
        sorted_headers.sort();
        for h in &sorted_headers {
            let bytes = std::fs::read(package_dir.join(h)).map_err(|e| {
                Diagnostic::new(Code::E108, format!("pipeline: cannot read bindgen header `{h}`: {e}"))
            })?;
            upstream_bytes.extend_from_slice(&bytes);
        }
        let upstream_name = driver.upstream.clone().unwrap_or_else(|| cfg.name.clone());
        let upstream = format!("{upstream_name}@sha256:{}", frontend::checksum::Sha256::hexdigest(&upstream_bytes));
        let mut outputs: Vec<(&String, &frontend::project::BindgenOutputDecl)> = driver.outputs.iter().collect();
        outputs.sort_by(|a, b| a.0.cmp(b.0));
        for (dir, decl) in outputs {
            let path = package_dir.join(dir).join(&decl.title);
            let bytes = std::fs::read(&path).map_err(|e| {
                Diagnostic::new(
                    Code::E108,
                    format!("pipeline: bindgen output `{}` is missing: {e} (run `rnx task bindgen` first)", path.display()),
                )
            })?;
            let target = decl.target.clone().unwrap_or_else(frontend::native::host_triple);
            inputs.push(frontend::oci::AssembleInput {
                role: frontend::bindgen::ROLE.to_string(),
                title: decl.title.clone(),
                kind: frontend::bindgen::KIND.to_string(),
                target: target.clone(),
                platform: frontend::oci::platform_for_triple(&target),
                generator: frontend::bindgen::generator_string(),
                upstream: upstream.clone(),
                bytes,
            });
        }
    }
    let stage = package_dir.join(PUBLISH_STAGE_DIR);
    if stage.is_dir() {
        let mut staged: Vec<std::path::PathBuf> = Vec::new();
        let mut stack = vec![stage.clone()];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<std::path::PathBuf> = std::fs::read_dir(&dir)
                .map(|r| r.filter_map(|e| e.ok().map(|e| e.path())).collect())
                .unwrap_or_default();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    stack.push(path);
                } else if path.is_file() {
                    staged.push(path);
                }
            }
        }
        staged.sort();
        for path in staged {
            let rel = path.strip_prefix(&stage).map(|r| r.to_string_lossy().replace('\\', "/")).unwrap_or_default();
            let mut parts = rel.splitn(3, '/');
            let (target, role, title) = match (parts.next(), parts.next(), parts.next()) {
                (Some(t), Some(r), Some(title)) if !t.is_empty() && !r.is_empty() && !title.is_empty() => (t, r, title),
                _ => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("pipeline: staged file `{rel}` must live at `{PUBLISH_STAGE_DIR}/<target>/<role>/<title>`"),
                    ));
                }
            };
            if !frontend::oci::valid_target(target) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("pipeline: staged target `{target}` must be an arch-vendor-os-env triple"),
                ));
            }
            let kind = staged_role_kind(role).ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("pipeline: staged role `{role}` must be one of `runtime`, `wasm`, `bindgen`, or `media`"),
                )
            })?;
            if !frontend::oci::valid_title(title) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("pipeline: staged title `{title}` must be a plain filename"),
                ));
            }
            let bytes = std::fs::read(&path).map_err(|e| {
                Diagnostic::new(Code::E108, format!("pipeline: cannot read staged `{rel}`: {e}"))
            })?;
            let package_name = cfg.name.clone();
            let upstream = format!("{package_name}@sha256:{}", frontend::checksum::Sha256::hexdigest(&bytes));
            inputs.push(frontend::oci::AssembleInput {
                role: role.to_string(),
                title: title.to_string(),
                kind: kind.to_string(),
                target: target.to_string(),
                platform: frontend::oci::platform_for_triple(target),
                generator: format!("rnx-publish {}", env!("CARGO_PKG_VERSION")),
                upstream,
                bytes,
            });
        }
    }
    Ok(inputs)
}

fn push_request(
    agent: &ureq::Agent,
    method: &str,
    url: &str,
    token: Option<&str>,
    content_type: &str,
    body: &[u8],
) -> Result<(u16, Vec<(String, String)>, Vec<u8>), Diagnostic> {
    let mut req = match method {
        "POST" => agent.post(url),
        _ => agent.put(url),
    };
    if let Some(t) = token {
        req = req.header("Authorization", &format!("Bearer {t}"));
    }
    let mut response = req
        .header("Content-Type", content_type)
        .send(body)
        .map_err(|e| Diagnostic::new(Code::E108, format!("artifact push failed ({method} {url}): {e}")))?;
    let status = response.status().as_u16();
    let mut headers = Vec::new();
    for name in ["location", "docker-content-digest"] {
        if let Some(value) = response.headers().get(name) {
            headers.push((name.to_string(), value.to_str().unwrap_or_default().to_string()));
        }
    }
    let bytes = response.body_mut().read_to_vec().unwrap_or_default();
    Ok((status, headers, bytes))
}

pub fn push_assembly(
    base_url: &str,
    repo: &str,
    token: Option<&str>,
    assembly: &frontend::oci::Assembly,
) -> Result<(), Diagnostic> {
    let base = base_url.trim().trim_end_matches('/').to_string();
    if !base.starts_with("http://") && !base.starts_with("https://") {
        return Err(Diagnostic::new(
            Code::E108,
            format!("artifact push failed: push URL `{base}` must start with `http://` or `https://`"),
        ));
    }
    let config = ureq::config::Config::builder()
        .timeout_global(Some(std::time::Duration::from_secs(60)))
        .http_status_as_error(false)
        .build();
    let agent: ureq::Agent = config.into();
    for (digest, bytes) in assembly.blobs.iter() {
        let mono = format!("{base}/v2/{repo}/blobs/uploads/?digest={digest}");
        let (status, headers, _) = push_request(&agent, "POST", &mono, token, "application/octet-stream", bytes)?;
        if status == 201 {
            continue;
        }
        let location = headers.iter().find(|(k, _)| k == "location").map(|(_, v)| v.clone());
        let location = match location {
            Some(loc) if status == 202 => loc,
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("artifact push failed (blob {digest} answered HTTP {status}, expected 201 or a 202 with a location)"),
                ));
            }
        };
        let resume = if location.contains("digest=") { location } else { format!("{location}&digest={digest}") };
        let resume = if resume.starts_with("http") { resume } else { format!("{base}{resume}") };
        let (status, _, _) = push_request(&agent, "PUT", &resume, token, "application/octet-stream", bytes)?;
        if !(200..300).contains(&status) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("artifact push failed (blob {digest} upload answered HTTP {status})"),
            ));
        }
    }
    for (digest, bytes) in assembly.manifests.iter() {
        let url = format!("{base}/v2/{repo}/manifests/{digest}");
        let (status, _, _) = push_request(&agent, "PUT", &url, token, "application/vnd.oci.image.manifest.v1+json", bytes)?;
        if !(200..300).contains(&status) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("artifact push failed (manifest {digest} answered HTTP {status})"),
            ));
        }
    }
    let url = format!("{base}/v2/{repo}/manifests/latest");
    let (status, _, _) = push_request(&agent, "PUT", &url, token, "application/vnd.oci.image.index.v1+json", &assembly.index_bytes)?;
    if !(200..300).contains(&status) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("artifact push failed (index push answered HTTP {status})"),
        ));
    }
    Ok(())
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

pub fn package_guides(
    package_dir: &Path,
    manifest: &frontend::project::Manifest,
) -> Result<Vec<frontend::pack::PackageGuide>, Diagnostic> {
    frontend::pack::package_guides(package_dir, manifest)
}

// The source tree the server canonicalizes: every file the pack would
// ship, as UTF-8 text. The server derives the stored bytes itself, so the
// client never supplies stored bytes under any path.
pub fn collect_publish_files(
    package_dir: &Path,
    manifest: &frontend::project::Manifest,
) -> Result<Vec<PublishFile>, Diagnostic> {
    let mut out = Vec::new();
    let strip_aux = manifest.project.as_ref().is_some_and(|c| !c.tasks.is_empty() || c.bindgen.is_some());
    for f in frontend::pack::package_file_list(package_dir, manifest)? {
        if strip_aux && f.path == frontend::project::MANIFEST_FILE {
            out.push(PublishFile { path: f.path, content: frontend::project::manifest_to_rnx_for_publish(manifest) });
            continue;
        }
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

pub fn publish_license(cfg: &frontend::project::ProjectConfig, package_dir: &Path) -> Result<String, Diagnostic> {
    if let Some(spdx) = cfg.license.as_deref() {
        frontend::license::validate_spdx_field(spdx)?;
    }
    let text = ["LICENSE", "LICENSE.txt"]
        .iter()
        .find_map(|name| {
            std::fs::read(package_dir.join(name))
                .ok()
                .and_then(|bytes| String::from_utf8(bytes).ok())
        });
    let Some(text) = text else {
        return Err(Diagnostic::new(
            Code::E108,
            "package ships no LICENSE (add a LICENSE file at the package root)",
        ));
    };
    if text.trim().is_empty() {
        return Err(Diagnostic::new(
            Code::E108,
            "package ships no LICENSE (the LICENSE file is empty)",
        ));
    }
    if let Some(spdx) = cfg.license.as_deref() {
        if !frontend::license::license_text_matches(&text, spdx) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("license mismatch: manifest declares `{spdx}` but the LICENSE file does not read as that grant"),
            ));
        }
        return Ok(spdx.to_string());
    }
    if let Some(inferred) = frontend::license::infer_spdx_from_text(&text) {
        return Ok(inferred);
    }
    Ok("MIT".to_string())
}

pub fn publish_frame(
    cfg: &frontend::project::ProjectConfig,
    readme: &str,
    files: &[PublishFile],
    docs: &str,
    guides: &[frontend::pack::PackageGuide],
    artifact_digest: Option<&str>,
) -> serde_json::Value {
    let file_values: Vec<serde_json::Value> = files
        .iter()
        .map(|f| serde_json::json!({"path": f.path, "content": f.content}))
        .collect();
    let docs_value: serde_json::Value =
        serde_json::from_str(docs).unwrap_or(serde_json::Value::Null);
    let guide_values: Vec<serde_json::Value> = guides
        .iter()
        .map(|g| {
            serde_json::json!({
                "slug": g.slug,
                "title": g.title,
                "source": g.source,
                "description": g.description,
                "section": g.section,
                "order": g.order,
            })
        })
        .collect();
    let license = cfg.license.clone().unwrap_or_else(|| "MIT".to_string());
    serde_json::json!({
        "name": cfg.name,
        "version": cfg.version,
        "files": file_values,
        "description": cfg.description,
        "readme": readme,
        "docs": docs_value,
        "guides": guide_values,
        "engineRange": cfg.engine,
        "keywords": cfg.keywords,
        "tags": [],
        "license": license,
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
    fn publish_frame_excludes_bindgen_output_but_pipeline_keeps_it() {
        let dir = fixture_project("genex", None);
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\",\n        description: \"Probe package\"\n    },\n    bindgen: {\n        headers: [\"build/foo.h\"],\n        native_lib: \"foo\",\n        outputs: {\n            \"src/gen\": { title: \"foo.rnx\" }\n        }\n    }\n}\n",
        )
        .unwrap();
        std::fs::create_dir_all(dir.join("build")).unwrap();
        std::fs::write(dir.join("build").join("foo.h"), "int foo_add(int a, int b);\n").unwrap();
        std::fs::create_dir_all(dir.join("src").join("gen")).unwrap();
        std::fs::write(dir.join("src").join("gen").join("foo.rnx"), "generated\n").unwrap();
        let cfg = fixture_cfg(&dir);
        let manifest =
            frontend::project::Manifest { project: Some(cfg.clone()), workspace: None };
        let files = collect_publish_files(&dir, &manifest).unwrap();
        assert!(files.iter().any(|f| f.path == "src/main.rnx"));
        assert!(!files.iter().any(|f| f.path.starts_with("src/gen")));
        let inputs = collect_pipeline_inputs(&dir, &cfg).unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].title, "foo.rnx");
        assert_eq!(inputs[0].bytes, b"generated\n");
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
        let frame = publish_frame(&cfg, &readme, &files, &docs, &[], None);
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
    fn frame_carries_guides_as_raw_source_without_html() {
        let dir = fixture_project("frame-guides", Some("# probe\n"));
        let cfg = fixture_cfg(&dir);
        let guides = vec![
            frontend::pack::PackageGuide {
                slug: "start".to_string(),
                title: "Start".to_string(),
                source: "# Start\n".to_string(),
                description: "Log in first.".to_string(),
                section: Some("Basics".to_string()),
                order: None,
            },
            frontend::pack::PackageGuide {
                slug: "deep-dive".to_string(),
                title: "Deep Dive".to_string(),
                source: "Body.\n".to_string(),
                description: String::new(),
                section: None,
                order: Some("2".to_string()),
            },
        ];
        let frame = publish_frame(&cfg, "# probe\n", &[], "{}", &guides, None);
        let carried = frame["guides"].as_array().unwrap();
        assert_eq!(carried.len(), 2);
        assert_eq!(carried[0]["slug"], "start");
        assert_eq!(carried[0]["title"], "Start");
        assert_eq!(carried[0]["source"], "# Start\n");
        assert_eq!(carried[0]["description"], "Log in first.");
        assert_eq!(carried[0]["section"], "Basics");
        assert!(carried[0]["order"].is_null(), "{}", carried[0]["order"]);
        assert_eq!(carried[1]["order"], "2");
        assert!(carried[1]["section"].is_null(), "{}", carried[1]["section"]);
        for entry in carried {
            assert!(entry.get("html").is_none(), "{entry}");
        }
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
        let frame = publish_frame(&cfg, "# probe\n", &[], "{}", &[], None);
        assert!(frame["artifactDigest"].is_null(), "{}", frame["artifactDigest"]);
        let digest = format!("sha256:{}", "cd".repeat(32));
        let frame = publish_frame(&cfg, "# probe\n", &[], "{}", &[], Some(&digest));
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
        let role_dir = dir.join(".rnx-cache").join("artifacts").join("runtime");
        std::fs::create_dir_all(&role_dir).unwrap();
        let blob = b"fake native bytes";
        std::fs::write(role_dir.join("libfoo.so"), blob).unwrap();
        let hash = frontend::checksum::Sha256::hexdigest(blob);
        let mut hexes = BTreeSet::new();
        hexes.insert(hash.clone());
        assert_eq!(cross_check_local_artifacts(&dir, &hexes).unwrap(), 1);
        let empty = BTreeSet::new();
        let err = cross_check_local_artifacts(&dir, &empty).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("runtime/libfoo.so"), "{}", err.message);
        std::fs::create_dir_all(dir.join(".rnx-cache").join("artifacts").join("blobs")).unwrap();
        std::fs::write(dir.join(".rnx-cache").join("artifacts").join("blobs").join(&hash), blob).unwrap();
        assert_eq!(cross_check_local_artifacts(&dir, &empty).unwrap_err().code, diagnostics::Code::E108);
        std::fs::remove_dir_all(dir.join(".rnx-cache")).unwrap();
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

    struct PushOrigin {
        base: String,
        blobs: std::sync::Arc<std::sync::Mutex<BTreeMap<String, Vec<u8>>>>,
        manifests: std::sync::Arc<std::sync::Mutex<BTreeMap<String, Vec<u8>>>>,
    }

    fn start_push_origin() -> PushOrigin {
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let blobs: std::sync::Arc<std::sync::Mutex<BTreeMap<String, Vec<u8>>>> =
            std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let manifests: std::sync::Arc<std::sync::Mutex<BTreeMap<String, Vec<u8>>>> =
            std::sync::Arc::new(std::sync::Mutex::new(BTreeMap::new()));
        let seen_blobs = blobs.clone();
        let seen_manifests = manifests.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else { continue };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                let mut content_len = 0usize;
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(0) => break,
                        Ok(_) => head.push(byte[0]),
                        Err(_) => break,
                    }
                    if head.len() > 65536 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                for line in text.lines() {
                    if let Some(rest) = line.strip_prefix("Content-Length:")
                        .or_else(|| line.strip_prefix("content-length:")) {
                        content_len = rest.trim().parse().unwrap_or(0);
                    }
                }
                let mut body = vec![0u8; content_len];
                let mut at = 0;
                while at < content_len {
                    match stream.read(&mut body[at..]) {
                        Ok(0) => break,
                        Ok(n) => at += n,
                        Err(_) => break,
                    }
                }
                body.truncate(at);
                let first = text.lines().next().unwrap_or_default().to_string();
                let mut status = "HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
                if first.starts_with("POST") && first.contains("/blobs/uploads/") {
                    if let Some(query) = first.split_whitespace().nth(1).and_then(|p| p.split('?').nth(1)) {
                        let digest = query.strip_prefix("digest=").unwrap_or_default().to_string();
                        if !digest.is_empty() && frontend::oci::is_artifact_digest(&digest) {
                            seen_blobs.lock().unwrap().insert(digest, body);
                            status = "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
                        }
                    } else {
                        status = "HTTP/1.1 202 Accepted\r\nLocation: /v2/org/sqlite3/blobs/uploads/uuid\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
                    }
                } else if first.starts_with("PUT") && first.contains("/blobs/uploads/uuid") {
                    seen_blobs.lock().unwrap().insert("resumed".to_string(), body);
                    status = "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
                } else if first.starts_with("PUT") && first.contains("/manifests/") {
                    let reference = first.split_whitespace().nth(1).unwrap_or_default().rsplit('/').next().unwrap_or_default().split('?').next().unwrap_or_default().to_string();
                    seen_manifests.lock().unwrap().insert(reference, body);
                    status = "HTTP/1.1 201 Created\r\nContent-Length: 0\r\nConnection: close\r\n\r\n".to_string();
                }
                let _ = stream.write_all(status.as_bytes());
                let _ = stream.flush();
            }
        });
        PushOrigin { base: format!("http://{addr}"), blobs, manifests }
    }

    fn pipeline_project(tag: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("pipe-{tag}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    artifacts: \"example.invalid/org/demo\"\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int {\n    return 0;\n}\n").unwrap();
        let staged = dir.join(PUBLISH_STAGE_DIR).join("x86_64-unknown-linux-gnu").join("runtime");
        std::fs::create_dir_all(&staged).unwrap();
        std::fs::write(staged.join("libdemo.so"), b"fake staged bytes").unwrap();
        dir
    }

    fn push_inputs() -> Vec<frontend::oci::AssembleInput> {
        vec![frontend::oci::AssembleInput {
            role: "runtime".to_string(),
            title: "libdemo.so".to_string(),
            kind: "binary".to_string(),
            target: "x86_64-unknown-linux-gnu".to_string(),
            platform: Some(("linux".to_string(), "amd64".to_string())),
            generator: "rnx-publish 0.1.0".to_string(),
            upstream: format!("demo@sha256:{}", "ab".repeat(32)),
            bytes: b"fake staged bytes".to_vec(),
        }]
    }

    #[test]
    fn pipeline_collects_staged_files_and_pushes_image() {
        let dir = pipeline_project("push");
        let cfg = fixture_cfg(&dir);
        let inputs = collect_pipeline_inputs(&dir, &cfg).unwrap();
        assert_eq!(inputs.len(), 1);
        assert_eq!(inputs[0].title, "libdemo.so");
        let assembly = frontend::oci::assemble_index(&inputs).unwrap();
        let origin = start_push_origin();
        push_assembly(&origin.base, "org/demo", None, &assembly).unwrap();
        let blobs = origin.blobs.lock().unwrap();
        assert!(blobs.values().any(|b| b == b"fake staged bytes"), "{blobs:?}");
        let manifests = origin.manifests.lock().unwrap();
        assert!(manifests.contains_key("latest"), "{manifests:?}");
        assert!(manifests.keys().any(|k| k.starts_with("sha256:")), "{manifests:?}");
        drop(blobs);
        drop(manifests);
        let hexes = frontend::oci::assembly_blob_hexes(&assembly);
        assert_eq!(cross_check_local_artifacts(&dir, &hexes).unwrap(), 0);
    }

    #[test]
    fn pipeline_rejects_bad_push_url_and_stray_layout() {
        let assembly = frontend::oci::assemble_index(&push_inputs()).unwrap();
        let err = push_assembly("not-a-url", "org/demo", None, &assembly).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("http"), "{}", err.message);
        let dir = pipeline_project("stray");
        std::fs::write(dir.join(PUBLISH_STAGE_DIR).join("flat.so"), b"x").unwrap();
        let cfg = fixture_cfg(&dir);
        let err = collect_pipeline_inputs(&dir, &cfg).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("<target>/<role>/<title>"), "{}", err.message);
    }

    #[test]
    fn frame_redacts_tasks_and_bindgen_but_keeps_pins() {
        let dir = crate::scratch::dir("frame-redact");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { zlib: { native: \"z\", system: true } },\n    native: { zlib: { system: false } },\n    tasks: { check: [\"rnx check\"] },\n    bindgen: { headers: [\"a.h\"], outputs: { out: { title: \"a.rnx\" } } }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int {\n    return 0;\n}\n").unwrap();
        let cfg = fixture_cfg(&dir);
        assert!(!cfg.tasks.is_empty());
        assert!(cfg.bindgen.is_some());
        let manifest = frontend::project::Manifest { project: Some(cfg), workspace: None };
        let files = collect_publish_files(&dir, &manifest).unwrap();
        let config = files.iter().find(|f| f.path == "Project.config").unwrap();
        assert!(!config.content.contains("tasks:"), "{}", config.content);
        assert!(!config.content.contains("bindgen:"), "{}", config.content);
        assert!(config.content.contains("native:"), "{}", config.content);
        assert!(config.content.contains("zlib"), "{}", config.content);
    }

    fn license_project(tag: &str, manifest_extra: &str, license_file: Option<&str>) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("license-{tag}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"probe\",\n        version: \"1.2.3\"\n    }}{manifest_extra}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int {\n    return 0;\n}\n").unwrap();
        if let Some(text) = license_file {
            std::fs::write(dir.join("LICENSE"), text).unwrap();
        }
        dir
    }

    #[test]
    fn publish_license_accepts_matching_spdx_and_infers_when_undeclared() {
        let dir = license_project("ok", ",\n    license: \"MIT\"", Some("MIT License\n\nPermission is hereby granted.\n"));
        let cfg = fixture_cfg(&dir);
        assert_eq!(publish_license(&cfg, &dir).unwrap(), "MIT");
        let dir = license_project("infer", "", Some("Licensed under the Apache License, Version 2.0\n"));
        let cfg = fixture_cfg(&dir);
        assert_eq!(publish_license(&cfg, &dir).unwrap(), "Apache-2.0");
        let frame = publish_frame(&cfg, "# probe\n", &[], "{}", &[], None);
        assert_eq!(frame["license"], "MIT");
    }

    #[test]
    fn publish_license_rejects_missing_and_mismatched_files() {
        let dir = license_project("missing", ",\n    license: \"MIT\"", None);
        let cfg = fixture_cfg(&dir);
        let err = publish_license(&cfg, &dir).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("no LICENSE"), "{}", err.message);
        let dir = license_project("mismatch", ",\n    license: \"MIT\"", Some("Licensed under the Apache License, Version 2.0\n"));
        let cfg = fixture_cfg(&dir);
        let err = publish_license(&cfg, &dir).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("mismatch"), "{}", err.message);
        let dir = license_project("unknown", ",\n    license: \"GPL-2.0\"", Some("MIT License\n"));
        let write_ok = std::fs::read_to_string(dir.join("Project.config")).is_ok();
        assert!(write_ok);
        let raw = std::fs::read_to_string(dir.join("Project.config")).unwrap();
        let _ = raw;
        let cfg_text = "export default {\n    project: { name: \"probe\", version: \"1.2.3\" },\n    license: \"GPL-2.0\"\n}\n";
        let parsed = frontend::project::parse_manifest_for_publish(cfg_text);
        assert!(parsed.is_err(), "unknown SPDX must not parse");
    }
}
