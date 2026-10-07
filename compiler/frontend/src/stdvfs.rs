use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

static VFS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

pub const MAX_STD_MODULE_BYTES: usize = 1024 * 1024;

pub fn pin_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

pub fn normalize_rest(name: &str) -> String {
    let rest = name
        .strip_prefix("@std/")
        .or_else(|| name.strip_prefix("std/"))
        .unwrap_or(name);
    rest.to_string()
}

fn vfs() -> std::sync::MutexGuard<'static, BTreeMap<String, String>> {
    VFS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set(rest: &str, source: String) {
    vfs().insert(normalize_rest(rest), source);
}

pub fn get(rest: &str) -> Option<String> {
    vfs().get(&normalize_rest(rest)).cloned()
}

pub fn contains(rest: &str) -> bool {
    vfs().contains_key(&normalize_rest(rest))
}

pub fn list() -> Vec<String> {
    vfs().keys().cloned().collect()
}

pub fn clear() {
    vfs().clear();
}

pub fn is_known_module(rest: &str) -> bool {
    stdlib::MODULES.contains(&normalize_rest(rest).as_str())
}

pub fn std_source(rest: &str) -> Option<String> {
    let key = normalize_rest(rest);
    if let Some(src) = vfs().get(&key).cloned() {
        return Some(src);
    }
    if cfg!(target_arch = "wasm32") {
        return None;
    }
    cached_std_source(&key)
}

fn cached_std_source(key: &str) -> Option<String> {
    if !stdlib::MODULES.contains(&key) {
        return None;
    }
    let (pkg, sub) = match key.split_once('/') {
        Some((top, s)) => (format!("@std/{top}"), Some(s)),
        None => (format!("@std/{key}"), None),
    };
    if let Some(pin) = crate::stdlib_seed::read_std_pin() {
        if let Some(version) = pin.packages.get(&pkg) {
            let dir = crate::fetch::cached_package_dir(&pin.registry, &pkg, version);
            if let Some(src) = read_cached_module(&dir, sub) {
                return Some(src);
            }
        }
    }
    find_scanned_std_source(&pkg, sub)
}

fn read_cached_module(dir: &Path, sub: Option<&str>) -> Option<String> {
    let path = match sub {
        Some(s) => dir.join("src").join(format!("{s}.rnx")),
        None => {
            let lib = dir.join("src").join("lib.rnx");
            if lib.is_file() {
                lib
            } else {
                dir.join("src").join("main.rnx")
            }
        }
    };
    let bytes = std::fs::read(&path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_STD_MODULE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn find_scanned_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let root = crate::fetch::registry_cache_root();
    let hosts = std::fs::read_dir(&root).ok()?;
    let mut best: Option<(SemKey, PathBuf)> = None;
    for host in hosts.flatten() {
        let pkg_dir = host.path().join(pkg);
        let versions = match std::fs::read_dir(&pkg_dir) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for entry in versions.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            if !dir.join(crate::project::MANIFEST_FILE).is_file() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(next) = sem_key(&name) else {
                continue;
            };
            let replace = match &best {
                None => true,
                Some((cur, _)) => next > *cur,
            };
            if replace {
                best = Some((next, dir));
            }
        }
    }
    let (_, dir) = best?;
    read_cached_module(&dir, sub)
}

type SemKey = (u64, u64, u64, String);

fn sem_key(version: &str) -> Option<SemKey> {
    let (core, pre) = match version.split_once('-') {
        Some((c, p)) => (c, p.to_string()),
        None => (version, String::new()),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, pre))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::testkit;

    struct Guard {
        _cache: testkit::EnvGuard,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            clear();
        }
    }

    fn isolated(tag: &str) -> Guard {
        let (guard, _dir) = testkit::isolate_cache(tag);
        clear();
        Guard { _cache: guard }
    }

    #[test]
    fn vfs_round_trips_and_normalizes_prefixes() {
        let _g = isolated("vfsnorm");
        set("@std/fs", "export fn hello(): Int { return 1; }\n".to_string());
        assert_eq!(
            get("fs"),
            Some("export fn hello(): Int { return 1; }\n".to_string())
        );
        assert_eq!(
            get("std/fs"),
            Some("export fn hello(): Int { return 1; }\n".to_string())
        );
        assert!(contains("@std/fs"));
        assert_eq!(list(), vec!["fs".to_string()]);
    }

    #[test]
    fn std_source_prefers_vfs_over_cache() {
        let _g = isolated("vfspref");
        set("prelude", "vfs-bytes".to_string());
        assert_eq!(std_source("prelude"), Some("vfs-bytes".to_string()));
        assert_eq!(std_source("@std/prelude"), Some("vfs-bytes".to_string()));
    }

    #[test]
    fn std_source_misses_without_cache() {
        let _g = isolated("stdmiss");
        assert!(!contains("prelude"));
        if cfg!(target_arch = "wasm32") {
            assert_eq!(std_source("prelude"), None);
        } else {
            assert_eq!(std_source("prelude"), None);
            assert_eq!(std_source("@std/nope"), None);
        }
    }

    #[test]
    fn std_source_serves_seeded_cache() {
        let _g = isolated("stdseeded");
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let pin = env!("CARGO_PKG_VERSION");
        let (gz, sha) = testkit::fixture_tarball("@std/seed", pin);
        let nodes: Vec<String> = crate::stdlib_seed::std_package_names()
            .iter()
            .map(|full| testkit::node_json(full, pin, &sha, false))
            .collect();
        let server = testkit::MockRegistry::start(testkit::MockConfig {
            version_spec: 1,
            resolve_status: 200,
            resolve_body: testkit::resolve_json_static(&nodes),
            manifest_status: 200,
            fallback_tarball: gz,
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let cfg = testkit::registry_cfg(&server.base);
        let report = crate::stdlib_seed::seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        assert!(!report.packages.is_empty());
        let src = std_source("prelude").expect("seeded prelude serves from cache");
        assert!(src.contains("export fn hello"));
        let fs = std_source("@std/fs").expect("seeded fs serves from cache");
        assert!(fs.contains("export fn hello"));
    }

    #[test]
    fn known_modules_match_the_manifest() {
        assert!(is_known_module("prelude"));
        assert!(is_known_module("@std/net/http"));
        assert!(!is_known_module("nope"));
        assert!(!is_known_module(""));
    }

    #[test]
    fn pin_version_matches_frontend_package() {
        assert_eq!(pin_version(), env!("CARGO_PKG_VERSION"));
    }
}
