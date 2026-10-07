struct UreqTransport {
    agent: ureq::Agent,
}

impl UreqTransport {
    fn new() -> UreqTransport {
        let config = ureq::config::Config::builder()
            .timeout_global(Some(std::time::Duration::from_secs(30)))
            .http_status_as_error(false)
            .build();
        UreqTransport { agent: config.into() }
    }

    fn reply(
        what: &str,
        result: Result<ureq::http::Response<ureq::Body>, ureq::Error>,
    ) -> Result<frontend::fetch::RegistryReply, diagnostics::Diagnostic> {
        let mut response = result.map_err(|e| match e {
            ureq::Error::Timeout(_) => diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!("{what} timed out after 30s"),
            ),
            other => diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!("{what} failed: {other}"),
            ),
        })?;
        let status = response.status().as_u16();
        let spec_header = response
            .headers()
            .get("rnx-registry-spec")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let body = response.body_mut().read_to_vec().map_err(|e| {
            diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!("{what} failed: {e}"),
            )
        })?;
        Ok(frontend::fetch::RegistryReply { status, spec_header, body })
    }
}

impl frontend::fetch::RegistryTransport for UreqTransport {
    fn get(&self, url: &str) -> Result<frontend::fetch::RegistryReply, diagnostics::Diagnostic> {
        let req = self.agent.get(url).header("Accept", "application/json");
        let req = if frontend::fetch::origin_direct() {
            req.header("Cache-Control", "no-cache").header("Pragma", "no-cache")
        } else {
            req
        };
        Self::reply("registry request", req.call())
    }

    fn post_json(
        &self,
        url: &str,
        body: &str,
    ) -> Result<frontend::fetch::RegistryReply, diagnostics::Diagnostic> {
        let req = self
            .agent
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json");
        let req = if frontend::fetch::origin_direct() {
            req.header("Cache-Control", "no-cache").header("Pragma", "no-cache")
        } else {
            req
        };
        Self::reply("registry request", req.send(body.as_bytes()))
    }
}

pub fn install_registry_transport() {
    frontend::fetch::set_registry_transport(std::sync::Arc::new(UreqTransport::new()));
}

pub fn ensure_stdlib_seeded() -> Result<bool, diagnostics::Diagnostic> {
    install_registry_transport();
    if frontend::stdlib_seed::missing_std_packages().is_empty() {
        return Ok(false);
    }
    let (default, overrides) = frontend::stdlib_seed::std_registry_from_env();
    let base = frontend::fetch::registry_base_for("@std/prelude", default.as_ref(), &overrides);
    match frontend::stdlib_seed::seed_stdlib_cache(default.as_ref(), &overrides) {
        Ok(report) => {
            println!(
                "Seeded {} stdlib packages from {}",
                report.packages.len(),
                report.registry
            );
            Ok(true)
        }
        Err(err) => {
            if err.message.contains(&base) {
                if err
                    .hint
                    .as_deref()
                    .is_some_and(|h| h.contains("rnx fetch-std"))
                {
                    return Err(err);
                }
                return Err(err.with_hint(
                    "check the registry URL and network, then retry `rnx fetch-std`",
                ));
            }
            Err(diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!(
                    "stdlib cache seeding from registry `{base}` failed: {}",
                    err.message
                ),
            )
            .with_hint("check the registry URL and network, then retry `rnx fetch-std`"))
        }
    }
}

pub fn generate_stdlib_docs_json(
    out_dir: &std::path::Path,
) -> Result<std::path::PathBuf, diagnostics::Diagnostic> {
    ensure_stdlib_seeded()?;
    let mut all = Vec::new();
    for name in stdlib::MODULES {
        let src = frontend::modules::load_std_module_source(name)?;
        let module = frontend::parser::Parser::parse_module(&src).map_err(|mut e| {
            e.message = format!("@std/{name}: {}", e.message);
            e
        })?;
        all.push(frontend::doc::collect_module(name, &module, true));
    }
    all.sort_by(|a, b| a.name.cmp(&b.name));
    std::fs::create_dir_all(out_dir).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write {}: {e}", out_dir.display()),
        )
    })?;
    let out_path = out_dir.join("api.json");
    std::fs::write(&out_path, frontend::doc::modules_to_json(&all)).map_err(|e| {
        diagnostics::Diagnostic::new(
            diagnostics::Code::E108,
            format!("cannot write {}: {e}", out_path.display()),
        )
    })?;
    Ok(out_path)
}

