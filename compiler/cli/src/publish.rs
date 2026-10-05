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
}
