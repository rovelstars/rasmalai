use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

use crate::depcache::DepProduct;

pub const PRODUCTS_DIR: &str = "products";
pub const GLOBAL_OBJECTS_DIR: &str = "objects";
pub const GIT_SCOPE: &str = "git";
pub const PRODUCT_PREFIX: &str = "product-";
pub const PRODUCT_EXT: &str = "bin";
pub const MANIFEST_NAME: &str = "manifest";
pub const MAX_SCAN_FILES: usize = 64;

static PROD_HITS: AtomicU64 = AtomicU64::new(0);
static PROD_MISSES: AtomicU64 = AtomicU64::new(0);
static PROD_POPULATED: AtomicU64 = AtomicU64::new(0);
static OBJ_HITS: AtomicU64 = AtomicU64::new(0);
static OBJ_MISSES: AtomicU64 = AtomicU64::new(0);

pub fn stats() -> (u64, u64, u64, u64, u64) {
    (
        PROD_HITS.load(Ordering::Relaxed),
        PROD_MISSES.load(Ordering::Relaxed),
        PROD_POPULATED.load(Ordering::Relaxed),
        OBJ_HITS.load(Ordering::Relaxed),
        OBJ_MISSES.load(Ordering::Relaxed),
    )
}

#[cfg(test)]
pub fn reset_stats() {
    PROD_HITS.store(0, Ordering::Relaxed);
    PROD_MISSES.store(0, Ordering::Relaxed);
    PROD_POPULATED.store(0, Ordering::Relaxed);
    OBJ_HITS.store(0, Ordering::Relaxed);
    OBJ_MISSES.store(0, Ordering::Relaxed);
}

pub fn products_root() -> PathBuf {
    crate::cache::global_cache_dir().join(PRODUCTS_DIR)
}

pub fn global_objects_root() -> PathBuf {
    crate::cache::global_cache_dir().join(GLOBAL_OBJECTS_DIR)
}

fn segment_ok(segment: &str) -> bool {
    !segment.is_empty()
        && segment != "."
        && segment != ".."
        && !segment.contains(['/', '\\'])
        && !segment.starts_with('.')
}

fn multipath_ok(value: &str) -> bool {
    !value.is_empty() && value.split('/').all(segment_ok)
}

pub fn product_dir(
    kind: &str,
    base: &str,
    pkg: &str,
    version: &str,
    toolchain: &str,
) -> Option<PathBuf> {
    let scope = if kind == crate::depcache::KIND_GIT {
        GIT_SCOPE.to_string()
    } else if kind == crate::depcache::KIND_SEMVER || kind == crate::depcache::KIND_STD {
        crate::fetch::host_of(base)
    } else {
        return None;
    };
    let pkg = crate::fetch::sanitize_full(pkg);
    let version = crate::fetch::sanitize_segment(version);
    let toolchain = crate::fetch::sanitize_segment(toolchain);
    if !segment_ok(&scope)
        || !multipath_ok(&pkg)
        || !segment_ok(&version)
        || !segment_ok(&toolchain)
    {
        return None;
    }
    let mut dir = products_root();
    dir.push(scope);
    dir.push(pkg);
    dir.push(version);
    dir.push(toolchain);
    Some(dir)
}

pub fn exact_pin(kind: &str, version_key: &str) -> bool {
    if kind == crate::depcache::KIND_GIT {
        return !version_key.is_empty();
    }
    if kind == crate::depcache::KIND_SEMVER || kind == crate::depcache::KIND_STD {
        return crate::modules::exact_version(version_key).is_some();
    }
    false
}

fn short_set(dep_set: &str) -> &str {
    let end = dep_set.len().min(16);
    &dep_set[..end]
}

pub fn product_file(dir: &Path, dep_set: &str) -> Option<PathBuf> {
    if dep_set.is_empty() {
        return None;
    }
    Some(dir.join(format!(
        "{PRODUCT_PREFIX}{}.{PRODUCT_EXT}",
        short_set(dep_set)
    )))
}

