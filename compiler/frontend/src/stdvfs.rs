use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Mutex;

static VFS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());

// TODO: lower this once account management is real and the analyzer can
// stream a package instead of holding the whole tree in memory.
pub const MAX_STD_MODULE_BYTES: usize = 5 * 1024 * 1024;

pub fn pin_version() -> String {
    env!("CARGO_PKG_VERSION").to_string()
}

pub fn normalize_rest(name: &str) -> String {
    let rest = name
        .strip_prefix("@std/")
        .or_else(|| name.strip_prefix("std/"))
        .unwrap_or(name);
    rest.to_string()
}

fn vfs() -> std::sync::MutexGuard<'static, BTreeMap<String, String>> {
    VFS.lock().unwrap_or_else(|e| e.into_inner())
}

pub fn set(rest: &str, source: String) {
    vfs().insert(normalize_rest(rest), source);
}

pub fn get(rest: &str) -> Option<String> {
    vfs().get(&normalize_rest(rest)).cloned()
}

pub fn contains(rest: &str) -> bool {
    vfs().contains_key(&normalize_rest(rest))
}

pub fn list() -> Vec<String> {
    vfs().keys().cloned().collect()
}

pub fn clear() {
    vfs().clear();
}

#[cfg(target_arch = "wasm32")]
pub fn is_known_module(rest: &str) -> bool {
    !normalize_rest(rest).is_empty()
}

#[cfg(not(target_arch = "wasm32"))]
pub fn is_known_module(rest: &str) -> bool {
    stdlib::MODULES.contains(&normalize_rest(rest).as_str())
        || vendored_module(rest).is_some()
}

fn vendored_module(rest: &str) -> Option<std::path::PathBuf> {
    let key = normalize_rest(rest);
    if key.is_empty() {
        return None;
    }
    let (pkg, sub) = match key.split_once('/') {
        Some((top, sub)) => (format!("@std/{top}"), Some(sub.to_string())),
        None => (format!("@std/{key}"), None),
    };
    let anchor = std::env::current_dir()
        .ok()
        .map(|cwd| crate::fetch::cache_anchor(&cwd))?;
    let dir = crate::fetch::local_override_dir(&pkg, &anchor)?;
    if let Some(sub) = sub
        && module_candidates(&dir, Some(&sub))
            .iter()
            .all(|path| !path.is_file())
    {
        return None;
    }
    Some(dir)
}

pub fn std_source(rest: &str) -> Option<String> {
    std_source_result(rest).ok().flatten()
}

pub fn std_source_result(rest: &str) -> Result<Option<String>, Diagnostic> {
    let key = normalize_rest(rest);
    if let Some(src) = vfs().get(&key).cloned() {
        return Ok(Some(src));
    }
    if cfg!(target_arch = "wasm32") {
        return Ok(None);
    }
    cached_std_source_result(&key)
}

enum LocalModule {
    Absent,
    Missing(PathBuf),
    Source(String),
}

fn vendor_std_module(pkg: &str, sub: Option<&str>) -> LocalModule {
    let anchor = std::env::current_dir()
        .map(|cwd| crate::fetch::cache_anchor(&cwd))
        .unwrap_or_else(|_| PathBuf::from("."));
    let version = crate::stdlib_seed::unpinned_std_version_for(Some(&anchor), pkg);
    let Some(dir) = crate::fetch::local_override_dir(pkg, &anchor) else {
        return LocalModule::Absent;
    };
    let explicit = std::env::var(crate::fetch::LOCAL_VENDOR_ENV)
        .is_ok_and(|v| !v.trim().is_empty());
    let ok = if explicit {
        crate::fetch::check_local_override_silent(pkg, &dir, "*", None)
    } else {
        crate::fetch::check_local_override_silent(pkg, &dir, &version, Some(&version))
    };
    if !ok {
        return LocalModule::Missing(dir);
    }
    let canonical = std::fs::canonicalize(&dir).unwrap_or_else(|_| dir.clone());
    match read_tree_module(&canonical, sub) {
        Some(src) => {
            crate::fetch::note_local_override(pkg, &version, &canonical);
            LocalModule::Source(src)
        }
        None => LocalModule::Missing(canonical),
    }
}

fn project_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let anchor = crate::fetch::cache_anchor(&cwd);
    let pin = crate::stdlib_seed::read_project_pin(&anchor)?;
    let version = pin.packages.get(pkg)?;
    let dir = crate::fetch::project_package_dir(&anchor, &pin.registry, pkg, version);
    if crate::fetch::cached_integrity(&dir).is_none() {
        return None;
    }
    read_tree_module(&dir, sub)
}

