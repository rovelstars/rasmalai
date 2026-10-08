use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

pub const PROJECT_CACHE_DIR: &str = ".rnx-cache";
pub const DEFAULT_RETENTION_BYTES: u64 = 2 * 1024 * 1024 * 1024;

pub fn filesystem_free_bytes(dir: &Path) -> Option<u64> {
    #[cfg(unix)]
    {
        let anchor = if dir.is_file() {
            dir.parent().unwrap_or_else(|| Path::new("."))
        } else {
            dir
        };
        let cstr = std::ffi::CString::new(anchor.as_os_str().as_encoded_bytes()).ok()?;
        let mut stat: libc::statvfs = unsafe { std::mem::zeroed() };
        if unsafe { libc::statvfs(cstr.as_ptr(), &mut stat) } != 0 {
            return None;
        }
        (stat.f_bavail as u64).checked_mul(stat.f_frsize as u64)
    }
    #[cfg(not(unix))]
    {
        let _ = dir;
        None
    }
}

pub fn effective_cap(dir: &Path, requested: Option<u64>) -> u64 {
    let base = requested.unwrap_or(DEFAULT_RETENTION_BYTES);
    match filesystem_free_bytes(dir) {
        Some(free) => base.min(free / 10).max(64 * 1024 * 1024),
        None => base,
    }
}

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
    match crate::stdlib_seed::read_std_pin() {
        Some(pin) => {
            hasher.update(&(pin.registry.len() as u64).to_le_bytes());
            hasher.update(pin.registry.as_bytes());
            for (full, version) in &pin.packages {
                hasher.update(&(full.len() as u64).to_le_bytes());
                hasher.update(full.as_bytes());
                hasher.update(&(version.len() as u64).to_le_bytes());
                hasher.update(version.as_bytes());
                let dir = crate::fetch::cached_package_dir(&pin.registry, full, version);
                hash_cached_package(&mut hasher, &dir);
            }
        }
        None => {
            for name in stdlib::MODULES {
                hasher.update(&(name.len() as u64).to_le_bytes());
                hasher.update(name.as_bytes());
            }
        }
    }
    hasher.finalize().to_vec()
}

fn hash_cached_package(hasher: &mut Sha256, dir: &Path) {
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_cached_files(dir, dir, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    hasher.update(&(files.len() as u64).to_le_bytes());
    for (rel, bytes) in &files {
        hasher.update(&(rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
}

fn collect_cached_files(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
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
            collect_cached_files(root, &path, out);
        } else if meta.file_type().is_file() {
            let rel = match path.strip_prefix(root) {
                Ok(r) => r.to_string_lossy().replace('\\', "/"),
                Err(_) => continue,
            };
            out.push((rel, std::fs::read(&path).unwrap_or_default()));
        }
    }
}

pub fn toolchain_parts(
    release: bool,
    opt_level: u8,
    target_triple: Option<&str>,
    host_triple: &str,
    debug: bool,
    rnx_version: &str,
    runtime_hash: &str,
    llvm_version: &str,
) -> Vec<Vec<u8>> {
    vec![
        format!(
            "release={release} opt={opt_level} debug={debug} target={}",
            target_triple.unwrap_or(host_triple)
        )
        .into_bytes(),
        rnx_version.as_bytes().to_vec(),
        llvm_version.as_bytes().to_vec(),
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
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.file_type().is_dir() {
            let skip = path
                .file_name()
                .is_some_and(|n| n == "target" || n == ".git" || n == ".rnx-cache");
            if !skip {
                collect_rnx(&path, out);
            }
        } else if meta.file_type().is_file() && path.extension().is_some_and(|e| e == "rnx") {
            out.push(path);
        }
    }
}

pub fn hash_src_tree_files(root: &Path) -> (Vec<u8>, Vec<(String, Vec<u8>)>) {
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
    let mut files = Vec::with_capacity(rels.len());
    let mut hasher = Sha256::new();
    for (rel, bytes) in &rels {
        let rel_bytes = rel.as_bytes();
        hasher.update(&(rel_bytes.len() as u64).to_le_bytes());
        hasher.update(rel_bytes);
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
        let mut entry = Sha256::new();
        entry.update(&(rel_bytes.len() as u64).to_le_bytes());
        entry.update(rel_bytes);
        entry.update(&(bytes.len() as u64).to_le_bytes());
        entry.update(bytes);
        files.push((rel.clone(), entry.finalize().to_vec()));
    }
    (hasher.finalize().to_vec(), files)
}

pub fn hash_src_tree(root: &Path) -> Vec<u8> {
    hash_src_tree_files(root).0
}

pub fn project_deps_dir(root: &Path) -> PathBuf {
    project_cache_dir(root).join("deps")
}

fn walk_bin_files(dir: &Path, out: &mut Vec<PathBuf>) {
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
            walk_bin_files(&path, out);
        } else if meta.file_type().is_file() && path.extension().is_some_and(|e| e == "bin") {
            out.push(path);
        }
    }
}