pub fn scan_product_files(dir: &Path) -> Vec<PathBuf> {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        let name = match path.file_name().and_then(|n| n.to_str()) {
            Some(name) => name,
            None => continue,
        };
        if !name.starts_with(PRODUCT_PREFIX) || !name.ends_with(&format!(".{PRODUCT_EXT}")) {
            continue;
        }
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_file() {
            out.push(path);
        }
    }
    out.sort();
    out.truncate(MAX_SCAN_FILES);
    out
}

pub fn read_product_file(path: &Path) -> Option<DepProduct> {
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() {
        PROD_MISSES.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    match crate::depcache::decode_product(&bytes) {
        Some(product) => {
            PROD_HITS.fetch_add(1, Ordering::Relaxed);
            Some(product)
        }
        None => {
            PROD_MISSES.fetch_add(1, Ordering::Relaxed);
            None
        }
    }
}

fn tmp_path(dest: &Path) -> PathBuf {
    dest.with_extension(format!("tmp-{}", std::process::id()))
}

fn write_atomic(dest: &Path, bytes: &[u8]) -> bool {
    if let Some(parent) = dest.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return false;
    }
    let tmp = tmp_path(dest);
    if std::fs::write(&tmp, bytes).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    if std::fs::rename(&tmp, dest).is_err() {
        let _ = std::fs::remove_file(&tmp);
        return false;
    }
    true
}

pub fn mirror_product(product: &DepProduct) -> bool {
    if product.toolchain.is_empty()
        || product.content.is_empty()
        || product.dep_set.is_empty()
        || !exact_pin(&product.kind, &product.version_key)
    {
        return false;
    }
    let dir = match product_dir(
        &product.kind,
        &product.base,
        &product.pkg,
        &product.version_key,
        &product.toolchain,
    ) {
        Some(dir) => dir,
        None => return false,
    };
    let path = match product_file(&dir, &product.dep_set) {
        Some(path) => path,
        None => return false,
    };
    if path.is_file() {
        if std::fs::read(&path)
            .ok()
            .and_then(|bytes| crate::depcache::decode_product(&bytes))
            .is_some()
        {
            return true;
        }
    }
    if !write_atomic(&path, &crate::depcache::encode_product(product)) {
        return false;
    }
    write_manifest(&dir, product);
    PROD_POPULATED.fetch_add(1, Ordering::Relaxed);
    true
}

pub fn render_manifest(product: &DepProduct) -> String {
    [
        product.pkg.as_str(),
        product.kind.as_str(),
        product.spec.as_str(),
        product.version_key.as_str(),
        product.toolchain.as_str(),
        product.dep_set.as_str(),
        product.content.as_str(),
        product.base.as_str(),
        product.commit.as_str(),
    ]
    .join("\n")
        + "\n"
}

pub fn write_manifest(dir: &Path, product: &DepProduct) -> bool {
    write_atomic(
        &dir.join(MANIFEST_NAME),
        render_manifest(product).as_bytes(),
    )
}

pub fn manifest_lines(dir: &Path) -> Option<Vec<String>> {
    let text = std::fs::read_to_string(dir.join(MANIFEST_NAME)).ok()?;
    let lines: Vec<String> = text.lines().map(|line| line.to_string()).collect();
    if lines.len() != 9 {
        return None;
    }
    Some(lines)
}

pub fn global_object_path(key: &str) -> Option<PathBuf> {
    if !crate::objects::key_ok(key) {
        return None;
    }
    Some(global_objects_root().join(format!("{key}.o")))
}

pub fn read_global_object(key: &str) -> Option<Vec<u8>> {
    let path = global_object_path(key)?;
    let bytes = std::fs::read(&path).ok()?;
    if bytes.len() < 64
        || bytes[0] != 0x7f
        || bytes[1] != b'E'
        || bytes[2] != b'L'
        || bytes[3] != b'F'
    {
        OBJ_MISSES.fetch_add(1, Ordering::Relaxed);
        return None;
    }
    OBJ_HITS.fetch_add(1, Ordering::Relaxed);
    Some(bytes)
}

