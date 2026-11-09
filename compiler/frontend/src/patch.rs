use crate::project::{PatchSource, ProjectConfig};
use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock};

#[derive(Clone, Default)]
struct PatchRoots {
    vendor: Option<PathBuf>,
    wrappers: Option<PathBuf>,
}

fn roots() -> &'static Mutex<BTreeMap<PathBuf, PatchRoots>> {
    static ROOTS: OnceLock<Mutex<BTreeMap<PathBuf, PatchRoots>>> = OnceLock::new();
    ROOTS.get_or_init(|| Mutex::new(BTreeMap::new()))
}

static ACTIVE_WRAPPERS: OnceLock<Mutex<Option<PathBuf>>> = OnceLock::new();

fn active_wrappers_slot() -> &'static Mutex<Option<PathBuf>> {
    ACTIVE_WRAPPERS.get_or_init(|| Mutex::new(None))
}

pub fn active_wrappers() -> Option<PathBuf> {
    active_wrappers_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .clone()
}

pub fn apply(project_root: &Path, cfg: &ProjectConfig) -> Result<(), Diagnostic> {
    let Some(patch) = &cfg.patch else {
        return Ok(());
    };
    let anchor = crate::fetch::cache_anchor(project_root);
    let cache = anchor.join(".rnx-cache").join("patches");
    let vendor = match &patch.vendor {
        Some(source) => Some(resolve(source, project_root, &cache, "vendor")?),
        None => None,
    };
    let wrappers = match &patch.wrappers {
        Some(source) => Some(resolve(source, project_root, &cache, "wrappers")?),
        None => None,
    };
    *active_wrappers_slot()
        .lock()
        .unwrap_or_else(|e| e.into_inner()) = wrappers.clone();
    roots()
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(anchor, PatchRoots { vendor, wrappers });
    Ok(())
}

pub fn vendor_root(anchor: &Path) -> Option<PathBuf> {
    lookup(anchor, |r| r.vendor.clone())
}

pub fn wrappers_root(anchor: &Path) -> Option<PathBuf> {
    lookup(anchor, |r| r.wrappers.clone())
}

fn lookup(anchor: &Path, pick: impl Fn(&PatchRoots) -> Option<PathBuf>) -> Option<PathBuf> {
    let map = roots().lock().unwrap_or_else(|e| e.into_inner());
    let mut best: Option<(usize, PathBuf)> = None;
    for (root, roots) in map.iter() {
        let Some(dir) = pick(roots) else {
            continue;
        };
        if anchor == root || anchor.starts_with(root) {
            let depth = root.components().count();
            if best.as_ref().map(|(d, _)| depth > *d).unwrap_or(true) {
                best = Some((depth, dir));
            }
        }
    }
    best.map(|(_, dir)| dir)
}

fn resolve(
    source: &PatchSource,
    project_root: &Path,
    cache: &Path,
    kind: &str,
) -> Result<PathBuf, Diagnostic> {
    let (base, subdir) = match source {
        PatchSource::Path { path, subdir } => (project_root.join(path), subdir.clone()),
        PatchSource::Git { git, rev, subdir } => {
            let key = format!(
                "{kind}-{}",
                crate::checksum::Sha256::hexdigest(format!("{git}\0{rev}").as_bytes())
            );
            (
                crate::fetch::fetch_git_tree(git, rev, &cache.join("git"), &key)?,
                subdir.clone(),
            )
        }
        PatchSource::Oci { oci, subdir } => (
            crate::oci::fetch_patch_artifact(oci, &cache.join("oci"), kind)?,
            subdir.clone(),
        ),
    };
    let dir = match subdir {
        Some(sub) => base.join(sub),
        None => base,
    };
    let dir = std::fs::canonicalize(&dir).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("patch directory `{}` is unreadable: {e}", dir.display()),
        )
    })?;
    if !dir.is_dir() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("patch `{kind}` path `{}` is not a directory", dir.display()),
        ));
    }
    Ok(dir)
}
