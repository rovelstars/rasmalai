use std::path::{Path, PathBuf};

/// Release artifacts must never pick up a build machine's checkout. Builds
/// made in CI, or with `RNX_RELEASE_BUILD` set, bake this to false.
pub const CHECKOUT_FALLBACK: bool = match option_env!("RNX_CHECKOUT_STDLIB") {
    Some(value) => value.len() == 1 && value.as_bytes()[0] == b'1',
    None => true,
};

pub fn source_stdlib() -> Option<PathBuf> {
    if let Some(stdlib) = exe_relative_stdlib() {
        return Some(stdlib);
    }
    manifest_stdlib()
}

fn exe_relative_stdlib() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let repo = exe.parent()?.parent()?.parent()?;
    let stdlib = repo.join("compiler").join("stdlib").join("src");
    has_stdlib(&stdlib).then_some(stdlib)
}

fn manifest_stdlib() -> Option<PathBuf> {
    if !CHECKOUT_FALLBACK {
        return None;
    }
    let manifest = Path::new(env!("CARGO_MANIFEST_DIR"));
    let stdlib = manifest.parent()?.join("stdlib").join("src");
    has_stdlib(&stdlib).then_some(stdlib)
}

fn has_stdlib(path: &Path) -> bool {
    path.join("app").join("Project.config").is_file()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_checkout_with_the_app_package_is_detected() {
        let root = crate::scratch::dir("checkout");
        let stdlib = root.join("compiler").join("stdlib").join("src");
        std::fs::create_dir_all(stdlib.join("app")).unwrap();
        std::fs::write(stdlib.join("app").join("Project.config"), "").unwrap();
        assert!(has_stdlib(&stdlib));
    }

    #[test]
    fn an_empty_checkout_is_not_detected() {
        let root = crate::scratch::dir("checkout-empty");
        let stdlib = root.join("compiler").join("stdlib").join("src");
        std::fs::create_dir_all(&stdlib).unwrap();
        assert!(!has_stdlib(&stdlib));
    }
}
