use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{LazyLock, Mutex};

pub const CACHE_SUBDIR: &str = "stdlib";
pub const LIB_FILENAME: &str = "librnx_stdlib.so";
pub const FINGERPRINT_FILENAME: &str = "fingerprint.txt";
const KEY_TAG: &[u8] = b"rnx-stdlib-prebuilt-v1";

pub fn stdlib_cache_key() -> String {
    let digest = frontend::cache::stdlib_sources_digest();
    frontend::cache::fingerprint_hex(&[
        KEY_TAG,
        b"release",
        std::env::consts::OS.as_bytes(),
        std::env::consts::ARCH.as_bytes(),
        env!("CARGO_PKG_VERSION").as_bytes(),
        &digest,
    ])
}

pub fn artifact_dir_in(cache_root: &Path) -> PathBuf {
    cache_root.join(CACHE_SUBDIR).join(stdlib_cache_key())
}

pub fn artifact_dir() -> PathBuf {
    artifact_dir_in(&frontend::cache::global_cache_dir())
}

pub fn artifact_path_in(cache_root: &Path) -> Option<PathBuf> {
    #[cfg(not(target_os = "linux"))]
    {
        // Linker .so layout is Linux-only and win/mac cache entries are
        // unaudited, so non-Linux stays on inline-JIT stdlib.
        let _ = cache_root;
        None
    }
    #[cfg(target_os = "linux")]
    {
        Some(artifact_dir_in(cache_root).join(LIB_FILENAME))
    }
}

pub fn artifact_path() -> Option<PathBuf> {
    artifact_path_in(&frontend::cache::global_cache_dir())
}

fn sidecar_matches(dir: &Path, key: &str) -> bool {
    match std::fs::read_to_string(dir.join(FINGERPRINT_FILENAME)) {
        Ok(text) => text.trim_end() == key,
        Err(_) => false,
    }
}

pub fn cached_stdlib_in(cache_root: &Path) -> Option<PathBuf> {
    let dir = artifact_dir_in(cache_root);
    let lib = artifact_path_in(cache_root)?;
    if !lib.is_file() {
        return None;
    }
    let key = stdlib_cache_key();
    if !sidecar_matches(&dir, &key) {
        return None;
    }
    Some(lib)
}

pub fn cached_stdlib() -> Option<PathBuf> {
    cached_stdlib_in(&frontend::cache::global_cache_dir())
}

pub fn mark_cached_in(cache_root: &Path) -> std::io::Result<PathBuf> {
    let dir = artifact_dir_in(cache_root);
    std::fs::create_dir_all(&dir)?;
    let key = stdlib_cache_key();
    std::fs::write(dir.join(FINGERPRINT_FILENAME), format!("{key}\n"))?;
    Ok(dir.join(LIB_FILENAME))
}

static OPEN_HANDLES: LazyLock<Mutex<HashMap<String, usize>>> =
    LazyLock::new(|| Mutex::new(HashMap::new()));

pub struct LoadedStdlib {
    key: String,
}

impl LoadedStdlib {
    pub fn key(&self) -> &str {
        &self.key
    }

    pub fn symbol(&self, name: &str) -> Option<*const u8> {
        #[cfg(target_os = "linux")]
        {
            let guard = OPEN_HANDLES.lock().ok()?;
            let handle = *guard.get(&self.key)? as *mut libc::c_void;
            let bytes = std::ffi::CString::new(name).ok()?;
            let addr = unsafe { libc::dlsym(handle, bytes.as_ptr()) };
            if addr.is_null() {
                return None;
            }
            Some(addr as *const u8)
        }
        #[cfg(not(target_os = "linux"))]
        {
            let _ = name;
            None
        }
    }
}

#[cfg(target_os = "linux")]
fn dlopen_now(path: &Path) -> Option<*mut libc::c_void> {
    use std::os::unix::ffi::OsStrExt;
    let raw = std::ffi::CString::new(path.as_os_str().as_bytes()).ok()?;
    let handle = unsafe { libc::dlopen(raw.as_ptr(), libc::RTLD_NOW | libc::RTLD_LOCAL) };
    if handle.is_null() {
        return None;
    }
    Some(handle)
}

