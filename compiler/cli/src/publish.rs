use diagnostics::{Code, Diagnostic};

pub struct PackageMeta {
    pub name: String,
    pub version: String,
    pub checksum: String,
}

pub fn read_tarball_meta(tarball: &std::path::Path, cwd: &std::path::Path) -> Result<(Vec<u8>, PackageMeta), Diagnostic> {
    let bytes = std::fs::read(tarball).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot read {}: {e}", tarball.display()))
    })?;
    let targets = crate::resolve_pack_targets(cwd, None)?;
    let (name, _, cfg) = targets.packages.first().ok_or_else(|| {
        Diagnostic::new(Code::E108, "no package found; run `rnx publish` inside a project".to_string())
    })?;
    let checksum = frontend::checksum::Sha256::hexdigest(&bytes);
    Ok((bytes, PackageMeta { name: name.clone(), version: cfg.version.clone(), checksum }))
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