pub fn write_global_object(key: &str, bytes: &[u8]) -> bool {
    match global_object_path(key) {
        Some(path) => write_atomic(&path, bytes),
        None => false,
    }
}

fn visit_object_files(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.file_type().is_dir() {
            visit_object_files(&path, out);
        } else if meta.file_type().is_file() && path.extension().is_some_and(|ext| ext == "o") {
            out.push(path);
        }
    }
}

pub fn global_object_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    visit_object_files(&global_objects_root(), &mut out);
    out
}

pub fn global_objects_usage_bytes() -> u64 {
    global_object_files()
        .iter()
        .map(|path| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .sum()
}

pub fn prune_global_objects_lru(keep_bytes: u64) -> u64 {
    let mut files: Vec<(u64, u64, PathBuf)> = Vec::new();
    for path in global_object_files() {
        let meta = match std::fs::metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        let age = meta
            .accessed()
            .or_else(|_| meta.modified())
            .ok()
            .and_then(|time| time.elapsed().ok())
            .map(|elapsed| elapsed.as_secs())
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
    freed
}

fn visit_product_bins(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) => return,
    };
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let meta = match std::fs::symlink_metadata(&path) {
            Ok(meta) => meta,
            Err(_) => continue,
        };
        if meta.file_type().is_symlink() {
            continue;
        }
        if meta.file_type().is_dir() {
            visit_product_bins(&path, out);
        } else if meta.file_type().is_file()
            && path.extension().is_some_and(|ext| ext == PRODUCT_EXT)
            && path
                .file_name()
                .and_then(|name| name.to_str())
                .is_some_and(|name| name.starts_with(PRODUCT_PREFIX))
        {
            out.push(path);
        }
    }
}

pub fn product_files() -> Vec<PathBuf> {
    let mut out = Vec::new();
    visit_product_bins(&products_root(), &mut out);
    out
}

