use crate::project::RegistryConfig;
use crate::{cache, fetch};
use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const STD_PIN_FILE: &str = "std-pin.json";
pub const STD_SEED_RANGE: &str = "*";
const STD_PIN_VERSION: u64 = 1;

pub fn std_pin_path() -> PathBuf {
    cache::global_cache_dir().join(STD_PIN_FILE)
}

pub fn std_registry_from_env() -> (Option<RegistryConfig>, BTreeMap<String, RegistryConfig>) {
    let url = std::env::var("RNX_REGISTRY")
        .ok()
        .filter(|s| !s.trim().is_empty());
    let default = url.map(|u| RegistryConfig {
        url: u,
        token_env: None,
        ca_cert: None,
    });
    (default, BTreeMap::new())
}

pub(crate) fn std_package_tops() -> Vec<String> {
    let mut tops: Vec<String> = stdlib::MODULES
        .iter()
        .map(|m| m.split('/').next().unwrap_or(m).to_string())
        .collect();
    tops.sort();
    tops.dedup();
    tops
}

pub(crate) fn std_top_modules() -> Vec<String> {
    std_package_tops()
}

pub fn std_package_names() -> Vec<String> {
    std_package_tops()
        .iter()
        .map(|m| format!("@std/{m}"))
        .collect()
}

pub fn std_requirements() -> BTreeMap<String, String> {
    std_package_tops()
        .iter()
        .map(|m| (format!("@std/{m}"), STD_SEED_RANGE.to_string()))
        .collect()
}

#[derive(Debug, Clone)]
pub struct StdSeedReport {
    pub packages: Vec<(String, String)>,
    pub pin_path: PathBuf,
    pub registry: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StdPin {
    pub date_secs: u64,
    pub registry: String,
    pub packages: BTreeMap<String, String>,
}

pub fn seed_stdlib_cache(
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
) -> Result<StdSeedReport, Diagnostic> {
    // Origin-direct: seeding must see one consistent generation
    // (resolve integrity + manifests + chunks); shared edge caches can
    // serve mixed generations with year-long TTLs.
    let _origin = fetch::origin_direct_guard();
    let requirements = std_requirements();
    let base = fetch::registry_base_for("@std/prelude", default_registry, overrides);
    let have = fetch::scan_cache_have();
    let fetched =
        fetch::ensure_registry_requirements(&requirements, default_registry, overrides, &have)
            .map_err(|e| seed_error(&base, e))?;
    let versions: BTreeMap<String, String> =
        fetched.into_iter().map(|f| (f.name, f.version)).collect();
    let mut packages: Vec<(String, String)> = Vec::with_capacity(requirements.len());
    for name in requirements.keys() {
        match versions.get(name) {
            Some(version) => packages.push((name.clone(), version.clone())),
            None => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("registry `{base}` resolved no version for package `{name}`"),
                )
                .with_hint("retry `rnx fetch-std` once the registry serves every @std/* package"));
            }
        }
    }
    packages.sort();
    let pin = StdPin {
        date_secs: now_secs(),
        registry: base.clone(),
        packages: packages.iter().cloned().collect(),
    };
    let path = std_pin_path();
    write_std_pin(&path, &pin)?;
    Ok(StdSeedReport {
        packages,
        pin_path: path,
        registry: base,
    })
}

fn seed_error(base: &str, err: Diagnostic) -> Diagnostic {
    if err.message.contains(base) {
        return err;
    }
    Diagnostic::new(
        Code::E108,
        format!(
            "stdlib cache seeding from registry `{base}` failed: {}",
            err.message
        ),
    )
    .with_hint("check the registry URL and network, then retry `rnx fetch-std`")
}

