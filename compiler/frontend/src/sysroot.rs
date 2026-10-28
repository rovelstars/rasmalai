use std::path::PathBuf;
use std::sync::Mutex;

static OVERRIDE: Mutex<Option<PathBuf>> = Mutex::new(None);

fn override_lock() -> std::sync::MutexGuard<'static, Option<PathBuf>> {
    OVERRIDE.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set_test_root(root: Option<PathBuf>) {
    *override_lock() = root;
}

pub fn configured_root() -> Option<PathBuf> {
    if let Some(root) = override_lock().clone() {
        return Some(root);
    }
    std::env::var(sysroot::ENV_VAR)
        .ok()
        .filter(|s| !s.trim().is_empty())
        .map(PathBuf::from)
}

pub fn active_sysroot() -> sysroot::Sysroot {
    match configured_root() {
        Some(root) => sysroot::Sysroot::custom(root),
        None => sysroot::Sysroot::host(),
    }
}

pub fn active_root() -> Option<PathBuf> {
    configured_root()
}

pub fn validation_error() -> Option<String> {
    let sysroot = active_sysroot();
    if sysroot.is_host() {
        return None;
    }
    sysroot.validate().err()
}

pub fn has_std_source(rest: &str) -> bool {
    active_sysroot().has_std_source(rest)
}

pub fn std_source(rest: &str) -> Option<String> {
    active_sysroot().std_source(rest)
}

pub fn manifest_std_version() -> Option<String> {
    let sysroot = active_sysroot();
    if sysroot.is_host() {
        return None;
    }
    sysroot.std_version()
}

pub fn overlay_digest() -> Option<Vec<u8>> {
    let sysroot = active_sysroot();
    if sysroot.is_host() {
        return None;
    }
    sysroot.std_overlay_digest()
}

pub struct TestRootGuard;

impl TestRootGuard {
    pub fn set(root: PathBuf) -> TestRootGuard {
        set_test_root(Some(root));
        TestRootGuard
    }
}

impl Drop for TestRootGuard {
    fn drop(&mut self) {
        set_test_root(None);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    static SERIAL: Mutex<()> = Mutex::new(());

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-frontend-sysroot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn override_sets_and_clears_active_root() {
        let _serial = crate::fetch::testkit::serial_env();
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let ambient = std::env::var(sysroot::ENV_VAR)
            .ok()
            .filter(|s| !s.trim().is_empty())
            .map(PathBuf::from);
        let explicit = scratch("explicit");
        std::fs::write(explicit.join("sysroot.json"), "{\"version\": 1}").unwrap();
        let _guard = TestRootGuard::set(explicit.clone());
        assert_eq!(configured_root(), Some(explicit.clone()));
        assert_eq!(active_root(), Some(explicit.clone()));
        assert!(validation_error().is_none());
        drop(_guard);
        assert_eq!(configured_root(), ambient);
        let _ = std::fs::remove_dir_all(&explicit);
    }

    fn checkout_prelude() -> String {
        let path =
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src/prelude/lib.rnx");
        std::fs::read_to_string(&path).expect("checkout prelude must exist")
    }

    #[test]
    fn overlay_serves_custom_prelude_and_extra_modules() {
        let _serial = crate::fetch::testkit::serial_env();
        let _lock = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let root = scratch("overlay");
        let std_dir = root.join("share").join("rnx").join("std");
        std::fs::create_dir_all(&std_dir).unwrap();
        let cli_version = env!("CARGO_PKG_VERSION");
        std::fs::write(
            root.join("sysroot.json"),
            format!("{{\"version\": 1, \"std_version\": \"{cli_version}\"}}"),
        )
        .unwrap();
        let mut custom = checkout_prelude();
        custom.push_str("\nexport fn sysrootMarker(): Int {\n    return 1;\n}\n");
        std::fs::write(std_dir.join("prelude.rnx"), &custom).unwrap();
        std::fs::write(
            std_dir.join("bareonly.rnx"),
            "export fn bareValue(): Int {\n    return 7;\n}\n",
        )
        .unwrap();
        let _guard = TestRootGuard::set(root.clone());
        assert_eq!(crate::stdvfs::std_source("prelude").unwrap(), custom);
        assert!(crate::stdvfs::std_source("bareonly").unwrap().contains("bareValue"));
        assert!(crate::stdvfs::is_known_module("bareonly"));
        assert!(!crate::stdvfs::is_known_module("nope"));
        assert_eq!(crate::stdvfs::pin_version(), cli_version.to_string());
        assert!(crate::prelude::provides("sysrootMarker"));
        assert!(crate::prelude::provides("Int"));
        assert!(!crate::prelude::provides("Ghost"));
        drop(_guard);
        assert!(!has_std_source("bareonly"));
        let _ = std::fs::remove_dir_all(&root);
    }
}
