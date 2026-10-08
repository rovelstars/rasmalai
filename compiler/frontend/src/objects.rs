use std::path::{Path, PathBuf};

use super::cache::PROJECT_CACHE_DIR;

pub(crate) fn key_ok(key: &str) -> bool {
    !key.is_empty()
        && !key.starts_with('/')
        && !key.split('/').any(|c| c.is_empty() || c == "." || c == "..")
        && key
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '/' || c == '-' || c == '_' || c == '.')
}

pub fn object_path(root: &Path, key: &str) -> Option<PathBuf> {
    if !key_ok(key) {
        return None;
    }
    Some(root.join(PROJECT_CACHE_DIR).join("objects").join(format!("{key}.o")))
}

fn tmp_path(dest: &Path) -> PathBuf {
    dest.with_extension(format!("tmp-{}", std::process::id()))
}

// Missing or unreadable entry is a miss, never an error. Entries that are
// too short to hold an ELF header or lack the ELF magic are corrupt and
// also a miss.
pub fn read_object(root: &Path, key: &str) -> Option<Vec<u8>> {
    let path = object_path(root, key)?;
    let bytes = std::fs::read(&path).ok()?;
    if bytes.len() < 64 || bytes[0] != 0x7f || bytes[1] != b'E' || bytes[2] != b'L' || bytes[3] != b'F' {
        return None;
    }
    Some(bytes)
}

// Fail-open store: any I/O problem reports false and the caller falls back
// to the regular emit path. Writes go to a temp file plus rename so a crash
// never leaves a half-written entry behind.
pub fn write_object(root: &Path, key: &str, bytes: &[u8]) -> bool {
    let dest = match object_path(root, key) {
        Some(p) => p,
        None => return false,
    };
    if let Some(parent) = dest.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return false;
        }
    }
    let tmp = tmp_path(&dest);
    if std::fs::write(&tmp, bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, &dest).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

pub fn delete_object(root: &Path, key: &str) -> bool {
    match object_path(root, key) {
        Some(p) => std::fs::remove_file(p).is_ok(),
        None => false,
    }
}

fn visit_objects(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.file_type().is_dir() {
            visit_objects(&path, out);
        } else if meta.file_type().is_file() && path.extension().is_some_and(|e| e == "o") {
            out.push(path);
        }
    }
}

fn object_files(root: &Path) -> Vec<PathBuf> {
    let mut out = Vec::new();
    visit_objects(&root.join(PROJECT_CACHE_DIR).join("objects"), &mut out);
    out
}

pub fn objects_usage_bytes(root: &Path) -> u64 {
    object_files(root)
        .iter()
        .map(|p| std::fs::metadata(p).map(|m| m.len()).unwrap_or(0))
        .sum()
}

// Same oldest-first age policy as cache::prune_lru, applied to objects/ so
// both pools share one accounting. Never fails the build; I/O problems
// just stop eviction.
pub fn prune_objects_lru(root: &Path, keep_bytes: u64) -> u64 {
    let mut files: Vec<(u64, u64, PathBuf)> = Vec::new();
    for path in object_files(root) {
        let meta = match std::fs::metadata(&path) {
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
    freed
}

#[cfg(test)]
mod tests {
    use super::*;

    fn root(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rnx-objects-{}-{}-{}",
            std::process::id(),
            name,
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        dir
    }

    fn elf(payload: &[u8]) -> Vec<u8> {
        let mut v = vec![0x7f, b'E', b'L', b'F'];
        v.resize(64, 0);
        v.extend_from_slice(payload);
        v
    }

    #[test]
    fn roundtrip_and_delete() {
        let dir = root("roundtrip");
        let bytes = elf(b"object-bytes");
        assert!(write_object(&dir, "dev/ab12/cd34", &bytes));
        assert_eq!(read_object(&dir, "dev/ab12/cd34"), Some(bytes));
        assert!(delete_object(&dir, "dev/ab12/cd34"));
        assert_eq!(read_object(&dir, "dev/ab12/cd34"), None);
        assert!(!delete_object(&dir, "dev/ab12/cd34"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn missing_and_corrupt_are_misses() {
        let dir = root("corrupt");
        assert_eq!(read_object(&dir, "dev/aa/bb"), None);
        let dest = object_path(&dir, "dev/aa/bad").unwrap();
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"not an object").unwrap();
        assert_eq!(read_object(&dir, "dev/aa/bad"), None);
        std::fs::write(&dest, vec![0x7f, b'E', b'L', b'F']).unwrap();
        assert_eq!(read_object(&dir, "dev/aa/bad"), None);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn bad_keys_never_escape_cache_dir() {
        let dir = root("badkey");
        assert_eq!(object_path(&dir, "../evil"), None);
        assert_eq!(object_path(&dir, "/abs"), None);
        assert_eq!(object_path(&dir, ""), None);
        assert_eq!(read_object(&dir, "../evil"), None);
        assert!(!write_object(&dir, "a/../../evil", b"x"));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn usage_and_prune_cover_objects() {
        let dir = root("prune");
        assert!(write_object(&dir, "dev/m/one", &elf(b"12345678")));
        assert!(write_object(&dir, "dev/m/two", &elf(b"1234567890123456")));
        let usage = objects_usage_bytes(&dir);
        assert_eq!(usage, (64 + 8) + (64 + 16));
        assert_eq!(prune_objects_lru(&dir, u64::MAX), 0);
        assert_eq!(objects_usage_bytes(&dir), usage);
        let freed = prune_objects_lru(&dir, usage - 1);
        assert!(freed > 0);
        assert!(objects_usage_bytes(&dir) <= usage - 1);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