pub fn read_std_pin() -> Option<StdPin> {
    let body = std::fs::read(std_pin_path()).ok()?;
    let value = fetch::parse_json(&body).ok()?;
    let version = value.get("version").and_then(|v| v.as_u64())?;
    if version != STD_PIN_VERSION {
        return None;
    }
    let date_secs = value.get("date").and_then(|v| v.as_u64()).unwrap_or(0);
    let registry = value
        .get("registry")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let mut packages = BTreeMap::new();
    if let Some(map) = value.get("packages") {
        match map {
            fetch::JsonValue::Object(fields) => {
                for (k, v) in fields {
                    packages.insert(k.clone(), v.as_str().unwrap_or_default().to_string());
                }
            }
            _ => return None,
        }
    }
    if packages.is_empty() {
        return None;
    }
    Some(StdPin {
        date_secs,
        registry,
        packages,
    })
}

pub fn missing_std_packages() -> Vec<String> {
    let have = fetch::scan_cache_have();
    let mut missing = Vec::new();
    match read_std_pin() {
        Some(pin) => {
            for (full, version) in &pin.packages {
                if !have
                    .iter()
                    .any(|h| &h.full == full && &h.version == version)
                {
                    missing.push(format!("{full}@{version}"));
                }
            }
        }
        None => {
            for name in std_package_names() {
                if !have.iter().any(|h| h.full == name) {
                    missing.push(name);
                }
            }
        }
    }
    missing.sort();
    missing
}

pub fn unpinned_std_version(pkg: &str) -> String {
    read_std_pin()
        .and_then(|pin| pin.packages.get(pkg).cloned())
        .unwrap_or_else(|| env!("CARGO_PKG_VERSION").to_string())
}

pub fn vendor_covers_std() -> bool {
    let anchor = std::env::current_dir()
        .map(|cwd| fetch::cache_anchor(&cwd))
        .unwrap_or_else(|_| PathBuf::from("."));
    std_package_names().iter().all(|pkg| {
        let version = unpinned_std_version(pkg);
        match fetch::local_override_dir(pkg, &anchor) {
            Some(dir) => {
                fetch::check_local_override_silent(pkg, &dir, &version, Some(&version))
            }
            None => false,
        }
    })
}

fn std_stage_permissions(name: &str) -> Option<&'static str> {
    match name {
        "fs" => Some(
            "permissions: [{ perm: \"unsafe:raw_memory\", reason: \"mmap is backed by a raw address\" }]",
        ),
        "io" => Some(
            "permissions: [\"term:read\", \"term:write\", { perm: \"env:read:NO_COLOR\", reason: \"color detection reads NO_COLOR\" }, { perm: \"env:read:COLORTERM\", reason: \"color detection reads COLORTERM\" }, { perm: \"env:read:TERM\", reason: \"color detection reads TERM\" }]",
        ),
        "process" => Some(
            "permissions: [{ perm: \"env:read:*\", reason: \"flattenEnv reads caller-provided keys\" }]",
        ),
        _ => None,
    }
}

fn std_tree_package(top: &str) -> Result<(String, String, String, Vec<(String, Vec<u8>)>), Diagnostic> {
    let version = env!("CARGO_PKG_VERSION").to_string();
    let full = format!("@std/{top}");
    let main = stdlib::source(top)
        .ok_or_else(|| {
            Diagnostic::new(
                Code::E108,
                format!("toolchain carries no source for `{full}`"),
            )
        })?
        .as_bytes()
        .to_vec();
    let mut files = vec![("src/lib.rnx".to_string(), main)];
    if top == "net" {
        let http = stdlib::source("net/http")
            .ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("toolchain carries no source for `{full}/http`"),
                )
            })?
            .as_bytes()
            .to_vec();
        files.push(("src/http.rnx".to_string(), http));
    }
    let mut config = format!(
        "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\",\n        description: \"Standard library package {full}, staged from the toolchain sources.\"\n    }},\n    entries: {{\n        main: \"src/lib.rnx\"\n    }}"
    );
    if let Some(permissions) = std_stage_permissions(top) {
        config.push_str(",\n    ");
        config.push_str(permissions);
    }
    config.push_str("\n}\n");
    Ok((full, version, config, files))
}

