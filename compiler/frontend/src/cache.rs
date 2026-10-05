use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const PROJECT_CACHE_DIR: &str = ".rnx-cache";
pub const DEFAULT_RETENTION_BYTES: u64 = 2 * 1024 * 1024 * 1024;

// Must track llvm/Cargo.toml (inkwell llvm22-1 today); a stale value reuses wrong artifacts.
pub const LLVM_VERSION: &str = "llvm22";

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

pub fn stdlib_sources_digest() -> Vec<u8> {
    let mut hasher = Sha256::new();
    for name in stdlib::MODULES {
        let src = stdlib::source(name).unwrap_or("");
        hasher.update(&(name.len() as u64).to_le_bytes());
        hasher.update(name.as_bytes());
        hasher.update(&(src.len() as u64).to_le_bytes());
        hasher.update(src.as_bytes());
    }
    hasher.finalize().to_vec()
}

pub fn toolchain_parts(
    release: bool,
    opt_level: u8,
    target_triple: Option<&str>,
    host_triple: &str,
    debug: bool,
    rnx_version: &str,
    runtime_hash: &str,
) -> Vec<Vec<u8>> {
    vec![
        format!(
            "release={release} opt={opt_level} debug={debug} target={}",
            target_triple.unwrap_or(host_triple)
        )
        .into_bytes(),
        rnx_version.as_bytes().to_vec(),
        LLVM_VERSION.as_bytes().to_vec(),
        format!("rt-{runtime_hash}").into_bytes(),
        stdlib_sources_digest(),
    ]
}

fn collect_rnx(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.flatten() {
        let path = entry.path();
        if path.is_dir() {
            collect_rnx(&path, out);
        } else if path.extension().is_some_and(|e| e == "rnx") {
            out.push(path);
        }
    }
}

pub fn hash_src_tree(root: &Path) -> Vec<u8> {
    let mut found = Vec::new();
    collect_rnx(&root.join("src"), &mut found);
    let mut rels: Vec<(String, Vec<u8>)> = Vec::new();
    for path in found {
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        rels.push((rel, std::fs::read(&path).unwrap_or_default()));
    }
    rels.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    for (rel, bytes) in &rels {
        let rel_bytes = rel.as_bytes();
        hasher.update(&(rel_bytes.len() as u64).to_le_bytes());
        hasher.update(rel_bytes);
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    hasher.finalize().to_vec()
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
    fn toolchain_parts_track_profile_and_toolchain() {
        let base = toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt");
        assert_eq!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt")
        );
        assert_ne!(
            base,
            toolchain_parts(true, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 0, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", true, "0.1.0", "rt")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "aarch64-unknown-linux-gnu", false, "0.1.0", "rt")
        );
        assert_ne!(
            base,
            toolchain_parts(
                false,
                1,
                Some("aarch64-unknown-linux-gnu"),
                "x86_64-unknown-linux-gnu",
                false,
                "0.1.0",
                "rt"
            )
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.1", "rt")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "other")
        );
        assert_eq!(base.last().cloned().unwrap_or_default(), stdlib_sources_digest());
    }

    #[test]
    fn stdlib_sources_digest_is_stable() {
        let first = stdlib_sources_digest();
        assert_eq!(first.len(), 32);
        assert_eq!(stdlib_sources_digest(), first);
    }

    #[test]
    fn src_tree_hash_covers_sources_only() {
        let dir = std::env::temp_dir().join(format!("rnx-src-tree-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        let first = hash_src_tree(&dir);
        assert_eq!(first.len(), 32);
        assert_eq!(hash_src_tree(&dir), first);
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
        std::fs::write(dir.join("src").join("notes.txt"), "ignored").unwrap();
        assert_eq!(hash_src_tree(&dir), first);
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 1; }\n").unwrap();
        assert_ne!(hash_src_tree(&dir), first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn prune_and_clean_roundtrip() {
        let dir = std::env::temp_dir().join(format!("rnx-cache-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(dir.join("aaa.bin"), b"hello world").unwrap();
        assert_eq!(cache_usage_bytes(&dir), 11);
        assert_eq!(prune_lru(&dir, 100).unwrap(), 0);
        assert_eq!(cache_usage_bytes(&dir), 11);
        assert_eq!(prune_lru(&dir, 6).unwrap(), 11);
        assert_eq!(cache_usage_bytes(&dir), 0);
        std::fs::write(dir.join("ccc.bin"), b"hello").unwrap();
        assert_eq!(clean_dir(&dir).unwrap(), 5);
        assert_eq!(cache_usage_bytes(&dir), 0);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