fn cached_std_source_result(key: &str) -> Result<Option<String>, Diagnostic> {
    let (pkg, sub) = match key.split_once('/') {
        Some((top, s)) => (format!("@std/{top}"), Some(s)),
        None => (format!("@std/{key}"), None),
    };
    match vendor_std_module(&pkg, sub) {
        LocalModule::Source(src) => return Ok(Some(src)),
        LocalModule::Missing(dir) => {
            let path = module_candidates(&dir, sub)
                .into_iter()
                .map(|p| format!("`{}`", p.display()))
                .collect::<Vec<_>>()
                .join(", ");
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "local source for `{pkg}` at `{}` cannot provide module `@std/{key}`",
                    dir.display()
                ),
            )
            .with_hint(format!(
                "check the local package layout ({path}) and its Project.config, or unset RNX_VENDOR_DIR to use the registry cache"
            )));
        }
        LocalModule::Absent => {}
    }
    if !stdlib::MODULES.contains(&key) {
        return Ok(None);
    }
    if let Some(src) = project_std_source(&pkg, sub) {
        return Ok(Some(src));
    }
    if let Some(pin) = crate::stdlib_seed::read_std_pin()
        && let Some(version) = pin.packages.get(&pkg)
        && let Some(dir) = crate::fetch::find_cached_package_dir(&pin.registry, &pkg, version)
        && let Some(src) = read_tree_module(&dir, sub)
    {
        return Ok(Some(src));
    }
    if let Some(src) = find_scanned_project_std_source(&pkg, sub) {
        return Ok(Some(src));
    }
    Ok(find_scanned_std_source(&pkg, sub))
}

fn module_candidates(dir: &Path, sub: Option<&str>) -> Vec<PathBuf> {
    match sub {
        None => vec![
            dir.join("lib.rnx"),
            dir.join("src").join("lib.rnx"),
            dir.join("main.rnx"),
            dir.join("src").join("main.rnx"),
        ],
        Some(s) if valid_subpath(s) => {
            let mut out = Vec::new();
            for base in [dir.to_path_buf(), dir.join("src")] {
                out.push(base.join(format!("{s}.rnx")));
                out.push(base.join(s).join("lib.rnx"));
                out.push(base.join(s).join("mod.rnx"));
                out.push(base.join(s).join("index.rnx"));
            }
            out
        }
        Some(_) => Vec::new(),
    }
}

fn valid_subpath(sub: &str) -> bool {
    !sub.is_empty()
        && Path::new(sub)
            .components()
            .all(|c| matches!(c, Component::Normal(_)))
}

fn read_tree_module(dir: &Path, sub: Option<&str>) -> Option<String> {
    module_candidates(dir, sub)
        .into_iter()
        .find_map(|path| read_module_file(&path))
}

fn read_module_file(path: &Path) -> Option<String> {
    let meta = std::fs::metadata(path).ok()?;
    if !meta.is_file() || meta.len() == 0 || meta.len() > MAX_STD_MODULE_BYTES as u64 {
        return None;
    }
    let bytes = std::fs::read(path).ok()?;
    if bytes.is_empty() || bytes.len() > MAX_STD_MODULE_BYTES {
        return None;
    }
    String::from_utf8(bytes).ok()
}

fn find_scanned_project_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let cwd = std::env::current_dir().ok()?;
    let anchor = crate::fetch::cache_anchor(&cwd);
    let root = crate::fetch::project_registry_root(&anchor);
    let hosts = std::fs::read_dir(&root).ok()?;
    let mut best: Option<(SemKey, PathBuf)> = None;
    for host in hosts.flatten() {
        let pkg_dir = host.path().join(pkg);
        let versions = match std::fs::read_dir(&pkg_dir) {
            Ok(v) => v,
            Err(_) => continue,
        };
        for entry in versions.flatten() {
            let dir = entry.path();
            if !dir.is_dir() {
                continue;
            }
            if !dir.join(crate::project::MANIFEST_FILE).is_file() {
                continue;
            }
            if crate::fetch::cached_integrity(&dir).is_none() {
                continue;
            }
            let name = entry.file_name().to_string_lossy().into_owned();
            let Some(next) = sem_key(&name) else {
                continue;
            };
            let replace = match &best {
                None => true,
                Some((cur, _)) => next > *cur,
            };
            if replace {
                best = Some((next, dir));
            }
        }
    }
    let (_, dir) = best?;
    read_tree_module(&dir, sub)
}