pub fn stage_std_vendor_tree(out: &Path) -> Result<Vec<String>, Diagnostic> {
    let mut staged = Vec::new();
    for top in std_package_tops() {
        let (full, _, config, files) = std_tree_package(&top)?;
        let dir = out.join(&full);
        for (name, bytes) in &files {
            let path = dir.join(name);
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).map_err(|e| {
                    Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", parent.display()))
                })?;
            }
            std::fs::write(&path, bytes).map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", path.display()))
            })?;
        }
        std::fs::write(dir.join(crate::project::MANIFEST_FILE), config).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot write `{}`: {e}", dir.display()),
            )
        })?;
        staged.push(full);
    }
    staged.sort();
    Ok(staged)
}

fn write_cached_tree(base: &str, full: &str, version: &str, files: &[(String, Vec<u8>)]) -> Result<(), Diagnostic> {
    let mut digest_entries = Vec::with_capacity(files.len());
    for (name, bytes) in files {
        digest_entries.push(crate::checksum::DigestEntry {
            name: name.clone(),
            size: bytes.len() as u64,
            dir: false,
            chunks: vec![crate::checksum::Sha256::hexdigest(bytes)],
        });
    }
    let digest = crate::checksum::manifest_digest_hex(&digest_entries)?;
    let dest = fetch::cached_package_dir(base, full, version);
    let _ = std::fs::remove_dir_all(&dest);
    if let Some(parent) = dest.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot create `{}`: {e}", parent.display()),
            )
        })?;
    }
    for (name, bytes) in files {
        let path = dest.join(name);
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Diagnostic::new(
                    Code::E108,
                    format!("cannot write `{}`: {e}", path.display()),
                )
            })?;
        }
        std::fs::write(&path, bytes).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot write `{}`: {e}", path.display()),
            )
        })?;
    }
    std::fs::write(dest.join(".rnx-integrity"), format!("{full}\n{version}\n{digest}\n")).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("cannot write `{}`: {e}", dest.display()),
        )
    })
}

pub fn seed_embedded_std_cache() -> Result<StdSeedReport, Diagnostic> {
    let base = fetch::expand_registry_base("");
    let mut packages: Vec<(String, String)> = Vec::with_capacity(std_package_names().len());
    for top in std_package_tops() {
        let (full, version, config, mut files) = std_tree_package(&top)?;
        files.push((
            crate::project::MANIFEST_FILE.to_string(),
            config.into_bytes(),
        ));
        files.sort_by(|a, b| a.0.cmp(&b.0));
        write_cached_tree(&base, &full, &version, &files)?;
        packages.push((full, version));
    }
    packages.sort();
    let pin = StdPin {
        date_secs: now_secs(),
        registry: base.clone(),
        packages: packages.iter().cloned().collect(),
    };
    let path = std_pin_path();
    write_std_pin(&path, &pin)?;
    Ok(StdSeedReport {
        packages,
        pin_path: path,
        registry: base,
    })
}

fn write_std_pin(path: &Path, pin: &StdPin) -> Result<(), Diagnostic> {
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot create `{}`: {e}", parent.display()),
            )
        })?;
    }
    let mut out = String::from("{\"version\": 1, \"date\": ");
    out.push_str(&pin.date_secs.to_string());
    out.push_str(", \"registry\": \"");
    out.push_str(&fetch::json_escape(&pin.registry));
    out.push_str("\", \"packages\": {");
    for (i, (full, version)) in pin.packages.iter().enumerate() {
        if i > 0 {
            out.push_str(", ");
        }
        out.push_str(&format!(
            "\"{}\": \"{}\"",
            fetch::json_escape(full),
            fetch::json_escape(version)
        ));
    }
    out.push_str("}}");
    std::fs::write(path, out).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("cannot write `{}`: {e}", path.display()),
        )
        .with_hint("check that the global cache directory is writable, then retry `rnx fetch-std`")
    })
}