pub fn open_cached_stdlib_in(cache_root: &Path) -> Option<LoadedStdlib> {
    #[cfg(not(target_os = "linux"))]
    {
        let _ = cache_root;
        return None;
    }
    #[cfg(target_os = "linux")]
    {
        let lib = cached_stdlib_in(cache_root)?;
        let key = lib.to_string_lossy().into_owned();
        {
            let guard = OPEN_HANDLES.lock().ok()?;
            if guard.contains_key(&key) {
                return Some(LoadedStdlib { key });
            }
        }
        let handle = dlopen_now(&lib)?;
        match OPEN_HANDLES.lock() {
            Ok(mut guard) => {
                guard.insert(key.clone(), handle as usize);
            }
            Err(_) => {
                unsafe { libc::dlclose(handle) };
                return None;
            }
        }
        Some(LoadedStdlib { key })
    }
}

pub fn open_cached_stdlib() -> Option<LoadedStdlib> {
    open_cached_stdlib_in(&frontend::cache::global_cache_dir())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cache_key_is_stable_hex() {
        let first = stdlib_cache_key();
        assert_eq!(first.len(), 64);
        assert!(first.chars().all(|c| c.is_ascii_hexdigit()));
        assert_eq!(stdlib_cache_key(), first);
    }

    #[test]
    fn cache_key_tracks_stdlib_digest() {
        let digest = frontend::cache::stdlib_sources_digest();
        assert!(!digest.is_empty());
        let direct = frontend::cache::fingerprint_hex(&[
            KEY_TAG,
            b"release",
            std::env::consts::OS.as_bytes(),
            std::env::consts::ARCH.as_bytes(),
            env!("CARGO_PKG_VERSION").as_bytes(),
            &digest,
        ]);
        assert_eq!(stdlib_cache_key(), direct);
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!(
            "rnx-stdlib-cache-{name}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn missing_artifact_falls_back() {
        let root = scratch("missing");
        assert_eq!(cached_stdlib_in(&root), None);
        assert!(open_cached_stdlib_in(&root).is_none());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn stale_sidecar_falls_back() {
        let root = scratch("stale");
        #[cfg(target_os = "linux")]
        {
            let dir = artifact_dir_in(&root);
            std::fs::create_dir_all(&dir).unwrap();
            std::fs::write(dir.join(LIB_FILENAME), b"fake").unwrap();
            std::fs::write(dir.join(FINGERPRINT_FILENAME), "deadbeef\n").unwrap();
            assert_eq!(cached_stdlib_in(&root), None);
            assert!(open_cached_stdlib_in(&root).is_none());
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(cached_stdlib_in(&root), None);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn sidecar_without_library_falls_back() {
        let root = scratch("nolib");
        #[cfg(target_os = "linux")]
        {
            mark_cached_in(&root).unwrap();
            assert_eq!(cached_stdlib_in(&root), None);
        }
        #[cfg(not(target_os = "linux"))]
        {
            assert_eq!(cached_stdlib_in(&root), None);
        }
        let _ = std::fs::remove_dir_all(&root);
    }

    fn has_cc() -> bool {
        std::env::var_os("PATH").map_or(false, |paths| {
            std::env::split_paths(&paths).any(|d| {
                d.join("cc").is_file() || d.join("gcc").is_file() || d.join("clang").is_file()
            })
        })
    }

    fn cc_driver() -> Option<String> {
        for name in ["cc", "gcc", "clang"] {
            let found = std::env::var_os("PATH").map_or(false, |paths| {
                std::env::split_paths(&paths).any(|d| d.join(name).is_file())
            });
            if found {
                return Some(name.to_string());
            }
        }
        None
    }

    #[test]
    fn probe_library_loads_and_resolves() {
        if !has_cc() {
            return;
        }
        let Some(driver) = cc_driver() else {
            return;
        };
        let root = scratch("probe");
        #[cfg(target_os = "linux")]
        {
            let dir = artifact_dir_in(&root);
            std::fs::create_dir_all(&dir).unwrap();
            let src = dir.join("probe.c");
            std::fs::write(&src, "int rnx_stdlib_probe(void) { return 42; }\n").unwrap();
            let out = dir.join(LIB_FILENAME);
            let status = std::process::Command::new(&driver)
                .args(["-shared", "-fPIC", "-o"])
                .arg(&out)
                .arg(&src)
                .status()
                .unwrap();
            assert!(status.success());
            mark_cached_in(&root).unwrap();
            assert_eq!(cached_stdlib_in(&root), Some(out.clone()));
            let loaded = open_cached_stdlib_in(&root).unwrap();
            assert_eq!(loaded.key(), out.to_str().unwrap());
            assert!(loaded.symbol("rnx_stdlib_probe").is_some());
            assert!(loaded.symbol("rnx_stdlib_missing_xyz").is_none());
            assert!(open_cached_stdlib_in(&root).is_some());
        }
        let _ = std::fs::remove_dir_all(&root);
    }
}
