use std::path::{Path, PathBuf};

pub const ENV_VAR: &str = "RNX_SYSROOT";
pub const MANIFEST_FILE: &str = "sysroot.json";
pub const MANIFEST_VERSION: u64 = 1;
pub const MAX_STD_MODULE_BYTES: usize = 1024 * 1024;

pub const SUPPORTED_TARGETS: &[&str] = &[
    "x86_64-unknown-linux-gnu",
    "aarch64-unknown-linux-gnu",
    "x86_64-unknown-linux-musl",
];

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Family {
    Gnu,
    Musl,
}

pub fn family_of(triple: &str) -> Option<Family> {
    if triple.ends_with("-unknown-linux-musl") {
        return Some(Family::Musl);
    }
    if triple.ends_with("-unknown-linux-gnu") {
        return Some(Family::Gnu);
    }
    None
}

pub fn is_supported_target(triple: &str) -> bool {
    SUPPORTED_TARGETS.contains(&triple)
}

pub fn interp_for(triple: &str) -> Option<&'static str> {
    match triple {
        "x86_64-unknown-linux-gnu" => Some("/lib64/ld-linux-x86-64.so.2"),
        "aarch64-unknown-linux-gnu" => Some("/lib/ld-linux-aarch64.so.1"),
        "x86_64-unknown-linux-musl" => Some("/lib/ld-musl-x86_64.so.1"),
        _ => None,
    }
}

pub fn sys_lib_file(family: Family, lib: &str) -> Option<&'static str> {
    match (family, lib) {
        (Family::Gnu, "c") | (Family::Gnu, "pthread") | (Family::Gnu, "dl") => Some("libc.a"),
        (Family::Musl, "c") | (Family::Musl, "pthread") | (Family::Musl, "dl") => {
            Some("libc.a")
        }
        (_, "m") => Some("libm.a"),
        _ => None,
    }
}

pub fn shared_lib_file(family: Family, lib: &str) -> Option<&'static str> {
    match (family, lib) {
        (Family::Gnu, "c") | (Family::Gnu, "pthread") | (Family::Gnu, "dl") => {
            Some("libc.so.6")
        }
        (Family::Gnu, "m") => Some("libm.so.6"),
        (Family::Gnu, "gcc_s") => Some("libgcc_s.so.1"),
        _ => None,
    }
}

