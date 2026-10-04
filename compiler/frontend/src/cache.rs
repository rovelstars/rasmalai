use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const PROJECT_CACHE_DIR: &str = ".rnx-cache";
pub const DEFAULT_RETENTION_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn global_cache_dir() -> PathBuf {
    if let Ok(dir) = std::env::var("RNX_CACHE_HOME") {
        if !dir.is_empty() {
            return PathBuf::from(dir);
        }
    }
    #[cfg(windows)]
    {
        if let Ok(local) = std::env::var("LOCALAPPDATA") {
            if !local.is_empty() {
                return PathBuf::from(local).join("rnx");
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(xdg) = std::env::var("XDG_CACHE_HOME") {
            if !xdg.is_empty() {
                return PathBuf::from(xdg).join("rnx");
            }
        }
    }
    home_dir().join(".cache").join("rnx")
}

fn home_dir() -> PathBuf {
    #[cfg(windows)]
    {
        if let Ok(profile) = std::env::var("USERPROFILE") {
            if !profile.is_empty() {
                return PathBuf::from(profile);
            }
        }
    }
    #[cfg(not(windows))]
    {
        if let Ok(home) = std::env::var("HOME") {
            if !home.is_empty() {
                return PathBuf::from(home);
            }
        }
    }
    PathBuf::from(".")
}

pub fn project_cache_dir(root: &Path) -> PathBuf {
    root.join(PROJECT_CACHE_DIR)
}

pub fn fingerprint_hex(parts: &[&[u8]]) -> String {
    let mut hasher = Sha256::new();
    for part in parts {
        let len = (part.len() as u64).to_le_bytes();
        hasher.update(len);
        hasher.update(part);
    }
    hex_encode(&hasher.finalize())
}

fn hex_encode(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
    }
    out
}

pub fn stdlib_sources() -> Vec<(&'static str, &'static str)> {
    stdlib::MODULES
        .iter()
        .filter_map(|name| stdlib::source(name).map(|src| (*name, src)))
        .collect()
}

pub fn stdlib_fingerprint(
    rnx_version: &str,
    llvm_version: &str,
    opt_profile: &str,
    target_triple: &str,
    runtime_hash: &str,
) -> String {
    let mut owned: Vec<String> = Vec::new();
    for (name, src) in stdlib_sources() {
        owned.push(name.to_string());
        owned.push(src.to_string());
    }
    owned.push(rnx_version.to_string());
    owned.push(llvm_version.to_string());
    owned.push(opt_profile.to_string());
    owned.push(target_triple.to_string());
    owned.push(runtime_hash.to_string());
    let parts: Vec<&[u8]> = owned.iter().map(|s| s.as_bytes()).collect();
    fingerprint_hex(&parts)
}

pub fn dep_fingerprint(
    tarball_bytes: &[u8],
    version: &str,
    pinned_commit: &str,
    transitive: &BTreeMap<String, String>,
    toolchain: &[&str],
) -> String {
    let mut parts: Vec<&[u8]> = vec![tarball_bytes, version.as_bytes(), pinned_commit.as_bytes()];
    let mut keys: Vec<&String> = transitive.keys().collect();
    keys.sort();
    let mut owned: Vec<String> = Vec::new();
    for k in keys {
        owned.push(k.clone());
        owned.push(transitive[k].clone());
    }
    for s in &owned {
        parts.push(s.as_bytes());
    }
    for t in toolchain {
        parts.push(t.as_bytes());
    }
    fingerprint_hex(&parts)
}

pub fn user_fingerprint(
    local_bytes: &[u8],
    direct_deps: &BTreeMap<String, String>,
    profile_flags: &str,
    toolchain: &[&str],
) -> String {
    let mut parts: Vec<&[u8]> = vec![local_bytes, profile_flags.as_bytes()];
    let mut keys: Vec<&String> = direct_deps.keys().collect();
    keys.sort();
    let mut owned: Vec<String> = Vec::new();
    for k in keys {
        owned.push(k.clone());
        owned.push(direct_deps[k].clone());
    }
    for s in &owned {
        parts.push(s.as_bytes());
    }
    for t in toolchain {
        parts.push(t.as_bytes());
    }
    fingerprint_hex(&parts)
}

fn artifact_path(dir: &Path, key: &str) -> PathBuf {
    dir.join(format!("{key}.bin"))
}

pub fn store_artifact(dir: &Path, key: &str, bytes: &[u8]) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    let tmp = dir.join(format!(".{key}.tmp"));
    std::fs::write(&tmp, bytes)?;
    std::fs::rename(&tmp, artifact_path(dir, key))?;
    Ok(())
}