pub fn cache_usage_bytes(dir: &Path) -> u64 {
    let mut files = Vec::new();
    walk_bin_files(dir, &mut files);
    let mut total = 0;
    for path in files {
        total += std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    }
    total
}

pub fn prune_lru(dir: &Path, keep_bytes: u64) -> std::io::Result<u64> {
    let mut paths = Vec::new();
    walk_bin_files(dir, &mut paths);
    let mut files: Vec<(u64, u64, PathBuf)> = Vec::new();
    for path in paths {
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
    files.sort_by(|a, b| b.0.cmp(&a.0).then_with(|| a.2.cmp(&b.2)));
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
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("toolparts");
        let base = toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt", "llvm22");
        assert_eq!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt", "llvm22")
        );
        assert_ne!(
            base,
            toolchain_parts(true, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt", "llvm22")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 0, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt", "llvm22")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", true, "0.1.0", "rt", "llvm22")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "aarch64-unknown-linux-gnu", false, "0.1.0", "rt", "llvm22")
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
                "rt",
                "llvm22"
            )
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.1", "rt", "llvm22")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "other", "llvm22")
        );
        assert_ne!(
            base,
            toolchain_parts(false, 1, None, "x86_64-unknown-linux-gnu", false, "0.1.0", "rt", "llvm23")
        );
        assert_eq!(base.last().cloned().unwrap_or_default(), stdlib_sources_digest());
    }

    #[test]
    fn stdlib_sources_digest_is_stable() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("digeststable");
        let first = stdlib_sources_digest();
        assert_eq!(first.len(), 32);
        assert_eq!(stdlib_sources_digest(), first);
    }

    #[test]
    fn stdlib_digest_tracks_pin_and_cache_bytes() {
        use crate::fetch::testkit;
        use std::collections::BTreeMap;

        let (_guard, _dir) = testkit::isolate_cache("digestpin");
        let empty = stdlib_sources_digest();
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
        let report =
            crate::stdlib_seed::seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        let seeded = stdlib_sources_digest();
        assert_eq!(seeded.len(), 32);
        assert_ne!(empty, seeded);
        assert_eq!(stdlib_sources_digest(), seeded);
        let (first_full, first_version) = report.packages.first().cloned().unwrap();
        let dir = crate::fetch::cached_package_dir(&report.registry, &first_full, &first_version);
        let target = dir.join("src").join("main.rnx");
        assert!(target.is_file());
        let mut bytes = std::fs::read(&target).unwrap();
        bytes.extend_from_slice(b"\n");
        std::fs::write(&target, &bytes).unwrap();
        assert_ne!(stdlib_sources_digest(), seeded);
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
    fn src_tree_hash_skips_build_dirs() {
        let dir = std::env::temp_dir().join(format!("rnx-src-skip-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        let first = hash_src_tree(&dir);
        for skipped in ["target", ".git", ".rnx-cache"] {
            let nested = dir.join("src").join(skipped);
            std::fs::create_dir_all(&nested).unwrap();
            std::fs::write(nested.join("junk.rnx"), "fn Junk(): Int { return 9; }\n").unwrap();
        }
        assert_eq!(hash_src_tree(&dir), first);
        let _ = std::fs::remove_dir_all(&dir);
    }

    fn src_tree_fixture(dir: &Path) {
        let _ = std::fs::remove_dir_all(dir);
        std::fs::create_dir_all(dir.join("src").join("lib")).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        std::fs::write(dir.join("src").join("lib").join("util.rnx"), "fn Util(): Int { return 1; }\n")
            .unwrap();
        std::fs::write(dir.join("src").join("notes.txt"), "ignored").unwrap();
        std::fs::write(dir.join("notes.txt"), "ignored").unwrap();
    }

    fn file_entry<'a>(files: &'a [(String, Vec<u8>)], rel: &str) -> &'a Vec<u8> {
        files
            .iter()
            .find(|(r, _)| r == rel)
            .map(|(_, h)| h)
            .expect("expected per-file entry")
    }

    #[test]
    fn src_tree_files_digest_matches_legacy_fold() {
        let dir = std::env::temp_dir().join(format!("rnx-src-files-{}", std::process::id()));
        src_tree_fixture(&dir);
        let (digest, files) = hash_src_tree_files(&dir);
        assert_eq!(digest.len(), 32);
        assert_eq!(digest, hash_src_tree(&dir));
        let mut expected = Sha256::new();
        for name in ["src/lib/util.rnx", "src/main.rnx"] {
            let bytes = std::fs::read(dir.join(name)).unwrap();
            let rel = name.as_bytes();
            expected.update(&(rel.len() as u64).to_le_bytes());
            expected.update(rel);
            expected.update(&(bytes.len() as u64).to_le_bytes());
            expected.update(&bytes);
        }
        assert_eq!(digest, expected.finalize().to_vec());
        let rels: Vec<&str> = files.iter().map(|(r, _)| r.as_str()).collect();
        assert_eq!(rels, vec!["src/lib/util.rnx", "src/main.rnx"]);
        for (_, h) in &files {
            assert_eq!(h.len(), 32);
        }
        for (rel, h) in &files {
            let bytes = std::fs::read(dir.join(rel)).unwrap();
            let mut entry = Sha256::new();
            let rel_bytes = rel.as_bytes();
            entry.update(&(rel_bytes.len() as u64).to_le_bytes());
            entry.update(rel_bytes);
            entry.update(&(bytes.len() as u64).to_le_bytes());
            entry.update(&bytes);
            assert_eq!(h, &entry.finalize().to_vec());
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn src_tree_files_edit_changes_single_entry() {
        let dir = std::env::temp_dir().join(format!("rnx-src-files-edit-{}", std::process::id()));
        src_tree_fixture(&dir);
        let (before_digest, before_files) = hash_src_tree_files(&dir);
        std::fs::write(dir.join("src").join("lib").join("util.rnx"), "fn Util(): Int { return 2; }\n")
            .unwrap();
        let (after_digest, after_files) = hash_src_tree_files(&dir);
        assert_ne!(before_digest, after_digest);
        assert_eq!(before_files.len(), 2);
        assert_eq!(after_files.len(), 2);
        assert_eq!(
            file_entry(&before_files, "src/main.rnx"),
            file_entry(&after_files, "src/main.rnx")
        );
        assert_ne!(
            file_entry(&before_files, "src/lib/util.rnx"),
            file_entry(&after_files, "src/lib/util.rnx")
        );
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn src_tree_files_ignores_non_rnx() {
        let dir =
            std::env::temp_dir().join(format!("rnx-src-files-ignore-{}", std::process::id()));
        src_tree_fixture(&dir);
        let (before_digest, before_files) = hash_src_tree_files(&dir);
        std::fs::write(dir.join("src").join("extra.txt"), "ignored").unwrap();
        std::fs::write(dir.join("README.md"), "ignored").unwrap();
        let (after_digest, after_files) = hash_src_tree_files(&dir);
        assert_eq!(before_digest, after_digest);
        assert_eq!(before_files, after_files);
        assert!(after_files.iter().all(|(r, _)| r.ends_with(".rnx")));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[cfg(unix)]
    #[test]
    fn src_tree_files_skips_symlinks() {
        use std::os::unix::fs::symlink;
        let dir =
            std::env::temp_dir().join(format!("rnx-src-files-link-{}", std::process::id()));
        src_tree_fixture(&dir);
        let (before_digest, before_files) = hash_src_tree_files(&dir);
        symlink(dir.join("src").join("main.rnx"), dir.join("src").join("alias.rnx")).unwrap();
        let (after_digest, after_files) = hash_src_tree_files(&dir);
        assert_eq!(before_digest, after_digest);
        assert_eq!(before_files, after_files);
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

    #[test]
    fn nested_bin_files_count_toward_lru() {
        let dir = std::env::temp_dir().join(format!("rnx-cache-nested-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let deep = dir.join("deps").join("pkg").join("1.0.0").join("tool");
        std::fs::create_dir_all(&deep).unwrap();
        std::fs::write(deep.join("product.bin"), b"1234567").unwrap();
        std::fs::write(dir.join("top.bin"), b"abc").unwrap();
        std::fs::write(dir.join("notes.txt"), b"ignored").unwrap();
        std::fs::create_dir_all(dir.join("empty")).unwrap();
        assert_eq!(cache_usage_bytes(&dir), 10);
        let freed = prune_lru(&dir, 5).unwrap();
        let rest = cache_usage_bytes(&dir);
        assert!(rest <= 5, "usage after prune is {rest}");
        assert_eq!(freed + rest, 10);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
