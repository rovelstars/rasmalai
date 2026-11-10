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

}
