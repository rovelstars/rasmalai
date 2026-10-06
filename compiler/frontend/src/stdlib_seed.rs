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

pub fn std_package_names() -> Vec<String> {
    stdlib::MODULES
        .iter()
        .map(|m| format!("@std/{m}"))
        .collect()
}

pub fn std_requirements() -> BTreeMap<String, String> {
    stdlib::MODULES
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

    fn std_nodes(version: &str, integrity: &str) -> Vec<String> {
        std_package_names()
            .iter()
            .map(|full| node_json(full, version, integrity, false))
            .collect()
    }

    fn start_std_registry(version: &str) -> (MockRegistry, String, RegistryConfig) {
        let (gz, sha) = fixture_tarball("@std/seed", version);
        let server = MockRegistry::start(MockConfig {
            version_spec: 1,
            resolve_status: 200,
            resolve_body: resolve_json_static(&std_nodes(version, &sha)),
            manifest_status: 200,
            fallback_tarball: gz,
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let cfg = RegistryConfig {
            url: server.base.clone(),
            token_env: None,
            ca_cert: None,
        };
        (server, sha, cfg)
    }

    #[test]
    fn requirements_cover_every_module_at_star() {
        let reqs = std_requirements();
        assert_eq!(reqs.len(), stdlib::MODULES.len());
        for m in stdlib::MODULES {
            assert_eq!(
                reqs.get(&format!("@std/{m}")).map(|s| s.as_str()),
                Some("*")
            );
        }
    }

    #[test]
    fn seed_populates_cache_and_writes_pin_in_one_round_trip() {
        let (_guard, _dir) = isolate_cache("stdseed");
        let (server, sha, cfg) = start_std_registry("1.0.0");
        let report = seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        assert_eq!(report.packages.len(), stdlib::MODULES.len());
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
        assert_eq!(pin.packages.len(), stdlib::MODULES.len());
        assert_eq!(pin.registry, server.base);
        assert!(pin.date_secs > 0);
        for name in std_package_names() {
            assert_eq!(pin.packages.get(&name).map(|s| s.as_str()), Some("1.0.0"));
        }
        let have = fetch::scan_cache_have();
        assert_eq!(have.len(), stdlib::MODULES.len());
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
