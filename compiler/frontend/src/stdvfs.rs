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

fn vendor_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let anchor = std::env::current_dir()
        .map(|cwd| crate::fetch::cache_anchor(&cwd))
        .unwrap_or_else(|_| PathBuf::from("."));
    let version = crate::stdlib_seed::unpinned_std_version_for(Some(&anchor), pkg);
    let dir = crate::fetch::local_override_dir(pkg, &anchor)?;
    if !crate::fetch::check_local_override_silent(pkg, &dir, &version, Some(&version)) {
        return None;
    }
    let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
    crate::fetch::note_local_override(pkg, &version, &canonical);
    read_cached_module(&canonical, sub)
}

fn project_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let anchor = crate::fetch::cache_anchor(&cwd);
    let pin = crate::stdlib_seed::read_project_pin(&anchor)?;
    let version = pin.packages.get(pkg)?;
    let dir = crate::fetch::project_package_dir(&anchor, &pin.registry, pkg, version);
    if crate::fetch::cached_integrity(&dir).is_none() {
        return None;
    }
    read_cached_module(&dir, sub)
}

fn cached_std_source(key: &str) -> Option<String> {
    if !stdlib::MODULES.contains(&key) {
        return None;
    }
    let (pkg, sub) = match key.split_once('/') {
        Some((top, s)) => (format!("@std/{top}"), Some(s)),
        None => (format!("@std/{key}"), None),
    };
    if let Some(src) = vendor_std_source(&pkg, sub) {
        return Some(src);
    }
    if let Some(src) = project_std_source(&pkg, sub) {
        return Some(src);
    }
    if let Some(pin) = crate::stdlib_seed::read_std_pin()
        && let Some(version) = pin.packages.get(&pkg)
        && let Some(dir) = crate::fetch::find_cached_package_dir(&pin.registry, &pkg, version)
        && let Some(src) = read_cached_module(&dir, sub)
    {
        return Some(src);
    }
    if let Some(src) = find_scanned_project_std_source(&pkg, sub) {
        return Some(src);
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

fn find_scanned_project_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let anchor = crate::fetch::cache_anchor(&cwd);
    let root = crate::fetch::project_registry_root(&anchor);
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
            if crate::fetch::cached_integrity(&dir).is_none() {
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

fn find_scanned_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let mut best: Option<(SemKey, PathBuf)> = None;
    for have in crate::fetch::store_have() {
        if have.full != pkg {
            continue;
        }
        let Some(next) = sem_key(&have.version) else {
            continue;
        };
        let replace = match &best {
            None => true,
            Some((cur, _)) => next > *cur,
        };
        if replace && let Some(dir) = crate::fetch::store_pkg_dir(&have.integrity) {
            best = Some((next, dir));
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
        let mut chunks = BTreeMap::new();
        let mut nodes = Vec::new();
        for full in crate::stdlib_seed::std_package_names() {
            let (manifest, pairs, digest, _pc, _main) = testkit::fixture_tree(&full, pin);
            nodes.push(testkit::node_json(&full, pin, &digest, false));
            chunks.insert(format!("{full}@{pin}"), (manifest, pairs));
        }
        let body = testkit::resolve_json_static(&nodes);
        let server = testkit::MockRegistry::start_dynamic(chunks, move |_| body.clone());
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