fn now_secs() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::testkit::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    struct GenerationServer {
        log: Arc<Mutex<Vec<(String, String, bool)>>>,
    }

    fn has_direct_salt(path: &str) -> bool {
        path.contains("origin-direct=1")
    }

    fn respond(stream: &mut std::net::TcpStream, status: u16, body: &[u8], json: bool) {
        let text = format!(
            "HTTP/1.1 {status} {}\r\nContent-Type: {}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            if status == 200 { "OK" } else { "Error" },
            if json { "application/json" } else { "application/octet-stream" },
            body.len()
        );
        let _ = stream.write_all(text.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    // Serves a fresh generation only to origin-direct reads
    // (`?origin-direct=1`) and a stale generation otherwise, reproducing
    // shared edge caches that hold manifests whose chunks no longer match
    // resolve integrity. Resolve always answers the fresh integrity. When
    // `always_stale` is set, even origin-direct reads get the stale
    // generation, reproducing a genuinely inconsistent registry.
    fn start_generation_server(always_stale: bool) -> (GenerationServer, RegistryConfig) {
        let (fresh_manifest, fresh_pairs, fresh_digest, _, _) = fixture_tree("@std/seed", "1.0.0");
        let (stale_manifest, stale_pairs, _stale_digest, _, _) = fixture_tree_with_main(
            "@std/seed",
            "0.0.9",
            b"export fn hello(): Int { return 0; }\n",
        );
        let resolve_body = resolve_json_static(&std_nodes("1.0.0", &fresh_digest));
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let log: Arc<Mutex<Vec<(String, String, bool)>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                loop {
                    match stream.read(&mut byte) {
                        Ok(0) => break,
                        Ok(_) => head.push(byte[0]),
                        Err(_) => break,
                    }
                    if head.ends_with(b"\r\n\r\n") || head.len() > 65536 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let mut lines = text.lines();
                let request = lines.next().unwrap_or_default().to_string();
                let mut parts = request.split_whitespace();
                let method = parts.next().unwrap_or_default().to_string();
                let path = parts.next().unwrap_or_default().to_string();
                let route = path.split('?').next().unwrap_or_default().to_string();
                let direct = has_direct_salt(&path) && !always_stale;
                seen.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push((method.clone(), path.clone(), direct));
                if method == "GET" && route == "/api/version" {
                    respond(&mut stream, 200, b"{\"spec\": 0}", true);
                } else if method == "POST" && route == "/api/v0/resolve" {
                    respond(&mut stream, 200, resolve_body.as_bytes(), true);
                } else if method == "GET" && route.ends_with("/chunks") {
                    let body = if direct { &fresh_manifest } else { &stale_manifest };
                    respond(&mut stream, 200, body.as_bytes(), true);
                } else if method == "GET" && route.contains("/chunk/") {
                    let hash = route.rsplit('/').next().unwrap_or_default();
                    let mut hit: Option<Vec<u8>> = None;
                    for pairs in [&fresh_pairs, &stale_pairs] {
                        for (h, b) in pairs {
                            if h == hash {
                                hit = Some(b.clone());
                                break;
                            }
                        }
                    }
                    match hit {
                        Some(bytes) => respond(&mut stream, 200, &bytes, false),
                        None => respond(&mut stream, 404, b"{}", true),
                    }
                } else {
                    respond(&mut stream, 404, b"{}", true);
                }
            }
        });
        let cfg = RegistryConfig { url: base.clone(), token_env: None, ca_cert: None };
        (GenerationServer { log }, cfg)
    }

    #[test]
    fn seed_reads_origin_direct_past_stale_edge() {
        let (_guard, _dir) = isolate_cache("stdorigin");
        let (server, cfg) = start_generation_server(false);
        let report = seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        assert_eq!(report.packages.len(), std_package_tops().len());
        assert!(report.packages.iter().all(|(_, v)| v == "1.0.0"));
        let logged = server.log.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let manifests: Vec<_> = logged
            .iter()
            .filter(|(m, p, _)| m == "GET" && p.contains("/chunks"))
            .collect();
        assert!(!manifests.is_empty());
        assert!(manifests.iter().all(|(_, p, _)| p.contains("origin-direct=1")));
        assert!(missing_std_packages().is_empty());
    }

    #[test]
    fn stale_edge_heals_through_guarded_retry() {
        let (_guard, _dir) = isolate_cache("stdretry");
        let (server, cfg) = start_generation_server(false);
        let fetched = fetch::ensure_registry_requirements(
            &std_requirements(),
            Some(&cfg),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), std_package_tops().len());
        let logged = server.log.lock().unwrap_or_else(|e| e.into_inner()).clone();
        let manifests: Vec<_> = logged
            .iter()
            .filter(|(m, p, _)| m == "GET" && p.contains("/chunks"))
            .collect();
        assert!(manifests.iter().any(|(_, p, _)| !p.contains("origin-direct=1")));
        assert!(manifests.iter().any(|(_, p, _)| p.contains("origin-direct=1")));
    }

    #[test]
    fn persistently_stale_registry_still_fails_closed() {
        let (_guard, _dir) = isolate_cache("stdstale");
        let (_server, cfg) = start_generation_server(true);
        let err = fetch::ensure_registry_requirements(
            &std_requirements(),
            Some(&cfg),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(
            err.message.contains("checksum mismatch") || err.message.contains("is missing"),
            "{}",
            err.message
        );
    }

    fn std_nodes(version: &str, integrity: &str) -> Vec<String> {
        std_package_names()
            .iter()
            .map(|full| node_json(full, version, integrity, false))
            .collect()
    }

    #[test]
    fn local_seed_populates_cache_for_offline_pinned_resolves() {
        let (_guard, _dir) = isolate_cache("stdlocal");
        let base = fetch::expand_registry_base("");
        let report = seed_embedded_std_cache().unwrap();
        assert_eq!(report.packages.len(), std_package_tops().len());
        assert_eq!(report.registry, base);
        assert_eq!(report.pin_path, std_pin_path());
        assert!(missing_std_packages().is_empty());
        let cli = env!("CARGO_PKG_VERSION");
        for full in std_package_names() {
            let dir = fetch::cached_package_dir(&base, &full, cli);
            assert!(dir.join(crate::project::MANIFEST_FILE).is_file(), "{}", dir.display());
            let integrity = std::fs::read_to_string(dir.join(".rnx-integrity")).unwrap();
            let mut lines = integrity.lines();
            assert_eq!(lines.next(), Some(full.as_str()));
            assert_eq!(lines.next(), Some(cli));
            assert!(lines.next().is_some_and(|s| s.len() == 64));
            let (found, _, version) = fetch::resolve_registry_package(
                &full,
                "*",
                None,
                &BTreeMap::new(),
                Some(cli.to_string()),
                &[],
                None,
            )
            .expect("locally seeded cache serves pinned resolves with no network");
            assert_eq!(version, cli);
            assert_eq!(found, std::fs::canonicalize(&dir).unwrap());
        }
    }

    #[test]
    fn stage_writes_a_resolvable_vendor_tree() {
        let (_guard, _dir) = isolate_cache("stdstage");
        let out = crate::scratch::dir("stdstage-out");
        let staged = stage_std_vendor_tree(out.path()).unwrap();
        assert_eq!(staged, std_package_names());
        let cli = env!("CARGO_PKG_VERSION");
        for full in &staged {
            let dir = out.path().join(full);
            let cfg = crate::project::ProjectConfig::load_from_dir(&dir)
                .unwrap()
                .expect("staged package carries a manifest");
            assert_eq!(&cfg.name, full);
            assert_eq!(cfg.version, cli);
            let top = full.strip_prefix("@std/").unwrap();
            let want = stdlib::source(top).expect("embedded source exists");
            let main = std::fs::read(dir.join("src").join("lib.rnx")).unwrap();
            assert_eq!(main, want.as_bytes());
        }
        let net_http = std::fs::read(out.path().join("@std/net").join("src").join("http.rnx")).unwrap();
        assert_eq!(net_http, stdlib::source("net/http").unwrap().as_bytes());
        let _vendor = set_vendor_dir(out.path());
        assert!(vendor_covers_std());
        let dead = RegistryConfig { url: "http://127.0.0.1:9".to_string(), token_env: None, ca_cert: None };
        let anchor = crate::scratch::dir("stdstage-anchor");
        let (dir, _, version) = fetch::resolve_registry_package(
            "@std/fs",
            "*",
            Some(&dead),
            &BTreeMap::new(),
            None,
            &[],
            Some(anchor.path()),
        )
        .expect("staged stdlib resolves with no network");
        assert_eq!(version, cli);
        assert_eq!(dir, std::fs::canonicalize(out.path().join("@std/fs")).unwrap());
        assert!(fetch::local_override_used("@std/fs"));
    }

    fn start_std_registry(version: &str) -> (MockRegistry, String, RegistryConfig) {
        let (manifest, pairs, digest, _, _) = fixture_tree("@std/seed", version);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&std_nodes(version, &digest)),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(pairs),
        });
        let cfg = RegistryConfig {
            url: server.base.clone(),
            token_env: None,
            ca_cert: None,
        };
        (server, digest, cfg)
    }

    #[test]
    fn requirements_cover_every_top_level_package_at_star() {
        let reqs = std_requirements();
        assert_eq!(reqs.len(), std_package_tops().len());
        for m in std_package_tops() {
            assert_eq!(
                reqs.get(&format!("@std/{m}")).map(|s| s.as_str()),
                Some("*")
            );
        }
        assert!(!reqs.contains_key("@std/net/http"));
    }

    #[test]
    fn seed_populates_cache_and_writes_pin_in_one_round_trip() {
        let (_guard, _dir) = isolate_cache("stdseed");
        let (server, sha, cfg) = start_std_registry("1.0.0");
        let report = seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        assert_eq!(report.packages.len(), std_package_tops().len());
        assert!(report.packages.iter().all(|(_, v)| v == "1.0.0"));
        assert_eq!(report.registry, server.base);
        assert_eq!(report.pin_path, std_pin_path());
        assert_eq!(server.resolve_bodies().len(), 1);
        let body = &server.resolve_bodies()[0];
        for name in std_package_names() {
            assert_eq!(requirement(body, &name), "*");
            let dir = fetch::cached_package_dir(&report.registry, &name, "1.0.0");
            assert!(
                dir.join(crate::project::MANIFEST_FILE).is_file(),
                "{}",
                dir.display()
            );
        }
        let pin = read_std_pin().expect("pin file written");
        assert_eq!(pin.packages.len(), std_package_tops().len());
        assert_eq!(pin.registry, server.base);
        assert!(pin.date_secs > 0);
        for name in std_package_names() {
            assert_eq!(pin.packages.get(&name).map(|s| s.as_str()), Some("1.0.0"));
        }
        let have = fetch::scan_cache_have();
        assert_eq!(have.len(), std_package_tops().len());
        assert!(have.iter().all(|h| h.integrity == sha));
        assert!(missing_std_packages().is_empty());
    }

    #[test]
    fn reseed_restores_a_deleted_package() {
        let (_guard, _dir) = isolate_cache("stdrepair");
        let (_server, _sha, cfg) = start_std_registry("1.0.0");
        let report = seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        let victim = fetch::cached_package_dir(&report.registry, "@std/fs", "1.0.0");
        assert!(victim.is_dir());
        std::fs::remove_dir_all(&victim).unwrap();
        assert_eq!(missing_std_packages(), vec!["@std/fs@1.0.0".to_string()]);
        let repaired = seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        assert!(victim.join(crate::project::MANIFEST_FILE).is_file());
        assert!(
            repaired
                .packages
                .contains(&("@std/fs".to_string(), "1.0.0".to_string()))
        );
        assert!(missing_std_packages().is_empty());
    }

    #[test]
    fn offline_seed_fails_loudly_without_pin() {
        let (_guard, _dir) = isolate_cache("stdoffline");
        let dead = RegistryConfig {
            url: "http://127.0.0.1:9".to_string(),
            token_env: None,
            ca_cert: None,
        };
        let err = seed_stdlib_cache(Some(&dead), &BTreeMap::new()).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("127.0.0.1:9"), "{}", err.message);
        assert!(!std_pin_path().exists());
        assert_eq!(read_std_pin(), None);
    }
}