pub fn products_usage() -> (usize, u64) {
    let files = product_files();
    let bytes = files
        .iter()
        .map(|path| std::fs::metadata(path).map(|meta| meta.len()).unwrap_or(0))
        .sum();
    (files.len(), bytes)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProductStatus {
    pub pkg: String,
    pub kind: String,
    pub version: Option<String>,
    pub present: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PrecompileReport {
    pub projects: usize,
    pub entries: usize,
    pub populated: u64,
}

pub fn empty_precompile_report() -> PrecompileReport {
    PrecompileReport { projects: 0, entries: 0, populated: 0 }
}

pub const STD_PRECOMPILE_PROJECT: &str = "std-precompile";
pub const STD_PRECOMPILE_VERSION: &str = "0.0.0";

pub fn std_scaffold_dir() -> PathBuf {
    std::env::temp_dir().join(format!("rnx-std-precompile-{}", std::process::id()))
}

fn std_scaffold_entry() -> String {
    let mut out = String::new();
    for top in crate::stdlib_seed::std_top_modules() {
        out.push_str(&format!("import \"@std/{top}\";\n"));
    }
    out.push_str("fn Main(): Int { return 0; }\n");
    out
}

pub fn write_std_scaffold(root: &Path) -> Result<(), String> {
    let manifest = format!(
        "export default {{\n    project: {{\n        name: \"{STD_PRECOMPILE_PROJECT}\",\n        version: \"{STD_PRECOMPILE_VERSION}\"\n    }}\n}}\n"
    );
    let entry = root.join("src").join("main.rnx");
    if let Some(parent) = entry.parent()
        && std::fs::create_dir_all(parent).is_err()
    {
        return Err(format!("cannot create `{}`", parent.display()));
    }
    if std::fs::write(root.join(crate::project::MANIFEST_FILE), &manifest).is_err() {
        return Err(format!(
            "cannot write `{}`",
            root.join(crate::project::MANIFEST_FILE).display()
        ));
    }
    if std::fs::write(&entry, std_scaffold_entry()).is_err() {
        return Err(format!("cannot write `{}`", entry.display()));
    }
    Ok(())
}

pub fn precompile_std() -> Result<PrecompileReport, String> {
    if !crate::depcache::dep_cache_enabled() {
        return Ok(empty_precompile_report());
    }
    if cfg!(target_arch = "wasm32") {
        return Ok(empty_precompile_report());
    }
    match crate::stdlib_seed::read_std_pin() {
        Some(pin) if !pin.packages.is_empty() => {}
        _ => return Err("stdlib cache has no pin file".to_string()),
    }
    let root = std_scaffold_dir();
    let _ = std::fs::remove_dir_all(&root);
    if let Err(e) = write_std_scaffold(&root) {
        let _ = std::fs::remove_dir_all(&root);
        return Err(e);
    }
    let report = precompile_scope(&root);
    let _ = std::fs::remove_dir_all(&root);
    if report.entries == 0 {
        return Err("stdlib entry failed to build against the seeded cache".to_string());
    }
    Ok(report)
}

pub fn std_products_summary() -> Option<(usize, usize)> {
    let pin = crate::stdlib_seed::read_std_pin()?;
    if pin.packages.is_empty() {
        return None;
    }
    let mut files = 0;
    let mut pkgs = std::collections::BTreeSet::new();
    for path in product_files() {
        let text = path.to_string_lossy().replace('\\', "/");
        if !text.contains("@std/") {
            continue;
        }
        files += 1;
        if let Some(dir) = path.parent()
            && let Some(lines) = manifest_lines(dir)
            && let Some(pkg) = lines.first()
        {
            pkgs.insert(pkg.clone());
        }
    }
    Some((files, pkgs.len()))
}

pub fn precompile_scope(scope_root: &Path) -> PrecompileReport {
    let mut report = PrecompileReport {
        projects: 0,
        entries: 0,
        populated: 0,
    };
    if !crate::depcache::dep_cache_enabled() {
        return report;
    }
    let manifest = match crate::project::load_manifest(scope_root) {
        Ok(Some(manifest)) => manifest,
        _ => return report,
    };
    let mut projects: Vec<(PathBuf, crate::project::ProjectConfig)> = Vec::new();
    match manifest.workspace {
        Some(workspace) => {
            let members = match crate::project::resolve_workspace_members(scope_root, &workspace) {
                Ok(members) => members,
                Err(_) => return report,
            };
            let mut names: Vec<&String> = members.keys().collect();
            names.sort();
            for name in names {
                if let Some((root, cfg)) = members.get(name) {
                    projects.push((root.clone(), cfg.clone()));
                }
            }
        }
        None => {
            if let Some(cfg) = manifest.project {
                projects.push((scope_root.to_path_buf(), cfg));
            }
        }
    }
    let populated_before = PROD_POPULATED.load(Ordering::Relaxed);
    for (root, cfg) in &projects {
        report.projects += 1;
        let entry = cfg.main_path(root);
        if !entry.is_file() {
            continue;
        }
        if crate::modules::ModuleGraph::build(&entry).is_ok() {
            report.entries += 1;
        }
    }
    report.populated = PROD_POPULATED
        .load(Ordering::Relaxed)
        .saturating_sub(populated_before);
    report
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::depcache::{KIND_GIT, KIND_SEMVER, KIND_STD};

    fn sample(kind: &str, version: &str) -> DepProduct {
        DepProduct {
            pkg: "@acme/widget".to_string(),
            kind: kind.to_string(),
            spec: "semver:^1.0.0".to_string(),
            version_key: version.to_string(),
            toolchain: "abc123".to_string(),
            dep_set: "d".repeat(64),
            content: "e".repeat(64),
            root: "/tmp/x".to_string(),
            base: "https://example.com".to_string(),
            commit: String::new(),
            files: Vec::new(),
            edges: Vec::new(),
        }
    }

    fn elf(payload: &[u8]) -> Vec<u8> {
        let mut out = vec![0x7f, b'E', b'L', b'F'];
        out.resize(64, 0);
        out.extend_from_slice(payload);
        out
    }

    #[test]
    fn layout_matches_registry_conventions() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("playout");
        let dir = product_dir(
            KIND_SEMVER,
            "https://reg.example.com",
            "@acme/widget",
            "1.2.0",
            "tool",
        )
        .expect("layout");
        assert_eq!(
            dir,
            products_root()
                .join(crate::fetch::host_of("https://reg.example.com"))
                .join("@acme/widget")
                .join("1.2.0")
                .join("tool")
        );
        let cached =
            crate::fetch::cached_package_dir("https://reg.example.com", "@acme/widget", "1.2.0");
        assert_eq!(cached.file_name(), Some(dir.iter().nth_back(1).unwrap()));
        let git = product_dir(KIND_GIT, "", "mydep", "deadbeef", "tool").expect("git layout");
        assert_eq!(git.iter().nth_back(3).unwrap(), GIT_SCOPE);
    }

    #[test]
    fn layout_never_escapes() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("pescape");
        assert_eq!(
            product_dir(KIND_SEMVER, "https://x.example", "../../etc", "1.0.0", "t"),
            None
        );
        assert_eq!(
            product_dir(KIND_SEMVER, "https://x.example", "pkg", "../t", "t"),
            None
        );
        assert_eq!(
            product_dir(KIND_SEMVER, "https://x.example", "pkg", "1.0.0", ".."),
            None
        );
        let empty_pkg =
            product_dir(KIND_SEMVER, "https://x.example", "", "1.0.0", "t").expect("fallback");
        assert_eq!(empty_pkg.file_name().unwrap(), "t");
        assert_eq!(empty_pkg.iter().nth_back(2).unwrap(), "pkg");
        assert_eq!(product_dir("path", "", "pkg", "1.0.0", "t"), None);
        assert_eq!(product_dir("workspace", "", "pkg", "1.0.0", "t"), None);
        assert_eq!(product_dir("bogus", "", "pkg", "1.0.0", "t"), None);
    }

    #[test]
    fn exact_pin_gates_floating_ranges() {
        assert!(exact_pin(KIND_SEMVER, "1.2.0"));
        assert!(!exact_pin(KIND_SEMVER, "^1.0.0"));
        assert!(!exact_pin(KIND_SEMVER, "~1.2.0"));
        assert!(!exact_pin(KIND_SEMVER, "*"));
        assert!(!exact_pin(KIND_SEMVER, "latest"));
        assert!(!exact_pin(KIND_SEMVER, ">=1.0.0"));
        assert!(!exact_pin(KIND_SEMVER, ""));
        assert!(exact_pin(KIND_STD, "0.5.1"));
        assert!(!exact_pin(KIND_STD, "^0.5.1"));
        assert!(exact_pin(KIND_GIT, "deadbeef"));
        assert!(!exact_pin(KIND_GIT, ""));
        assert!(!exact_pin("path", "1.0.0"));
    }

    #[test]
    fn mirror_roundtrip_with_manifest() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("pmirror");
        reset_stats();
        let product = sample(KIND_SEMVER, "1.2.0");
        assert!(mirror_product(&product));
        let dir = product_dir(KIND_SEMVER, &product.base, &product.pkg, "1.2.0", "abc123").unwrap();
        let files = scan_product_files(&dir);
        assert_eq!(files.len(), 1);
        assert_eq!(read_product_file(&files[0]), Some(product.clone()));
        let lines = manifest_lines(&dir).expect("manifest");
        assert_eq!(lines.len(), 9);
        assert_eq!(lines[0], "@acme/widget");
        assert_eq!(lines[3], "1.2.0");
        assert_eq!(lines[5], "d".repeat(64));
        assert_eq!(stats().2, 1);
        assert!(mirror_product(&product));
        assert_eq!(stats().2, 1);
        let floating = sample(KIND_SEMVER, "^1.0.0");
        assert!(!mirror_product(&floating));
        assert_eq!(stats().2, 1);
    }

    #[test]
    fn corrupt_entry_falls_back_and_heals() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("pheal");
        reset_stats();
        let product = sample(KIND_SEMVER, "1.2.0");
        assert!(mirror_product(&product));
        let dir = product_dir(KIND_SEMVER, &product.base, &product.pkg, "1.2.0", "abc123").unwrap();
        let path = scan_product_files(&dir).pop().unwrap();
        std::fs::write(&path, b"not a product").unwrap();
        assert_eq!(read_product_file(&path), None);
        assert_eq!(stats().1, 1);
        assert!(mirror_product(&product));
        assert_eq!(read_product_file(&path), Some(product));
    }

    #[test]
    fn key_changes_invalidate_by_construction() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("pkey");
        reset_stats();
        assert!(mirror_product(&sample(KIND_SEMVER, "1.2.0")));
        let other_toolchain = product_dir(
            KIND_SEMVER,
            "https://example.com",
            "@acme/widget",
            "1.2.0",
            "zzz999",
        )
        .unwrap();
        assert!(scan_product_files(&other_toolchain).is_empty());
        let other_version = product_dir(
            KIND_SEMVER,
            "https://example.com",
            "@acme/widget",
            "2.0.0",
            "abc123",
        )
        .unwrap();
        assert!(scan_product_files(&other_version).is_empty());
        let other_host = product_dir(
            KIND_SEMVER,
            "https://other.example",
            "@acme/widget",
            "1.2.0",
            "abc123",
        )
        .unwrap();
        assert!(scan_product_files(&other_host).is_empty());
    }

    #[test]
    fn global_objects_roundtrip_byte_identical() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("pobj");
        reset_stats();
        let bytes = elf(b"object-bytes");
        assert!(write_global_object("release/m/t", &bytes));
        assert_eq!(read_global_object("release/m/t"), Some(bytes.clone()));
        assert_eq!(stats().3, 1);
        assert_eq!(global_object_path("../evil"), None);
        assert!(!write_global_object("../evil", &bytes));
        assert_eq!(read_global_object("../evil"), None);
        let dest = global_object_path("release/m/bad").unwrap();
        std::fs::create_dir_all(dest.parent().unwrap()).unwrap();
        std::fs::write(&dest, b"not an object").unwrap();
        assert_eq!(read_global_object("release/m/bad"), None);
        assert_eq!(read_global_object("release/m/missing"), None);
    }

    #[test]
    fn object_lru_accounting() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("pobjlru");
        assert!(write_global_object("release/m/one", &elf(b"12345678")));
        assert!(write_global_object(
            "release/m/two",
            &elf(b"1234567890123456")
        ));
        let usage = global_objects_usage_bytes();
        assert_eq!(usage, (64 + 8) + (64 + 16));
        assert_eq!(prune_global_objects_lru(u64::MAX), 0);
        assert_eq!(global_objects_usage_bytes(), usage);
        assert!(prune_global_objects_lru(usage - 1) > 0);
        assert!(global_objects_usage_bytes() <= usage - 1);
    }

    #[test]
    fn miss_path_never_fails() {
        let (_guard, _dir) = crate::fetch::testkit::isolate_cache("pmiss");
        reset_stats();
        assert_eq!(
            read_product_file(Path::new("/nonexistent/product-abc.bin")),
            None
        );
        assert!(scan_product_files(Path::new("/nonexistent")).is_empty());
        assert!(product_files().is_empty());
        assert_eq!(products_usage(), (0, 0));
        assert_eq!(global_objects_usage_bytes(), 0);
        assert_eq!(prune_global_objects_lru(0), 0);
        assert_eq!(manifest_lines(Path::new("/nonexistent")), None);
        let odd = sample("path", "1.0.0");
        assert!(!mirror_product(&odd));
        assert_eq!(product_file(Path::new("/x"), ""), None);
    }
}
