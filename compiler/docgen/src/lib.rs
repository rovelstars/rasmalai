#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

pub fn install_registry_transport() {
    frontend::fetch::install_http2_transport();
}

pub fn ensure_stdlib_seeded() -> Result<bool, diagnostics::Diagnostic> {
    install_registry_transport();
    if frontend::stdlib_seed::missing_std_packages().is_empty() {
        return Ok(false);
    }
    if frontend::stdlib_seed::vendor_covers_std() {
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
                    .is_some_and(|h| h.contains("rnx download"))
                {
                    return Err(err);
                }
                return Err(err.with_hint(
                    "check the registry URL and network, then retry `rnx download`",
                ));
            }
            Err(diagnostics::Diagnostic::new(
                diagnostics::Code::E108,
                format!(
                    "stdlib cache seeding from registry `{base}` failed: {}",
                    err.message
                ),
            )
            .with_hint("check the registry URL and network, then retry `rnx download`"))
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
    use std::collections::BTreeMap;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex, OnceLock};

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    struct EnvGuard {
        lock: Option<std::sync::MutexGuard<'static, ()>>,
        prev_cache: Option<String>,
        prev_registry: Option<String>,
        prev_vendor: Option<String>,
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
                match self.prev_vendor.clone() {
                    Some(v) => std::env::set_var(frontend::fetch::LOCAL_VENDOR_ENV, v),
                    None => std::env::remove_var(frontend::fetch::LOCAL_VENDOR_ENV),
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

    fn isolate(tag: &str) -> (EnvGuard, crate::scratch::Scratch) {
        let lock = ENV_LOCK
            .get_or_init(|| Mutex::new(()))
            .lock()
            .unwrap_or_else(|e| e.into_inner());
        let dir = crate::scratch::dir(&format!("docgen-{tag}"));
        let prev_cache = std::env::var("RNX_CACHE_HOME").ok();
        let prev_registry = std::env::var("RNX_REGISTRY").ok();
        let prev_vendor = std::env::var(frontend::fetch::LOCAL_VENDOR_ENV).ok();
        unsafe {
            std::env::set_var("RNX_CACHE_HOME", dir.path());
            std::env::remove_var("RNX_REGISTRY");
            std::env::remove_var(frontend::fetch::LOCAL_VENDOR_ENV);
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
            EnvGuard { lock: Some(lock), prev_cache, prev_registry, prev_vendor, prev_proxies },
            dir,
        )
    }

    fn set_registry(url: &str) {
        unsafe {
            std::env::set_var("RNX_REGISTRY", url);
        }
    }

    fn fixture_tree(full: &str, version: &str) -> (String, Vec<(String, Vec<u8>)>, String) {
        let original = format!(
            "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\"\n    }}\n}}\n"
        );
        let stored = original.replace(
            &format!("version: \"{version}\""),
            "version: \"0.0.0-published\"",
        );
        let files = vec![
            ("Project.config".to_string(), stored.into_bytes()),
            ("src/main.rnx".to_string(), b"export fn hello(): Int { return 1; }\n".to_vec()),
            ("src/http.rnx".to_string(), b"export fn hello(): Int { return 1; }\n".to_vec()),
        ];
        let mut entries = Vec::new();
        let mut pairs = Vec::new();
        for (name, bytes) in &files {
            let h = frontend::checksum::Sha256::hexdigest(bytes);
            entries.push(format!(
                "{{\"name\": \"{name}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{h}\"]}}",
                bytes.len()
            ));
            pairs.push((h, bytes.clone()));
        }
        let manifest = format!("{{\"entries\": [{}]}}", entries.join(", "));
        let digest = frontend::checksum::manifest_digest_hex(
            &files
                .iter()
                .map(|(name, bytes)| {
                    let h = frontend::checksum::Sha256::hexdigest(bytes);
                    frontend::checksum::DigestEntry {
                        name: name.clone(),
                        size: bytes.len() as u64,
                        dir: false,
                        chunks: vec![h],
                    }
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        (manifest, pairs, digest)
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

    struct TreeFixture {
        trees: BTreeMap<String, (String, Vec<(String, Vec<u8>)>)>,
    }

    fn fixture_for(names: &[String], version: &str) -> (Vec<String>, TreeFixture) {
        let mut trees = BTreeMap::new();
        let mut nodes = Vec::new();
        for full in names {
            let (manifest, pairs, digest) = fixture_tree(full, version);
            nodes.push(node_json(full, version, &digest));
            trees.insert(format!("{full}@{version}"), (manifest, pairs));
        }
        (nodes, TreeFixture { trees })
    }

    struct MockRegistry {
        base: String,
        log: Arc<Mutex<Vec<(String, String, Vec<u8>)>>>,
    }

    impl MockRegistry {
        fn start(resolve_body: String, fixture: TreeFixture) -> MockRegistry {
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
                    // Transport reads may carry `?origin-direct=1`; match on
                    // the path without query so salted and plain reads behave
                    // identically here.
                    let route = path.split('?').next().unwrap_or_default();
                    let with_spec: &[(&str, &str)] = &[("rnx-registry-spec", "0")];
                    if method == "GET" && route == "/api/version" {
                        let body =
                            "{\"spec\": 0, \"capabilities\": [], \"registry\": \"test\"}";
                        respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                    } else if method == "POST" && route == "/api/v0/resolve" {
                        respond(
                            &mut stream,
                            200,
                            "application/json",
                            resolve_body.as_bytes(),
                            with_spec,
                        );
                    } else if method == "GET" && route.ends_with("/chunks") {
                        let body = fixture
                            .trees
                            .iter()
                            .find(|(key, _)| route.contains(key.as_str()))
                            .map(|(_, (manifest, _))| manifest.as_bytes())
                            .unwrap_or_default();
                        respond(&mut stream, 200, "application/json", body, with_spec);
                    } else if method == "GET" && route.contains("/chunk/") {
                        let hash = route.rsplit('/').next().unwrap_or_default().to_string();
                        let hit = fixture
                            .trees
                            .values()
                            .flat_map(|(_, pairs)| pairs)
                            .find(|(h, _)| h == &hash)
                            .map(|(_, b)| b.clone());
                        match hit {
                            Some(bytes) => respond(
                                &mut stream,
                                200,
                                "application/octet-stream",
                                &bytes,
                                with_spec,
                            ),
                            None => respond(
                                &mut stream,
                                404,
                                "application/json",
                                b"{\"message\": \"chunk not found\"}",
                                with_spec,
                            ),
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
                .filter(|(m, p, _)| m == "POST" && p == "/api/v0/resolve")
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

    #[test]
    fn cold_cache_seeds_once_then_generates_docs() {
        let (_guard, dir) = isolate("cold");
        let names = frontend::stdlib_seed::std_package_names();
        let (nodes, fixture) = fixture_for(&names, "1.0.0");
        let server = MockRegistry::start(resolve_json_static(&nodes), fixture);
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
        let names = frontend::stdlib_seed::std_package_names();
        let (nodes, fixture) = fixture_for(&names, "1.0.0");
        let first = MockRegistry::start(resolve_json_static(&nodes), fixture);
        set_registry(&first.base);
        let first_out =
            crate::generate_stdlib_docs_json(&dir.join("out-first")).expect("initial seed");
        assert_eq!(first.resolve_posts(), 1);
        let (nodes2, fixture2) = fixture_for(&names, "1.0.0");
        let second = MockRegistry::start(resolve_json_static(&nodes2), fixture2);
        set_registry(&second.base);
        frontend::stdvfs::clear();
        let second_out =
            crate::generate_stdlib_docs_json(&dir.join("out-second")).expect("warm docs generate");
        assert_eq!(second.resolve_posts(), 0);
        let a = std::fs::read(first_out).unwrap();
        let b = std::fs::read(second_out).unwrap();
        assert_eq!(a, b);
    }

    #[test]
    fn vendored_stdlib_generates_docs_with_no_network() {
        let (_guard, dir) = isolate("vendored");
        let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src");
        let vendor = crate::scratch::dir("docgen-vendor");
        let staged = frontend::stdlib_seed::stage_std_checkout_tree(&src, vendor.path())
            .expect("stage checkout stdlib");
        assert_eq!(staged.len(), frontend::stdlib_seed::std_package_names().len());
        unsafe {
            std::env::set_var(frontend::fetch::LOCAL_VENDOR_ENV, vendor.path());
            std::env::set_var("RNX_REGISTRY", "http://127.0.0.1:9");
        }
        let path = crate::generate_stdlib_docs_json(&dir.join("out"))
            .expect("vendored stdlib docs generate with no network");
        assert!(path.is_file());
        let body = std::fs::read_to_string(&path).unwrap();
        assert!(body.contains("hello") || body.contains("fs"));
    }
}