pub fn compiler_runtime_archives(family: Family, fully_static: bool) -> &'static [&'static str] {
    match (family, fully_static) {
        (Family::Gnu, true) | (Family::Musl, _) => &["libgcc.a", "libgcc_eh.a"],
        (Family::Gnu, false) => &["libgcc.a"],
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Manifest {
    pub version: u64,
    pub std_version: Option<String>,
    pub targets: Vec<String>,
    pub content_hash: Option<String>,
}

pub fn parse_manifest(body: &[u8]) -> Result<Manifest, String> {
    let value: serde_json::Value =
        serde_json::from_slice(body).map_err(|e| format!("{MANIFEST_FILE}: invalid JSON: {e}"))?;
    let version = value
        .get("version")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| format!("{MANIFEST_FILE}: missing numeric `version`"))?;
    if version != MANIFEST_VERSION {
        return Err(format!(
            "{MANIFEST_FILE}: unsupported version {version}; expected {MANIFEST_VERSION}"
        ));
    }
    let std_version = value
        .get("std_version")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    let mut targets = Vec::new();
    if let Some(list) = value.get("targets").and_then(|v| v.as_array()) {
        for entry in list {
            match entry.as_str() {
                Some(t) => targets.push(t.to_string()),
                None => {
                    return Err(format!("{MANIFEST_FILE}: `targets` entries must be strings"));
                }
            }
        }
    }
    targets.sort();
    targets.dedup();
    let content_hash = value
        .get("content_hash")
        .and_then(|v| v.as_str())
        .map(|s| s.to_string());
    Ok(Manifest { version, std_version, targets, content_hash })
}

fn host_driver() -> String {
    let driver = std::env::var_os("RNX_CC")
        .or_else(|| std::env::var_os("CC"))
        .map(|s| s.to_string_lossy().into_owned())
        .unwrap_or_else(|| "cc".to_string());
    driver
}

fn host_root_uncached() -> PathBuf {
    let out = std::process::Command::new(host_driver())
        .arg("-print-sysroot")
        .output();
    match out {
        Ok(o) if o.status.success() => {
            let text = String::from_utf8_lossy(&o.stdout).trim().to_string();
            if text.is_empty() {
                PathBuf::from("/")
            } else {
                PathBuf::from(text)
            }
        }
        _ => PathBuf::from("/"),
    }
}

fn host_root_cached() -> PathBuf {
    static ROOT: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    ROOT.get_or_init(host_root_uncached).clone()
}

pub fn normalize_std_rest(name: &str) -> String {
    name.strip_prefix("@std/")
        .or_else(|| name.strip_prefix("std/"))
        .unwrap_or(name)
        .to_string()
}

pub fn content_hash(root: &Path) -> String {
    use sha2::{Digest, Sha256};
    let mut files: Vec<(String, Vec<u8>)> = Vec::new();
    collect_payload(root, root, &mut files);
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut hasher = Sha256::new();
    hasher.update(&(files.len() as u64).to_le_bytes());
    for (rel, bytes) in &files {
        hasher.update(&(rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update(&(bytes.len() as u64).to_le_bytes());
        hasher.update(bytes);
    }
    let digest = hasher.finalize();
    let mut out = String::with_capacity(7 + digest.len() * 2);
    out.push_str("sha256:");
    for b in digest {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
    }
    out
}

fn collect_payload(root: &Path, dir: &Path, out: &mut Vec<(String, Vec<u8>)>) {
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
            collect_payload(root, &path, out);
            continue;
        }
        if !meta.file_type().is_file() {
            continue;
        }
        if path.file_name().and_then(|n| n.to_str()) == Some(MANIFEST_FILE) && path.parent() == Some(root) {
            continue;
        }
        let rel = match path.strip_prefix(root) {
            Ok(r) => r.to_string_lossy().replace('\\', "/"),
            Err(_) => continue,
        };
        out.push((rel, std::fs::read(&path).unwrap_or_default()));
    }
}

#[derive(Clone, Debug)]
pub struct Sysroot {
    root: Option<PathBuf>,
}

impl Sysroot {
    pub fn host() -> Sysroot {
        Sysroot { root: None }
    }

    pub fn custom(root: PathBuf) -> Sysroot {
        Sysroot { root: Some(root) }
    }

    pub fn discover(explicit: Option<&Path>) -> Sysroot {
        if let Some(path) = explicit {
            if !path.as_os_str().is_empty() {
                return Sysroot::custom(path.to_path_buf());
            }
        }
        match std::env::var(ENV_VAR).ok().filter(|s| !s.trim().is_empty()) {
            Some(dir) => Sysroot::custom(PathBuf::from(dir)),
            None => Sysroot::host(),
        }
    }

    pub fn is_host(&self) -> bool {
        self.root.is_none()
    }

    pub fn root(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    pub fn base_dir(&self) -> PathBuf {
        match &self.root {
            Some(root) => root.clone(),
            None => host_root_cached(),
        }
    }

    pub fn display_root(&self) -> String {
        self.base_dir().to_string_lossy().into_owned()
    }

    pub fn validate(&self) -> Result<Manifest, String> {
        match &self.root {
            None => Ok(Manifest {
                version: MANIFEST_VERSION,
                std_version: None,
                targets: SUPPORTED_TARGETS.iter().map(|s| s.to_string()).collect(),
                content_hash: None,
            }),
            Some(root) => {
                let body = std::fs::read(root.join(MANIFEST_FILE)).map_err(|e| {
                    format!("sysroot `{}` has no {MANIFEST_FILE}: {e}", root.display())
                })?;
                parse_manifest(&body)
            }
        }
    }

    pub fn target_lib_dir(&self, triple: &str) -> Option<PathBuf> {
        let cand = self.base_dir().join("lib").join("rustlib").join(triple).join("lib");
        if cand.is_dir() {
            Some(cand)
        } else {
            None
        }
    }

    fn std_candidates(&self, rest: &str) -> Vec<PathBuf> {
        let rest = normalize_std_rest(rest);
        let base = self.base_dir().join("share").join("rnx").join("std");
        vec![base.join(format!("{rest}.rnx")), base.join(&rest).join("lib.rnx")]
    }

    pub fn has_std_source(&self, rest: &str) -> bool {
        if self.root.is_none() {
            return false;
        }
        self.std_candidates(rest).iter().any(|p| p.is_file())
    }

    pub fn std_source(&self, rest: &str) -> Option<String> {
        if self.root.is_none() {
            return None;
        }
        for path in self.std_candidates(rest) {
            let Ok(bytes) = std::fs::read(&path) else {
                continue;
            };
            if bytes.is_empty() || bytes.len() > MAX_STD_MODULE_BYTES {
                continue;
            }
            if let Ok(text) = String::from_utf8(bytes) {
                return Some(text);
            }
        }
        None
    }

    pub fn std_version(&self) -> Option<String> {
        if self.root.is_none() {
            return None;
        }
        self.validate().ok().and_then(|m| m.std_version)
    }

    pub fn identity(&self) -> String {
        match &self.root {
            Some(root) => content_hash(root),
            None => "host".to_string(),
        }
    }

    pub fn std_overlay_digest(&self) -> Option<Vec<u8>> {
        if self.root.is_none() {
            return None;
        }
        use sha2::{Digest, Sha256};
        let base = self.base_dir().join("share").join("rnx").join("std");
        if !base.is_dir() {
            return None;
        }
        let mut files: Vec<(String, Vec<u8>)> = Vec::new();
        collect_payload(&base, &base, &mut files);
        files.sort_by(|a, b| a.0.cmp(&b.0));
        let mut hasher = Sha256::new();
        hasher.update(&(files.len() as u64).to_le_bytes());
        for (rel, bytes) in &files {
            hasher.update(&(rel.len() as u64).to_le_bytes());
            hasher.update(rel.as_bytes());
            hasher.update(&(bytes.len() as u64).to_le_bytes());
            hasher.update(bytes);
        }
        Some(hasher.finalize().to_vec())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn scratch(tag: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-sysroot-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn write_manifest(root: &Path, extra: &str) {
        let body = format!("{{\"version\": 1{extra}}}");
        std::fs::write(root.join(MANIFEST_FILE), body).unwrap();
    }

    #[test]
    fn family_classification_is_triple_driven() {
        assert_eq!(family_of("x86_64-unknown-linux-gnu"), Some(Family::Gnu));
        assert_eq!(family_of("aarch64-unknown-linux-gnu"), Some(Family::Gnu));
        assert_eq!(family_of("x86_64-unknown-linux-musl"), Some(Family::Musl));
        assert_eq!(family_of("x86_64-pc-windows-msvc"), None);
        assert_eq!(family_of("wasm32-unknown-unknown"), None);
    }

    #[test]
    fn loader_paths_are_baked_per_target() {
        assert_eq!(interp_for("x86_64-unknown-linux-gnu"), Some("/lib64/ld-linux-x86-64.so.2"));
        assert_eq!(interp_for("x86_64-unknown-linux-musl"), Some("/lib/ld-musl-x86_64.so.1"));
        assert_eq!(interp_for("aarch64-unknown-linux-gnu"), Some("/lib/ld-linux-aarch64.so.1"));
        assert_eq!(interp_for("x86_64-pc-windows-msvc"), None);
    }

    #[test]
    fn musl_never_selects_shared_objects() {
        assert_eq!(shared_lib_file(Family::Musl, "c"), None);
        assert_eq!(shared_lib_file(Family::Musl, "m"), None);
        assert_eq!(shared_lib_file(Family::Gnu, "c"), Some("libc.so.6"));
        assert_eq!(shared_lib_file(Family::Gnu, "m"), Some("libm.so.6"));
        assert_eq!(shared_lib_file(Family::Gnu, "gcc_s"), Some("libgcc_s.so.1"));
        assert_eq!(sys_lib_file(Family::Musl, "pthread"), Some("libc.a"));
        assert_eq!(sys_lib_file(Family::Gnu, "pthread"), Some("libc.a"));
        assert_eq!(sys_lib_file(Family::Musl, "m"), Some("libm.a"));
        assert_eq!(sys_lib_file(Family::Gnu, "rt"), None);
    }

    #[test]
    fn discovery_prefers_explicit_over_env_over_host() {
        let explicit = scratch("explicit");
        write_manifest(&explicit, "");
        let from_env = scratch("env");
        write_manifest(&from_env, "");
        unsafe { std::env::set_var(ENV_VAR, &from_env) };
        let found = Sysroot::discover(Some(&explicit));
        assert_eq!(found.root(), Some(explicit.as_path()));
        let found = Sysroot::discover(None);
        assert_eq!(found.root(), Some(from_env.as_path()));
        unsafe { std::env::remove_var(ENV_VAR) };
        let found = Sysroot::discover(None);
        assert!(found.is_host());
        let _ = std::fs::remove_dir_all(&explicit);
        let _ = std::fs::remove_dir_all(&from_env);
    }

    #[test]
    fn custom_sysroot_requires_a_versioned_manifest() {
        let root = scratch("manifest");
        let bad = Sysroot::custom(root.clone());
        assert!(bad.validate().is_err());
        std::fs::write(root.join(MANIFEST_FILE), "{\"version\": 999}").unwrap();
        assert!(bad.validate().is_err());
        std::fs::write(
            root.join(MANIFEST_FILE),
            "{\"version\": 1, \"std_version\": \"9.9.9\", \"targets\": [\"b\", \"a\", \"b\"]}",
        )
        .unwrap();
        let manifest = bad.validate().unwrap();
        assert_eq!(manifest.std_version.as_deref(), Some("9.9.9"));
        assert_eq!(manifest.targets, vec!["a".to_string(), "b".to_string()]);
        assert!(bad.target_lib_dir("x86_64-unknown-linux-musl").is_none());
        std::fs::create_dir_all(
            root.join("lib").join("rustlib").join("x86_64-unknown-linux-musl").join("lib"),
        )
        .unwrap();
        assert!(bad.target_lib_dir("x86_64-unknown-linux-musl").is_some());
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn identity_ignores_mtimes_and_manifest_itself() {
        let root = scratch("identity");
        write_manifest(&root, "");
        let payload = root.join("lib").join("a.a");
        std::fs::create_dir_all(payload.parent().unwrap()).unwrap();
        std::fs::write(&payload, b"bytes").unwrap();
        let first = Sysroot::custom(root.clone()).identity();
        assert!(first.starts_with("sha256:"));
        std::fs::write(root.join(MANIFEST_FILE), "{\"version\": 1, \"note\": \"changed\"}").unwrap();
        assert_eq!(Sysroot::custom(root.clone()).identity(), first);
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        let _ = std::fs::File::options().write(true).open(&payload).map(|f| {
            f.set_modified(later).ok();
        });
        assert_eq!(Sysroot::custom(root.clone()).identity(), first);
        std::fs::write(&payload, b"other").unwrap();
        assert_ne!(Sysroot::custom(root.clone()).identity(), first);
        assert_eq!(Sysroot::host().identity(), "host");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn std_overlay_reads_file_then_directory_form() {
        let root = scratch("stdoverlay");
        write_manifest(&root, "");
        let sysroot = Sysroot::custom(root.clone());
        assert!(!sysroot.has_std_source("prelude"));
        let base = root.join("share").join("rnx").join("std");
        std::fs::create_dir_all(&base).unwrap();
        std::fs::write(base.join("prelude.rnx"), "export fn custom(): Int { return 1; }\n").unwrap();
        assert!(sysroot.has_std_source("@std/prelude"));
        assert!(sysroot.std_source("prelude").unwrap().contains("custom"));
        std::fs::create_dir_all(base.join("time")).unwrap();
        std::fs::write(base.join("time").join("lib.rnx"), "export fn t(): Int { return 2; }\n").unwrap();
        assert!(sysroot.std_source("@std/time").unwrap().contains("fn t"));
        assert!(Sysroot::host().std_source("prelude").is_none());
        let _ = std::fs::remove_dir_all(&root);
    }
}