pub fn load_artifact(dir: &Path, key: &str) -> Option<Vec<u8>> {
    std::fs::read(artifact_path(dir, key)).ok()
}

pub fn cache_usage_bytes(dir: &Path) -> u64 {
    let mut total = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return 0,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_file() && path.extension().is_some_and(|e| e == "bin") {
            total += entry.metadata().map(|m| m.len()).unwrap_or(0);
        }
    }
    total
}

pub fn prune_lru(dir: &Path, keep_bytes: u64) -> std::io::Result<u64> {
    let mut files: Vec<(u64, u64, PathBuf)> = Vec::new();
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(0),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() || !path.extension().is_some_and(|e| e == "bin") {
            continue;
        }
        let meta = match entry.metadata() {
            Ok(m) => m,
            Err(_) => continue,
        };
        let age = meta
            .accessed()
            .or_else(|_| meta.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .map(|d| d.as_secs())
            .unwrap_or(0);
        files.push((age, meta.len(), path));
    }
    files.sort_by(|a, b| b.0.cmp(&a.0));
    let mut total: u64 = files.iter().map(|(_, len, _)| len).sum();
    let mut freed = 0;
    for (_, len, path) in files {
        if total <= keep_bytes {
            break;
        }
        if std::fs::remove_file(&path).is_ok() {
            total -= len;
            freed += len;
        }
    }
    Ok(freed)
}

pub fn clean_dir(dir: &Path) -> std::io::Result<u64> {
    let mut freed = 0;
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return Ok(0),
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }
        let len = entry.metadata().map(|m| m.len()).unwrap_or(0);
        if std::fs::remove_file(&path).is_ok() {
            freed += len;
        }
    }
    Ok(freed)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fingerprint_is_stable_and_length_prefixed() {
        let a = fingerprint_hex(&[b"ab", b"c"]);
        let b = fingerprint_hex(&[b"a", b"bc"]);
        assert_eq!(a.len(), 64);
        assert_ne!(a, b);
        assert_eq!(a, fingerprint_hex(&[b"ab", b"c"]));
    }

    #[test]
    fn dep_fingerprint_ignores_map_order() {
        let mut m1 = BTreeMap::new();
        m1.insert("b".to_string(), "2".to_string());
        m1.insert("a".to_string(), "1".to_string());
        let mut m2 = BTreeMap::new();
        m2.insert("a".to_string(), "1".to_string());
        m2.insert("b".to_string(), "2".to_string());
        assert_eq!(
            dep_fingerprint(b"tar", "1.0.0", "", &m1, &["llvm22"]),
            dep_fingerprint(b"tar", "1.0.0", "", &m2, &["llvm22"])
        );
    }

    #[test]
    fn store_load_prune_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rnx-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        store_artifact(&dir, "aaa", b"hello").unwrap();
        store_artifact(&dir, "bbb", b"world!").unwrap();
        assert_eq!(load_artifact(&dir, "aaa").unwrap(), b"hello");
        assert_eq!(cache_usage_bytes(&dir), 11);
        let freed = prune_lru(&dir, 6).unwrap();
        assert_eq!(freed, 6);
        assert_eq!(cache_usage_bytes(&dir), 5);
        let cleaned = clean_dir(&dir).unwrap();
        assert_eq!(cleaned, 5);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn stdlib_fingerprint_covers_embedded_sources() {
        let f = stdlib_fingerprint("0.1.0", "llvm22", "dev", "x86_64-linux", "rt");
        assert_eq!(f.len(), 64);
        assert_ne!(f, stdlib_fingerprint("0.1.1", "llvm22", "dev", "x86_64-linux", "rt"));
    }
}