#[cfg(test)]
mod tests {
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();
    static TAG_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    struct EnvGuard {
        lock: Option<std::sync::MutexGuard<'static, ()>>,
        prev_cache: Option<String>,
        prev_registry: Option<String>,
        prev_proxies: Vec<(String, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            frontend::stdvfs::clear();
            unsafe {
                match self.prev_cache.clone() {
                    Some(v) => std::env::set_var("RNX_CACHE_HOME", v),
                    None => std::env::remove_var("RNX_CACHE_HOME"),
                }
                match self.prev_registry.clone() {
                    Some(v) => std::env::set_var("RNX_REGISTRY", v),
                    None => std::env::remove_var("RNX_REGISTRY"),
                }
            }
            for (key, prev) in self.prev_proxies.drain(..) {
                unsafe {
                    match prev {
                        Some(v) => std::env::set_var(&key, v),
                        None => std::env::remove_var(&key),
                    }
                }
            }
            self.lock.take();
        }
    }

    fn isolate(tag: &str) -> (EnvGuard, std::path::PathBuf) {
        let lock = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let n = TAG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "rnx-docgen-{tag}-{n}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prev_cache = std::env::var("RNX_CACHE_HOME").ok();
        let prev_registry = std::env::var("RNX_REGISTRY").ok();
        unsafe {
            std::env::set_var("RNX_CACHE_HOME", &dir);
            std::env::remove_var("RNX_REGISTRY");
        }
        let mut prev_proxies = Vec::new();
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            prev_proxies.push((key.to_string(), std::env::var(key).ok()));
            unsafe {
                std::env::remove_var(key);
            }
        }
        frontend::stdvfs::clear();
        (
            EnvGuard { lock: Some(lock), prev_cache, prev_registry, prev_proxies },
            dir,
        )
    }

    fn set_registry(url: &str) {
        unsafe {
            std::env::set_var("RNX_REGISTRY", url);
        }
    }

    fn fixture_tarball(version: &str) -> (Vec<u8>, String) {
        let manifest = format!(
            "export default {{\n    project: {{\n        name: \"@std/seed\",\n        version: \"{version}\"\n    }}\n}}\n"
        );
        let mut buf = Vec::new();
        {
            let mut tar = frontend::tar::TarWriter::new(&mut buf);
            tar.add_file("Project.config", manifest.as_bytes()).unwrap();
            tar.add_file("src/main.rnx", b"export fn hello(): Int { return 1; }\n")
                .unwrap();
            tar.add_file("src/http.rnx", b"export fn hello(): Int { return 1; }\n")
                .unwrap();
            tar.finish().unwrap();
        }
        let gz = frontend::gzip::compress_gzip(&buf);
        let sha = frontend::checksum::Sha256::hexdigest(&gz);
        (gz, sha)
    }

    fn node_json(full: &str, version: &str, integrity: &str) -> String {
        format!(
            "{{\"full\": \"{full}\", \"version\": \"{version}\", \"path\": \"{full}@{version}/chunks\", \"integrity\": \"{integrity}\", \"engineRange\": \"\", \"deps\": [], \"yanked\": false}}"
        )
    }

    fn resolve_json_static(nodes: &[String]) -> String {
        let mut resolved = String::new();
        for node in nodes {
            let value = frontend::fetch::parse_json(node.as_bytes()).unwrap();
            let full = value.get("full").and_then(|v| v.as_str()).unwrap_or_default();
            let version = value
                .get("version")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if !resolved.is_empty() {
                resolved.push_str(", ");
            }
            resolved.push_str(&format!("\"{full}\": \"{version}\""));
        }
        format!(
            "{{\"resolved\": {{{resolved}}}, \"base\": \"\", \"levels\": [[{}]]}}",
            nodes.join(", ")
        )
    }

    fn fallback_manifest(tarball: &[u8]) -> String {
        let digest = frontend::checksum::Sha256::hexdigest(tarball);
        format!(
            "{{\"entries\": [{{\"name\": \"\", \"size\": {}, \"dir\": false, \"chunks\": [\"{digest}\"]}}]}}",
            tarball.len()
        )
    }

    struct MockRegistry {
        base: String,
        log: Arc<Mutex<Vec<(String, String, Vec<u8>)>>>,
    }

    impl MockRegistry {
        fn start(resolve_body: String, tarball: Vec<u8>) -> MockRegistry {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            let log: Arc<Mutex<Vec<(String, String, Vec<u8>)>>> =
                Arc::new(Mutex::new(Vec::new()));
            let seen = log.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let Some((method, path, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    seen.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push((method.clone(), path.clone(), body));
                    let with_spec: &[(&str, &str)] = &[("rnx-registry-spec", "1")];
                    if method == "GET" && path == "/api/version" {
                        let body =
                            "{\"spec\": 1, \"capabilities\": [], \"registry\": \"test\"}";
                        respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                    } else if method == "POST" && path == "/api/resolve" {
                        respond(
                            &mut stream,
                            200,
                            "application/json",
                            resolve_body.as_bytes(),
                            with_spec,
                        );
                    } else if method == "GET" && path.ends_with("/chunks") {
                        let body = fallback_manifest(&tarball);
                        respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                    } else if method == "GET" && path.contains("/chunk/") {
                        let hash = path.rsplit('/').next().unwrap_or_default().to_string();
                        let digest = frontend::checksum::Sha256::hexdigest(&tarball);
                        if digest == hash {
                            respond(
                                &mut stream,
                                200,
                                "application/octet-stream",
                                &tarball,
                                with_spec,
                            );
                        } else {
                            respond(
                                &mut stream,
                                404,
                                "application/json",
                                b"{\"message\": \"chunk not found\"}",
                                with_spec,
                            );
                        }
                    } else {
                        respond(
                            &mut stream,
                            404,
                            "application/json",
                            b"{\"message\": \"unknown\"}",
                            with_spec,
                        );
                    }
                }
            });
            MockRegistry { base, log }
        }

        fn resolve_posts(&self) -> usize {
            self.log
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .filter(|(m, p, _)| m == "POST" && p == "/api/resolve")
                .count()
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String, Vec<u8>)> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) => return None,
                Ok(_) => head.push(byte[0]),
                Err(_) => return None,
            }
            if head.len() > 65536 {
                return None;
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
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                length = rest.trim().parse::<usize>().unwrap_or(0);
            }
            if let Some(rest) = line.strip_prefix("content-length:") {
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
        Some((method, path, body))
    }

    fn respond(
        stream: &mut std::net::TcpStream,
        status: u16,
        content_type: &str,
        body: &[u8],
        extra: &[(&str, &str)],
    ) {
        let text = match status {
            200 => "OK",
            _ => "Not Found",
        };
        let mut head = format!(
            "HTTP/1.1 {status} {text}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
            body.len()
        );
        for (k, v) in extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    fn std_nodes(version: &str, integrity: &str) -> Vec<String> {
        frontend::stdlib_seed::std_package_names()
            .iter()
            .map(|full| node_json(full, version, integrity))
            .collect()
    }

    #[test]
    fn cold_cache_seeds_once_then_generates_docs() {
        let (_guard, dir) = isolate("cold");
        let (gz, sha) = fixture_tarball("1.0.0");
        let server = MockRegistry::start(resolve_json_static(&std_nodes("1.0.0", &sha)), gz);
        set_registry(&server.base);
        let path =
            crate::generate_stdlib_docs_json(&dir.join("out")).expect("docs generate on cold cache");
        assert_eq!(server.resolve_posts(), 1);
        assert!(path.is_file());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("hello"));
        assert!(frontend::stdlib_seed::missing_std_packages().is_empty());
    }

    #[test]
    fn warm_cache_generates_docs_without_network() {
        let (_guard, dir) = isolate("warm");
        let (gz, sha) = fixture_tarball("1.0.0");
        let first = MockRegistry::start(resolve_json_static(&std_nodes("1.0.0", &sha)), gz);
        set_registry(&first.base);
        let first_out =
            crate::generate_stdlib_docs_json(&dir.join("out-first")).expect("initial seed");
        assert_eq!(first.resolve_posts(), 1);
        let (gz2, sha2) = fixture_tarball("1.0.0");
        let nodes2 = std_nodes("1.0.0", &sha2);
        let second = MockRegistry::start(resolve_json_static(&nodes2), gz2);
        set_registry(&second.base);
        frontend::stdvfs::clear();
        let second_out =
            crate::generate_stdlib_docs_json(&dir.join("out-second")).expect("warm docs generate");
        assert_eq!(second.resolve_posts(), 0);
        let a = std::fs::read(first_out).unwrap();
        let b = std::fs::read(second_out).unwrap();
        assert_eq!(a, b);
    }
}