fn find_scanned_std_source(pkg: &str, sub: Option<&str>) -> Option<String> {
    let mut best: Option<(SemKey, PathBuf)> = None;
    for have in crate::fetch::store_have() {
        if have.full != pkg {
            continue;
        }
        let Some(next) = sem_key(&have.version) else {
            continue;
        };
        let replace = match &best {
            None => true,
            Some((cur, _)) => next > *cur,
        };
        if replace && let Some(dir) = crate::fetch::store_pkg_dir(&have.integrity) {
            best = Some((next, dir));
        }
    }
    let (_, dir) = best?;
    read_tree_module(&dir, sub)
}

type SemKey = (u64, u64, u64, String);

fn sem_key(version: &str) -> Option<SemKey> {
    let (core, pre) = match version.split_once('-') {
        Some((c, p)) => (c, p.to_string()),
        None => (version, String::new()),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    Some((major, minor, patch, pre))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::testkit;

    struct Guard {
        _cache: testkit::EnvGuard,
    }
    impl Drop for Guard {
        fn drop(&mut self) {
            clear();
        }
    }

    fn isolated(tag: &str) -> Guard {
        let (guard, _dir) = testkit::isolate_cache(tag);
        clear();
        Guard { _cache: guard }
    }

    #[test]
    fn vfs_round_trips_and_normalizes_prefixes() {
        let _g = isolated("vfsnorm");
        set("@std/fs", "export fn hello(): Int { return 1; }\n".to_string());
        assert_eq!(
            get("fs"),
            Some("export fn hello(): Int { return 1; }\n".to_string())
        );
        assert_eq!(
            get("std/fs"),
            Some("export fn hello(): Int { return 1; }\n".to_string())
        );
        assert!(contains("@std/fs"));
        assert_eq!(list(), vec!["fs".to_string()]);
    }

    #[test]
    fn std_source_prefers_vfs_over_cache() {
        let _g = isolated("vfspref");
        set("prelude", "vfs-bytes".to_string());
        assert_eq!(std_source("prelude"), Some("vfs-bytes".to_string()));
        assert_eq!(std_source("@std/prelude"), Some("vfs-bytes".to_string()));
    }

    #[test]
    fn std_source_misses_without_cache() {
        let _g = isolated("stdmiss");
        assert!(!contains("prelude"));
        if cfg!(target_arch = "wasm32") {
            assert_eq!(std_source("prelude"), None);
        } else {
            assert_eq!(std_source("prelude"), None);
            assert_eq!(std_source("@std/nope"), None);
        }
    }

    #[test]
    fn std_source_serves_seeded_cache() {
        let _g = isolated("stdseeded");
        if cfg!(target_arch = "wasm32") {
            return;
        }
        let pin = env!("CARGO_PKG_VERSION");
        let mut chunks = BTreeMap::new();
        let mut nodes = Vec::new();
        for full in crate::stdlib_seed::std_package_names() {
            let (manifest, pairs, digest, _pc, _main) = testkit::fixture_tree(&full, pin);
            nodes.push(testkit::node_json(&full, pin, &digest, false));
            chunks.insert(format!("{full}@{pin}"), (manifest, pairs));
        }
        let body = testkit::resolve_json_static(&nodes);
        let server = testkit::MockRegistry::start_dynamic(chunks, move |_| body.clone());
        let cfg = testkit::registry_cfg(&server.base);
        let report = crate::stdlib_seed::seed_stdlib_cache(Some(&cfg), &BTreeMap::new()).unwrap();
        assert!(!report.packages.is_empty());
        let src = std_source("prelude").expect("seeded prelude serves from cache");
        assert!(src.contains("export fn hello"));
        let fs = std_source("@std/fs").expect("seeded fs serves from cache");
        assert!(fs.contains("export fn hello"));
    }

    fn write_vendor_package(
        root: &Path,
        name: &str,
        pkg: &str,
        layout: &str,
        modules: &[(&str, &str)],
    ) {
        let dir = root.join(name);
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            format!(
                "export default {{\n    project: {{ name: \"{pkg}\", version: \"0.2.1\" }}\n}}\n"
            ),
        )
        .unwrap();
        for (module, body) in modules {
            let path = dir.join(layout).join(format!("{module}.rnx"));
            if let Some(parent) = path.parent() {
                std::fs::create_dir_all(parent).unwrap();
            }
            std::fs::write(path, body).unwrap();
        }
    }

    fn write_store_package(
        cache: &Path,
        pkg: &str,
        version: &str,
        modules: &[(&str, &str)],
        hex: char,
    ) {
        let integrity: String = std::iter::repeat_n(hex, 64).collect();
        let dir = cache.join("store").join("pkgs").join(&integrity);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            format!(
                "export default {{\n    project: {{ name: \"{pkg}\", version: \"{version}\" }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            dir.join(".rnx-integrity"),
            format!("{pkg}\n{version}\n{integrity}\n"),
        )
        .unwrap();
        for (module, body) in modules {
            std::fs::write(dir.join("src").join(format!("{module}.rnx")), body).unwrap();
        }
    }

    #[test]
    fn vendor_root_lib_wins_over_cached_package() {
        let (_guard, cache) = testkit::isolate_cache("vendorrootlib");
        clear();
        write_store_package(
            &cache,
            "@std/prelude",
            "0.2.1",
            &[("lib", "cache prelude\n")],
            'a',
        );
        let tree = crate::scratch::dir("vendor-root-lib");
        write_vendor_package(
            &tree,
            "prelude",
            "@std/prelude",
            "",
            &[("lib", "vendor prelude\n")],
        );
        let _vendor = testkit::set_vendor_dir(&tree);

        assert_eq!(std_source("prelude"), Some("vendor prelude\n".to_string()));
    }

    #[test]
    fn vendor_src_lib_wins_over_cached_package() {
        let (_guard, cache) = testkit::isolate_cache("vendorsrclib");
        clear();
        write_store_package(
            &cache,
            "@std/prelude",
            "0.2.1",
            &[("lib", "cache prelude\n")],
            'b',
        );
        let tree = crate::scratch::dir("vendor-src-lib");
        write_vendor_package(
            &tree,
            "prelude",
            "@std/prelude",
            "src",
            &[("lib", "vendor src prelude\n")],
        );
        let _vendor = testkit::set_vendor_dir(&tree);

        assert_eq!(
            std_source("prelude"),
            Some("vendor src prelude\n".to_string())
        );
    }

    #[test]
    fn vendor_nested_module_wins_over_cached_package() {
        let (_guard, cache) = testkit::isolate_cache("vendornested");
        clear();
        write_store_package(
            &cache,
            "@std/net",
            "0.2.1",
            &[("lib", "cache net\n"), ("http", "cache http\n")],
            'c',
        );
        let tree = crate::scratch::dir("vendor-nested");
        write_vendor_package(
            &tree,
            "net",
            "@std/net",
            "",
            &[("lib", "vendor net\n"), ("http", "vendor http\n")],
        );
        let _vendor = testkit::set_vendor_dir(&tree);

        assert_eq!(std_source("net/http"), Some("vendor http\n".to_string()));
    }

    #[test]
    fn cache_still_serves_without_a_vendor_directory() {
        let (_guard, cache) = testkit::isolate_cache("novendorcache");
        clear();
        write_store_package(
            &cache,
            "@std/prelude",
            "0.2.1",
            &[("lib", "cache prelude\n")],
            'd',
        );
        write_store_package(
            &cache,
            "@std/net",
            "0.2.1",
            &[("lib", "cache net\n"), ("http", "cache http\n")],
            'e',
        );

        assert_eq!(std_source("prelude"), Some("cache prelude\n".to_string()));
        assert_eq!(std_source("net/http"), Some("cache http\n".to_string()));
    }

    #[test]
    fn vendor_override_miss_does_not_fall_back_to_cache() {
        let (_guard, cache) = testkit::isolate_cache("vendormiss");
        clear();
        write_store_package(
            &cache,
            "@std/prelude",
            "0.2.1",
            &[("lib", "cache prelude\n")],
            'f',
        );
        let tree = crate::scratch::dir("vendor-miss");
        write_vendor_package(&tree, "prelude", "@std/prelude", "", &[]);
        let _vendor = testkit::set_vendor_dir(&tree);

        let err =
            std_source_result("prelude").expect_err("missing vendor module must not hit the cache");
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("@std/prelude"), "{}", err.message);
        assert_eq!(std_source("prelude"), None);
    }

    #[test]
    fn known_modules_match_the_manifest() {
        assert!(is_known_module("prelude"));
        assert!(is_known_module("@std/net/http"));
        assert!(!is_known_module("nope"));
        assert!(!is_known_module(""));
    }

    #[test]
    fn pin_version_matches_frontend_package() {
        assert_eq!(pin_version(), env!("CARGO_PKG_VERSION"));
    }
}
