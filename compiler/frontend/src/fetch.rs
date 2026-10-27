use crate::checksum::Sha256;
use crate::project::{self, ProjectConfig, RegistryConfig};
use diagnostics::{Code, Diagnostic};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const REGISTRY_SPEC: u32 = 0;
pub const DEFAULT_REGISTRY_DOMAIN: &str = "rasmalai.rovelstars.com";
pub const LOCAL_VENDOR_ENV: &str = "RNX_VENDOR_DIR";
pub const VENDOR_DIR: &str = ".vendor";
const RESOLVE_INLINE_CHUNKS: &str = "resolve-inline-chunks";
const REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_DOWNLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 1024 * 1024;
const MAX_CHUNK_ENTRIES: usize = 4096;
const CHUNK_FETCH_CONCURRENCY: usize = 8;
const PACKAGE_FETCH_CONCURRENCY: usize = 16;
const GLOBAL_CHUNK_INFLIGHT: usize = 32;
const POOL_MAX_PER_HOST: usize = 16;

pub fn cache_anchor(proj_root: &Path) -> PathBuf {
    project::find_workspace_root_strict(proj_root).unwrap_or_else(|| proj_root.to_path_buf())
}

fn override_marks() -> &'static std::sync::Mutex<BTreeSet<String>> {
    static MARKS: std::sync::OnceLock<std::sync::Mutex<BTreeSet<String>>> =
        std::sync::OnceLock::new();
    MARKS.get_or_init(|| std::sync::Mutex::new(BTreeSet::new()))
}

pub(crate) fn note_local_override(full: &str, version: &str, dir: &Path) {
    let key = format!("{full}@{version} from {}", dir.display());
    let mut marks = override_marks().lock().unwrap_or_else(|e| e.into_inner());
    if marks.insert(key) {
        eprintln!(
            "warning: using local source for `{full}@{version}` from `{}` (registry cache overridden)",
            dir.display()
        );
    }
}

pub fn local_override_used(full: &str) -> bool {
    let marks = override_marks().lock().unwrap_or_else(|e| e.into_inner());
    marks.iter().any(|k| k.starts_with(&format!("{full}@")))
}

pub fn local_override_dir(pkg: &str, anchor: &Path) -> Option<PathBuf> {
    if let Some(dir) = std::env::var(LOCAL_VENDOR_ENV)
        .ok()
        .filter(|s| !s.trim().is_empty())
    {
        let candidate = PathBuf::from(dir).join(pkg);
        if candidate
            .join(crate::project::MANIFEST_FILE)
            .is_file()
        {
            return Some(candidate);
        }
    }
    let candidate = anchor.join(VENDOR_DIR).join(pkg);
    if candidate
        .join(crate::project::MANIFEST_FILE)
        .is_file()
    {
        return Some(candidate);
    }
    None
}

fn check_local_override(
    pkg: &str,
    dir: &Path,
    range: &str,
    pinned: Option<&str>,
) -> Result<(ProjectConfig, String), Diagnostic> {
    let cfg = ProjectConfig::load_from_dir(dir)?.ok_or_else(|| {
        Diagnostic::new(
            Code::E108,
            format!(
                "local source for `{pkg}` at `{}` has no Project.config",
                dir.display()
            ),
        )
    })?;
    if cfg.name != pkg {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "local source at `{}` names package `{}` but `{pkg}` was requested",
                dir.display(),
                cfg.name
            ),
        ));
    }
    if cfg.artifacts.is_some() {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "local source for `{pkg}@{}` declares `artifacts`; vendored OCI artifacts are not supported",
                cfg.version
            ),
        )
        .with_hint("resolve the package from the registry or drop the `artifacts` table from the vendored Project.config"));
    }
    if !is_valid_range(range) {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "requirement for `{pkg}` is not a valid range (*, latest, X.Y.Z, ^, ~, >=)"
            ),
        ));
    }
    let exact = crate::modules::exact_version(range);
    let pin = pinned.or(exact.as_deref());
    match pin {
        Some(want) => {
            if cfg.version != want {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "local source for `{pkg}` is version `{}` but `{want}` is required",
                        cfg.version
                    ),
                )
                .with_hint("update the vendored source or the required version so they agree"));
            }
        }
        None => {
            if !engine_satisfied(range, &cfg.version) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "local source for `{pkg}` is version `{}` which does not satisfy `{range}`",
                        cfg.version
                    ),
                )
                .with_hint("update the vendored source or the required range so they agree"));
            }
        }
    }
    let version = cfg.version.clone();
    Ok((cfg, version))
}

pub(crate) fn check_local_override_silent(
    pkg: &str,
    dir: &Path,
    range: &str,
    pinned: Option<&str>,
) -> bool {
    check_local_override(pkg, dir, range, pinned).is_ok()
}

pub fn resolve_local_override(
    pkg: &str,
    range: &str,
    pinned: Option<&str>,
    anchor: &Path,
) -> Result<Option<(PathBuf, ProjectConfig, String)>, Diagnostic> {
    let Some(dir) = local_override_dir(pkg, anchor) else {
        return Ok(None);
    };
    let (cfg, version) = check_local_override(pkg, &dir, range, pinned)?;
    let canonical = std::fs::canonicalize(&dir).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("cannot read `{}`: {e}", dir.display()),
        )
    })?;
    note_local_override(pkg, &version, &canonical);
    Ok(Some((canonical, cfg, version)))
}

pub fn short_rev(rev: &str) -> String {
    let kept: String = rev
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || *c == '.' || *c == '_' || *c == '-')
        .take(12)
        .collect();
    if kept.is_empty() {
        "rev".to_string()
    } else {
        kept
    }
}

fn git_cmd(args: &[&str], cwd: &Path) -> Result<String, Diagnostic> {
    let out = std::process::Command::new("git")
        .args(args)
        .current_dir(cwd)
        .output()
        .map_err(|_| Diagnostic::new(Code::E108, "git is not installed or not in PATH"))?;
    if !out.status.success() {
        let tail = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(Diagnostic::new(
            Code::E108,
            format!("git {} failed: {tail}", args.join(" ")),
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).trim().to_string())
}

fn marker_matches(dir: &Path, git_url: &str, rev: &str) -> bool {
    let text = match std::fs::read_to_string(dir.join(".rnx-fetch")) {
        Ok(t) => t,
        Err(_) => return false,
    };
    let mut lines = text.lines();
    lines.next() == Some(git_url) && lines.next() == Some(rev)
}

fn write_marker(dir: &Path, git_url: &str, rev: &str) -> Result<(), Diagnostic> {
    let commit = git_cmd(&["rev-parse", "HEAD"], dir)?;
    std::fs::write(dir.join(".rnx-fetch"), format!("{git_url}\n{rev}\n{commit}\n")).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dir.display()))
    })
}

pub fn fetch_git_dependency(
    name: &str,
    git_url: &str,
    rev: &str,
    cache_dir: &Path,
) -> Result<PathBuf, Diagnostic> {
    let dir = cache_dir.join(format!("{}-{}", name, short_rev(rev)));
    if dir.join(project::MANIFEST_FILE).is_file() && marker_matches(&dir, git_url, rev) {
        return Ok(dir);
    }
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", parent.display()))
        })?;
    }
    let shallow = std::process::Command::new("git")
        .args(["clone", "--depth", "1", "--branch", rev, git_url])
        .arg(&dir)
        .output()
        .map_err(|_| Diagnostic::new(Code::E108, "git is not installed or not in PATH"))?;
    if !shallow.status.success() {
        git_cmd(&["clone", git_url, &dir.to_string_lossy()], cache_dir)?;
        git_cmd(&["checkout", rev], &dir).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot reach revision `{rev}` of `{git_url}`: {}", e.message),
            )
        })?;
    }
    if !dir.join(project::MANIFEST_FILE).is_file() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("package `{name}` from `{git_url}` has no Project.config"),
        ));
    }
    write_marker(&dir, git_url, rev)?;
    Ok(dir)
}

pub fn resolve_git_dep(
    anchor: &Path,
    name: &str,
    git_url: &str,
    rev: &str,
) -> Result<PathBuf, Diagnostic> {
    let vendor = anchor.join(VENDOR_DIR).join(name);
    if vendor.join(project::MANIFEST_FILE).is_file() {
        return std::fs::canonicalize(&vendor).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", vendor.display()))
        });
    }
    let dir = fetch_git_dependency(name, git_url, rev, &anchor.join(".rnx-cache").join("cache").join("git"))?;
    std::fs::canonicalize(&dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", dir.display())))
}

fn excluded_dir(name: &str) -> bool {
    matches!(name, ".git" | "target" | "tests" | ".rnx-cache")
}

fn copy_tree(from: &Path, to: &Path) -> Result<(), Diagnostic> {
    std::fs::create_dir_all(to)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", to.display())))?;
    let mut entries: Vec<PathBuf> = Vec::new();
    for entry in std::fs::read_dir(from)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", from.display())))?
    {
        let entry = entry.map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", from.display()))
        })?;
        entries.push(entry.path());
    }
    entries.sort();
    for path in entries {
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let ft = std::fs::symlink_metadata(&path).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display()))
        })?;
        if ft.file_type().is_symlink() {
            continue;
        }
        let dest = to.join(&name);
        if ft.file_type().is_dir() {
            if !excluded_dir(&name) {
                copy_tree(&path, &dest)?;
            }
        } else if ft.file_type().is_file() {
            if name == ".rnx-fetch" {
                continue;
            }
            std::fs::copy(&path, &dest).map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot copy `{}`: {e}", path.display()))
            })?;
        }
    }
    Ok(())
}

pub fn fetch_all_git_deps(scope_root: &Path) -> Result<Vec<(String, String, String)>, Diagnostic> {
    let manifest = project::load_manifest(scope_root)?.ok_or_else(|| {
        Diagnostic::new(Code::E108, "No file specified and no Project.config found")
    })?;
    let mut starts: Vec<(PathBuf, ProjectConfig)> = Vec::new();
    match manifest.workspace {
        Some(ws) => {
            let members = project::resolve_workspace_members(scope_root, &ws)?;
            starts.extend(members.into_values());
        }
        None => {
            let cfg = manifest.project.ok_or_else(|| {
                Diagnostic::new(Code::E108, "No file specified and no Project.config found")
            })?;
            starts.push((scope_root.to_path_buf(), cfg));
        }
    }
    let mut seen: BTreeMap<String, (String, String)> = BTreeMap::new();
    let mut stack: Vec<(PathBuf, ProjectConfig)> = starts;
    while let Some((pkg_root, cfg)) = stack.pop() {
        for (dep_name, spec) in &cfg.dependencies {
            let crate::project::DependencySpec::Git { git, rev, .. } = spec else {
                continue;
            };
            if let Some((url, old_rev)) = seen.get(dep_name) {
                if url != git || old_rev != rev {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("conflicting git sources for package `{dep_name}`"),
                    ));
                }
                continue;
            }
            let anchor = cache_anchor(&pkg_root);
            let dir = resolve_git_dep(&anchor, dep_name, git, rev)?;
            seen.insert(dep_name.clone(), (git.clone(), rev.clone()));
            let dep_cfg = ProjectConfig::load_from_dir(&dir)?.ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("package `{dep_name}` has no Project.config"))
            })?;
            stack.push((dir, dep_cfg));
        }
    }
    let mut out: Vec<(String, String, String)> = seen
        .into_iter()
        .map(|(name, (url, rev))| (name, url, rev))
        .collect();
    out.sort();
    Ok(out)
}

pub fn vendor_all_in(scope_root: &Path, dest: &Path) -> Result<Vec<String>, Diagnostic> {
    let fetched = fetch_all_git_deps(scope_root)?;
    let mut names: Vec<String> = Vec::new();
    for (name, url, rev) in &fetched {
        let out = dest.join(name);
        let _ = std::fs::remove_dir_all(&out);
        let src = resolve_git_dep(scope_root, name, url, rev)?;
        copy_tree(&src, &out)?;
        names.push(name.clone());
    }
    names.sort();
    Ok(names)
}

pub fn default_vendor_dir(scope_root: &Path) -> PathBuf {
    scope_root.join(VENDOR_DIR)
}

fn vendor_copy(src: &Path, dest: &Path, name: &str) -> Result<(), Diagnostic> {
    let out = dest.join(name);
    let _ = std::fs::remove_dir_all(&out);
    if let Some(parent) = out.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", parent.display()))
        })?;
    }
    copy_tree(src, &out)
}

fn scope_dep_spec(scope_root: &Path, name: &str) -> Option<crate::project::DependencySpec> {
    let manifest = project::load_manifest(scope_root).ok()??;
    match manifest.workspace {
        Some(ws) => {
            let members = project::resolve_workspace_members(scope_root, &ws).ok()?;
            for (_, cfg) in members.values() {
                if let Some(spec) = cfg.dependencies.get(name) {
                    return Some(spec.clone());
                }
            }
            None
        }
        None => manifest.project?.dependencies.get(name).cloned(),
    }
}

pub fn find_stdlib_checkout(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    loop {
        let cand = dir.join("compiler").join("stdlib").join("src");
        if cand.join("prelude").join("lib.rnx").is_file() {
            return Some(cand);
        }
        if !dir.pop() {
            break;
        }
    }
    let compiled = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("stdlib")
        .join("src");
    if compiled.join("prelude").join("lib.rnx").is_file() {
        return Some(compiled);
    }
    None
}

fn best_cached_version(have: &[HaveEntry], name: &str, range: &str) -> Option<String> {
    let mut best: Option<((u64, u64, u64, String), String)> = None;
    for h in have {
        if h.full != name {
            continue;
        }
        if !engine_satisfied(range, &h.version) {
            continue;
        }
        let Some(v) = parse_semver(&h.version) else {
            continue;
        };
        let key = (v.0, v.1, v.2, v.3.clone());
        let take = match &best {
            None => true,
            Some((bk, _)) => {
                cmp_semver((key.0, key.1, key.2, key.3.as_str()), (bk.0, bk.1, bk.2, bk.3.as_str()))
                    == std::cmp::Ordering::Greater
            }
        };
        if take {
            best = Some((key, h.version.clone()));
        }
    }
    best.map(|(_, version)| version)
}

fn vendor_registry_package(
    scope_root: &Path,
    dest: &Path,
    name: &str,
    range: &str,
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
) -> Result<String, Diagnostic> {
    let anchor = cache_anchor(scope_root);
    let mut have = scan_cache_have();
    have.extend(scan_have_under(&project_registry_root(scope_root)));
    have.sort_by(|a, b| (&a.full, &a.version).cmp(&(&b.full, &b.version)));
    have.dedup_by(|a, b| a.full == b.full && a.version == b.version);
    let locked = crate::modules::locked_registry_version(scope_root, name);
    let pinned = locked.or_else(|| {
        if name.starts_with("@std/") {
            let pin_ver = crate::stdlib_seed::unpinned_std_version_for(Some(scope_root), name);
            if engine_satisfied(range, &pin_ver) {
                return Some(pin_ver);
            }
        }
        best_cached_version(&have, name, range)
    });
    let (dir, _cfg, version, staged) = match resolve_registry_package(
        name,
        range,
        default_registry,
        overrides,
        pinned,
        &have,
        Some(&anchor),
    ) {
        Ok((dir, cfg, version)) => (dir, cfg, version, false),
        Err(first) => {
            let Some(src_root) = find_stdlib_checkout(scope_root) else {
                return Err(first);
            };
            if !name.starts_with("@std/") {
                return Err(first);
            }
            let out = dest.join(name);
            let _ = std::fs::remove_dir_all(&out);
            match crate::stdlib_seed::stage_std_checkout_package(&src_root, name, &out) {
                Ok(version) => {
                    let cfg = ProjectConfig::load_from_dir(&out).map_err(|e| {
                        Diagnostic::new(Code::E108, format!("cannot stage `{name}`: {e}"))
                    })?.ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("package `{name}` has no Project.config"))
                    })?;
                    (out, cfg, version, true)
                }
                Err(_) => {
                    let _ = std::fs::remove_dir_all(&out);
                    return Err(first);
                }
            }
        }
    };
    if !staged {
        vendor_copy(&dir, dest, name)?;
    }
    Ok(format!("{name}@{version}"))
}

pub fn vendor_packages(
    scope_root: &Path,
    dest: &Path,
    names: &[String],
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
) -> Result<Vec<String>, Diagnostic> {
    if names.is_empty() {
        return vendor_all_in(scope_root, dest);
    }
    let mut done: Vec<String> = Vec::new();
    for raw in names {
        let parsed = crate::deplock::parse_update_selector(raw).map_err(|_| {
            Diagnostic::new(
                Code::E108,
                format!("bad vendor target `{raw}`: expected `name` or `name@range`"),
            )
        })?;
        let name = parsed.name;
        let range = parsed.selector.unwrap_or_else(|| "*".to_string());
        if !is_valid_range(&range) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("requirement for `{name}` is not a valid range (*, latest, X.Y.Z, ^, ~, >=)"),
            ));
        }
        if let Some(spec) = scope_dep_spec(scope_root, &name) {
            match spec {
                crate::project::DependencySpec::Path { path, .. } => {
                    let dir = if path.is_absolute() {
                        path
                    } else {
                        scope_root.join(path)
                    };
                    let cfg = ProjectConfig::load_from_dir(&dir)?.ok_or_else(|| {
                        Diagnostic::new(
                            Code::E108,
                            format!("local source for `{name}` at `{}` has no Project.config", dir.display()),
                        )
                    })?;
                    if cfg.name != name {
                        return Err(Diagnostic::new(
                            Code::E108,
                            format!("local source at `{}` names package `{}` but `{name}` was requested", dir.display(), cfg.name),
                        ));
                    }
                    vendor_copy(&dir, dest, &name)?;
                    done.push(format!("{name}@{}", cfg.version));
                    continue;
                }
                crate::project::DependencySpec::Git { git, rev, .. } => {
                    let src = resolve_git_dep(scope_root, &name, &git, &rev)?;
                    vendor_copy(&src, dest, &name)?;
                    done.push(format!("{name}#{rev}"));
                    continue;
                }
                _ => {}
            }
        }
        done.push(vendor_registry_package(scope_root, dest, &name, &range, default_registry, overrides)?);
    }
    done.sort();
    Ok(done)
}

pub fn project_registry_root(scope_root: &Path) -> PathBuf {
    cache_anchor(scope_root).join(crate::cache::PROJECT_CACHE_DIR).join("registry")
}

pub fn project_package_dir(scope_root: &Path, base: &str, full: &str, version: &str) -> PathBuf {
    package_dir_under(&project_registry_root(scope_root), base, full, version)
}

pub fn package_dir_under(root: &Path, base: &str, full: &str, version: &str) -> PathBuf {
    root.join(host_of(base)).join(sanitize_full(full)).join(sanitize_segment(version))
}

pub fn scan_have_under(root: &Path) -> Vec<HaveEntry> {
    let mut out = Vec::new();
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(e) => e,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                if path.join(crate::project::MANIFEST_FILE).is_file() {
                    let text = match std::fs::read_to_string(integrity_path(&path)) {
                        Ok(t) => t,
                        Err(_) => continue,
                    };
                    let mut lines = text.lines();
                    match (lines.next(), lines.next(), lines.next()) {
                        (Some(full), Some(version), Some(integrity))
                            if !full.is_empty()
                                && !version.is_empty()
                                && !integrity.is_empty() =>
                        {
                            out.push(HaveEntry {
                                full: full.to_string(),
                                version: version.to_string(),
                                integrity: integrity.to_string(),
                            });
                        }
                        _ => continue,
                    }
                } else {
                    stack.push(path);
                }
            }
        }
    }
    out.sort_by(|a, b| (&a.full, &a.version).cmp(&(&b.full, &b.version)));
    out.dedup_by(|a, b| a.full == b.full && a.version == b.version);
    out
}

pub fn download_packages(
    requirements: &BTreeMap<String, String>,
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
    scope_root: Option<&Path>,
) -> Result<Vec<FetchedPackage>, Diagnostic> {
    let _origin = origin_direct_guard();
    let (root, have) = match scope_root {
        Some(scope) => {
            let root = project_registry_root(scope);
            let mut have = scan_cache_have();
            have.extend(scan_have_under(&root));
            have.sort_by(|a, b| (&a.full, &a.version).cmp(&(&b.full, &b.version)));
            have.dedup_by(|a, b| a.full == b.full && a.version == b.version);
            (root, have)
        }
        None => (store_root(), scan_cache_have()),
    };
    Ok(ensure_registry_requirements_in(&root, requirements, default_registry, overrides, &have)?
        .into_iter()
        .collect())
}

fn registry_scope(full: &str) -> Option<&str> {
    let rest = full.strip_prefix('@')?;
    let end = rest.find('/')?;
    if end == 0 || end + 1 >= full.len() {
        return None;
    }
    Some(&full[..end + 1])
}

pub fn expand_registry_base(raw: &str) -> String {
    let mut trimmed = raw.trim().trim_end_matches('/').to_string();
    if trimmed.is_empty() {
        return format!("https://{DEFAULT_REGISTRY_DOMAIN}");
    }
    // The registry value is a host base. Older setups carried the packages
    // path (`/api/packages`); strip it so those values keep working.
    for suffix in ["/api/v0/packages", "/api/packages"] {
        if let Some(base) = trimmed.strip_suffix(suffix) {
            trimmed = base.trim_end_matches('/').to_string();
            break;
        }
    }
    if trimmed.contains("://") {
        return trimmed;
    }
    format!("https://{trimmed}")
}

pub fn registry_base_for(
    full: &str,
    default: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
) -> String {
    if let Some(scope) = registry_scope(full) {
        if let Some(cfg) = overrides.get(scope) {
            if !cfg.url.trim().is_empty() {
                return expand_registry_base(&cfg.url);
            }
        }
    }
    if let Some(cfg) = default {
        if !cfg.url.trim().is_empty() {
            return expand_registry_base(&cfg.url);
        }
    }
    expand_registry_base("")
}

pub fn is_valid_range(range: &str) -> bool {
    let r = range.trim();
    if r.is_empty() || r == "*" || r == "latest" {
        return true;
    }
    if let Some(rest) = r.strip_prefix('^').or_else(|| r.strip_prefix('~')) {
        return parse_semver(rest.trim()).is_some();
    }
    if let Some(rest) = r.strip_prefix(">=") {
        return parse_semver(rest.trim()).is_some();
    }
    parse_semver(r).is_some()
}

fn parse_semver(v: &str) -> Option<(u64, u64, u64, String)> {
    let (core, pre) = match v.split_once('-') {
        Some((c, p)) => (c, p.to_string()),
        None => (v, String::new()),
    };
    let mut parts = core.split('.');
    let major = parts.next()?.parse::<u64>().ok()?;
    let minor = parts.next()?.parse::<u64>().ok()?;
    let patch = parts.next()?.parse::<u64>().ok()?;
    if parts.next().is_some() {
        return None;
    }
    if core.split('.').any(|p| p.is_empty() || !p.bytes().all(|b| b.is_ascii_digit())) {
        return None;
    }
    if !pre.is_empty()
        && (pre.bytes().any(|b| !(b.is_ascii_alphanumeric() || b == b'-' || b == b'.'))
            || pre.split('.').any(|p| p.is_empty()))
    {
        return None;
    }
    Some((major, minor, patch, pre))
}

fn cmp_pre(a: &str, b: &str) -> std::cmp::Ordering {
    if a == b {
        return std::cmp::Ordering::Equal;
    }
    if a.is_empty() {
        return std::cmp::Ordering::Greater;
    }
    if b.is_empty() {
        return std::cmp::Ordering::Less;
    }
    let pa: Vec<&str> = a.split('.').collect();
    let pb: Vec<&str> = b.split('.').collect();
    for i in 0..pa.len().max(pb.len()) {
        match (pa.get(i), pb.get(i)) {
            (None, _) => return std::cmp::Ordering::Less,
            (_, None) => return std::cmp::Ordering::Greater,
            (Some(x), Some(y)) => {
                if x == y {
                    continue;
                }
                let nx = x.parse::<u64>().ok();
                let ny = y.parse::<u64>().ok();
                match (nx, ny) {
                    (Some(n), Some(m)) => {
                        if n != m {
                            return n.cmp(&m);
                        }
                    }
                    _ => {
                        if x != y {
                            return x.cmp(y);
                        }
                    }
                }
            }
        }
    }
    std::cmp::Ordering::Equal
}

fn cmp_semver(a: (u64, u64, u64, &str), b: (u64, u64, u64, &str)) -> std::cmp::Ordering {
    (a.0, a.1, a.2)
        .cmp(&(b.0, b.1, b.2))
        .then_with(|| cmp_pre(a.3, b.3))
}

fn range_base(range: &str) -> Option<(u64, u64, u64, String)> {
    let r = range.trim();
    if let Some(rest) = r.strip_prefix('^').or_else(|| r.strip_prefix('~')) {
        return parse_semver(rest.trim());
    }
    if let Some(rest) = r.strip_prefix(">=") {
        return parse_semver(rest.trim());
    }
    parse_semver(r)
}

fn range_admits_pre(range: &str, current: (u64, u64, u64, &str)) -> bool {
    let r = range.trim();
    if r.is_empty() || r == "*" || r == "latest" {
        return false;
    }
    let Some(b) = range_base(r) else {
        return false;
    };
    if b.3.is_empty() {
        return false;
    }
    b.0 == current.0 && b.1 == current.1 && b.2 == current.2
}

pub(crate) fn engine_satisfied(range: &str, current: &str) -> bool {
    let Some(v) = parse_semver(current.trim()) else {
        return false;
    };
    let r = range.trim();
    if r.is_empty() || r == "*" || r == "latest" {
        return v.3.is_empty();
    }
    if !v.3.is_empty() && !range_admits_pre(r, (v.0, v.1, v.2, v.3.as_str())) {
        return false;
    }
    if let Some(rest) = r.strip_prefix('^') {
        let Some(b) = parse_semver(rest.trim()) else {
            return false;
        };
        if cmp_semver((v.0, v.1, v.2, v.3.as_str()), (b.0, b.1, b.2, b.3.as_str()))
            == std::cmp::Ordering::Less
        {
            return false;
        }
        if b.0 > 0 {
            return v.0 == b.0;
        }
        if b.1 > 0 {
            return v.0 == 0 && v.1 == b.1;
        }
        return v.0 == 0 && v.1 == 0 && v.2 == b.2;
    }
    if let Some(rest) = r.strip_prefix('~') {
        let Some(b) = parse_semver(rest.trim()) else {
            return false;
        };
        if cmp_semver((v.0, v.1, v.2, v.3.as_str()), (b.0, b.1, b.2, b.3.as_str()))
            == std::cmp::Ordering::Less
        {
            return false;
        }
        return v.0 == b.0 && v.1 == b.1;
    }
    if let Some(rest) = r.strip_prefix(">=") {
        let Some(b) = parse_semver(rest.trim()) else {
            return false;
        };
        return cmp_semver((v.0, v.1, v.2, v.3.as_str()), (b.0, b.1, b.2, b.3.as_str()))
            != std::cmp::Ordering::Less;
    }
    let Some(b) = parse_semver(r) else {
        return false;
    };
    cmp_semver((v.0, v.1, v.2, v.3.as_str()), (b.0, b.1, b.2, b.3.as_str()))
        == std::cmp::Ordering::Equal
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HaveEntry {
    pub full: String,
    pub version: String,
    pub integrity: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveNode {
    pub full: String,
    pub version: String,
    pub path: String,
    pub integrity: String,
    pub engine_range: String,
    pub deps: Vec<String>,
    pub yanked: bool,
    pub artifact_digest: Option<String>,
    pub entries: Option<Vec<ChunkEntry>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolveResponse {
    pub resolved: BTreeMap<String, String>,
    pub base: String,
    pub levels: Vec<Vec<ResolveNode>>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPackage {
    pub name: String,
    pub version: String,
    pub dir: PathBuf,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Discovery {
    pub spec: u32,
    pub capabilities: Vec<String>,
    pub registry: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RegistryReply {
    pub status: u16,
    pub spec_header: Option<String>,
    pub body: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestReply {
    pub reply: RegistryReply,
    pub content_encoding: Option<String>,
}

pub trait RegistryTransport: Send + Sync {
    fn get(&self, url: &str) -> Result<RegistryReply, Diagnostic>;
    fn post_json(&self, url: &str, body: &str) -> Result<RegistryReply, Diagnostic>;
    fn manifest_get(&self, url: &str) -> Result<ManifestReply, Diagnostic> {
        Ok(ManifestReply { reply: self.get(url)?, content_encoding: None })
    }
}

static TRANSPORT: std::sync::Mutex<Option<std::sync::Arc<dyn RegistryTransport>>> =
    std::sync::Mutex::new(None);

static ORIGIN_DIRECT: std::sync::atomic::AtomicBool = std::sync::atomic::AtomicBool::new(false);

pub fn origin_direct() -> bool {
    ORIGIN_DIRECT.load(std::sync::atomic::Ordering::SeqCst)
}

pub struct OriginDirectGuard {
    prev: bool,
}

impl Drop for OriginDirectGuard {
    fn drop(&mut self) {
        ORIGIN_DIRECT.store(self.prev, std::sync::atomic::Ordering::SeqCst);
    }
}

// Reads transport payloads (manifests, chunks) under a fixed
// `?origin-direct=1` namespace instead of the bare URL. Shared edge caches
// can hold manifest generations whose chunks no longer match resolve
// integrity, and client revalidation headers do not bypass them; a
// distinct key populated from origin (always live truth, rows are
// write-once) stays correct, and the server answers it no-store so the
// namespace can never cache a stale generation. Tooling that must see
// one consistent generation (stdlib seeding, stdlib docs) holds the
// guard; per-run and install paths keep edge caching with a single
// guarded retry on generation mismatch.
pub fn origin_direct_guard() -> OriginDirectGuard {
    let prev = ORIGIN_DIRECT.swap(true, std::sync::atomic::Ordering::SeqCst);
    OriginDirectGuard { prev }
}

fn direct_url(url: &str, direct: bool) -> String {
    if !direct {
        return url.to_string();
    }
    if url.contains('?') {
        format!("{url}&origin-direct=1")
    } else {
        format!("{url}?origin-direct=1")
    }
}

fn is_generation_error(e: &Diagnostic) -> bool {
    e.message.contains("checksum mismatch")
        || e.message.contains("is missing")
        || e.message.contains("failed its integrity check")
        || e.message.contains("failed to reassemble")
}

pub fn set_registry_transport(transport: std::sync::Arc<dyn RegistryTransport>) {
    let mut slot = TRANSPORT.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(transport);
}

/// Installs the built-in HTTP/2 client. The registry answers ALPN with
/// h2, so one session per host carries the whole chunk burst of a cold
/// seed: the streams multiplex over it instead of every request paying
/// its own handshake.
pub fn install_http2_transport() {
    set_registry_transport(http2_transport());
}

fn http2_transport() -> std::sync::Arc<dyn RegistryTransport> {
    #[cfg(not(target_arch = "wasm32"))]
    {
        static CLIENT: std::sync::OnceLock<std::sync::Arc<dyn RegistryTransport>> =
            std::sync::OnceLock::new();
        CLIENT
            .get_or_init(|| match H2Transport::new() {
                Ok(client) => std::sync::Arc::new(client),
                Err(_) => std::sync::Arc::new(StdTransport),
            })
            .clone()
    }
    #[cfg(target_arch = "wasm32")]
    {
        std::sync::Arc::new(StdTransport)
    }
}

fn transport() -> std::sync::Arc<dyn RegistryTransport> {
    let slot = TRANSPORT.lock().unwrap_or_else(|e| e.into_inner());
    if let Some(active) = slot.clone() {
        return active;
    }
    std::sync::Arc::new(StdTransport)
}

struct StdTransport;

fn split_http_url(url: &str) -> Result<(String, u16, String), Diagnostic> {
    let rest = url.strip_prefix("http://").ok_or_else(|| {
        Diagnostic::new(
            Code::E108,
            format!("registry `{url}` needs http for the built-in transport (https is served through the rnx CLI)"),
        )
    })?;
    let (authority, path) = match rest.find('/') {
        Some(i) => (rest[..i].to_string(), rest[i..].to_string()),
        None => (rest.to_string(), "/".to_string()),
    };
    let (host, port) = match authority.rsplit_once(':') {
        Some((h, p)) => match p.parse::<u16>() {
            Ok(n) => (h.to_string(), n),
            Err(_) => (authority, 80),
        },
        None => (authority, 80),
    };
    if host.is_empty() || path.is_empty() {
        return Err(Diagnostic::new(Code::E108, format!("bad registry url `{url}`")));
    }
    Ok((host, port, path))
}

fn read_http_response(stream: &mut std::net::TcpStream) -> Result<(RegistryReply, bool, Option<String>), Diagnostic> {
    use std::io::Read;
    let mut head = Vec::new();
    let mut byte = [0u8; 1];
    while !head.ends_with(b"\r\n\r\n") {
        match stream.read(&mut byte) {
            Ok(0) => break,
            Ok(_) => head.push(byte[0]),
            Err(e) => {
                return Err(Diagnostic::new(Code::E108, format!("registry request failed: {e}")));
            }
        }
        if head.len() > 65536 {
            return Err(Diagnostic::new(Code::E108, "registry response header too large"));
        }
    }
    let text = String::from_utf8_lossy(&head).into_owned();
    let mut lines = text.lines();
    let status_line = lines.next().unwrap_or_default().to_string();
    let status = status_line
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| Diagnostic::new(Code::E108, "registry returned a bad status line"))?;
    let http11 = status_line.starts_with("HTTP/1.1");
    let mut spec_header = None;
    let mut content_encoding = None;
    let mut content_length = None;
    let mut chunked = false;
    let mut conn_close = false;
    let mut conn_keep = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "rnx-registry-spec" => spec_header = Some(value.trim().to_string()),
                "content-encoding" => content_encoding = Some(value.trim().to_string()),
                "content-length" => content_length = value.trim().parse::<usize>().ok(),
                "transfer-encoding" => {
                    chunked = value.to_ascii_lowercase().contains("chunked");
                }
                "connection" => {
                    for token in value.split(',') {
                        match token.trim().to_ascii_lowercase().as_str() {
                            "close" => conn_close = true,
                            "keep-alive" => conn_keep = true,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
    }
    let framed = chunked || content_length.is_some();
    let reusable = framed && !conn_close && (http11 || conn_keep);
    if chunked {
        return Ok((
            RegistryReply {
                status,
                spec_header,
                body: read_chunked(stream)?,
            },
            reusable,
            content_encoding,
        ));
    }
    let mut body = Vec::new();
    match content_length {
        Some(0) => {}
        Some(n) => {
            let mut rest = vec![0u8; n.min(MAX_DOWNLOAD_BYTES + 1)];
            let mut read = 0usize;
            while read < rest.len() {
                match stream.read(&mut rest[read..]) {
                    Ok(0) => break,
                    Ok(m) => read += m,
                    Err(e) => {
                        return Err(Diagnostic::new(
                            Code::E108,
                            format!("registry request failed: {e}"),
                        ));
                    }
                }
            }
            rest.truncate(read);
            body = rest;
        }
        None => {
            let mut rest = Vec::new();
            if stream.read_to_end(&mut rest).is_err() {
                return Err(Diagnostic::new(Code::E108, "registry request failed"));
            }
            body = rest;
        }
    }
    Ok((RegistryReply { status, spec_header, body }, reusable, content_encoding))
}

fn read_chunked(stream: &mut std::net::TcpStream) -> Result<Vec<u8>, Diagnostic> {
    use std::io::Read;
    let mut body = Vec::new();
    loop {
        let mut line = Vec::new();
        let mut byte = [0u8; 1];
        while !line.ends_with(b"\r\n") {
            match stream.read(&mut byte) {
                Ok(0) => {
                    return Err(Diagnostic::new(Code::E108, "truncated chunked response"));
                }
                Ok(_) => line.push(byte[0]),
                Err(e) => {
                    return Err(Diagnostic::new(Code::E108, format!("registry request failed: {e}")));
                }
            }
            if line.len() > 32 {
                return Err(Diagnostic::new(Code::E108, "bad chunk size"));
            }
        }
        let size_text = String::from_utf8_lossy(&line).trim().to_string();
        let size_text = size_text.split(';').next().unwrap_or("").trim();
        let size = usize::from_str_radix(size_text, 16)
            .map_err(|_| Diagnostic::new(Code::E108, "bad chunk size"))?;
        if size == 0 {
            break;
        }
        if body.len() + size > MAX_DOWNLOAD_BYTES + 1 {
            return Err(Diagnostic::new(Code::E108, "registry response too large"));
        }
        let mut chunk = vec![0u8; size];
        let mut read = 0usize;
        while read < size {
            match stream.read(&mut chunk[read..]) {
                Ok(0) => {
                    return Err(Diagnostic::new(Code::E108, "truncated chunked response"));
                }
                Ok(m) => read += m,
                Err(e) => {
                    return Err(Diagnostic::new(Code::E108, format!("registry request failed: {e}")));
                }
            }
        }
        body.extend_from_slice(&chunk);
        let mut crlf = [0u8; 2];
        let mut read = 0usize;
        while read < 2 {
            match stream.read(&mut crlf[read..]) {
                Ok(0) => {
                    return Err(Diagnostic::new(Code::E108, "truncated chunked response"));
                }
                Ok(m) => read += m,
                Err(e) => {
                    return Err(Diagnostic::new(Code::E108, format!("registry request failed: {e}")));
                }
            }
        }
    }
    Ok(body)
}

impl RegistryTransport for StdTransport {
    fn get(&self, url: &str) -> Result<RegistryReply, Diagnostic> {
        std_request("GET", url, None)
    }

    fn post_json(&self, url: &str, body: &str) -> Result<RegistryReply, Diagnostic> {
        std_request("POST", url, Some(body.as_bytes()))
    }

    fn manifest_get(&self, url: &str) -> Result<ManifestReply, Diagnostic> {
        std_request_full("GET", url, None, true)
            .map(|(reply, content_encoding)| ManifestReply { reply, content_encoding })
    }
}

// One session per host, many streams on it: the chunk reads of a cold
// seed share one handshake instead of paying one per request.
#[cfg(not(target_arch = "wasm32"))]
struct H2Transport {
    client: reqwest::blocking::Client,
}

#[cfg(not(target_arch = "wasm32"))]
impl H2Transport {
    fn new() -> Result<H2Transport, Diagnostic> {
        let client = reqwest::blocking::Client::builder()
            .timeout(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS))
            .user_agent("rnx")
            .build()
            .map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot start the registry client: {e}"))
            })?;
        Ok(H2Transport { client })
    }

    fn reply(
        what: &str,
        result: Result<reqwest::blocking::Response, reqwest::Error>,
    ) -> Result<(RegistryReply, Option<String>), Diagnostic> {
        use std::io::Read;
        let response = result.map_err(|e| {
            if e.is_timeout() {
                Diagnostic::new(
                    Code::E108,
                    format!("{what} timed out after {REQUEST_TIMEOUT_SECS}s"),
                )
            } else {
                Diagnostic::new(Code::E108, format!("{what} failed: {}", reqwest_chain(&e)))
            }
        })?;
        let status = response.status().as_u16();
        let spec_header = response
            .headers()
            .get("rnx-registry-spec")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.to_string());
        let content_encoding = response
            .headers()
            .get("content-encoding")
            .and_then(|v| v.to_str().ok())
            .map(|s| s.trim().to_string());
        let mut body = Vec::new();
        response
            .take((MAX_DOWNLOAD_BYTES + 1) as u64)
            .read_to_end(&mut body)
            .map_err(|e| Diagnostic::new(Code::E108, format!("{what} failed: {e}")))?;
        if body.len() > MAX_DOWNLOAD_BYTES {
            return Err(Diagnostic::new(Code::E108, "registry response too large"));
        }
        Ok((RegistryReply { status, spec_header, body }, content_encoding))
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl RegistryTransport for H2Transport {
    fn get(&self, url: &str) -> Result<RegistryReply, Diagnostic> {
        let result = self.client.get(url).header("Accept", "application/json").send();
        Self::reply("registry request", result).map(|(reply, _)| reply)
    }

    fn post_json(&self, url: &str, body: &str) -> Result<RegistryReply, Diagnostic> {
        let result = self
            .client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json")
            .body(body.to_string())
            .send();
        Self::reply("registry request", result).map(|(reply, _)| reply)
    }

    fn manifest_get(&self, url: &str) -> Result<ManifestReply, Diagnostic> {
        let result = self
            .client
            .get(url)
            .header("Accept", "application/json")
            .header("Accept-Encoding", "gzip")
            .send();
        let (reply, content_encoding) = Self::reply("registry request", result)?;
        Ok(ManifestReply { reply, content_encoding })
    }
}

fn std_request(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> Result<RegistryReply, Diagnostic> {
    std_request_full(method, url, body, false).map(|(reply, _)| reply)
}

fn std_request_full(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
    accept_gzip: bool,
) -> Result<(RegistryReply, Option<String>), Diagnostic> {
    let (host, port, path) = split_http_url(url)?;
    let mut stream = match pool_take(&host, port) {
        Some(reused) => reused,
        None => connect_new(&host, port)?,
    };
    match exchange_once(method, &host, &path, body, accept_gzip, &mut stream) {
        Ok((reply, reusable, content_encoding)) => {
            if reusable {
                pool_put(&host, port, stream);
            }
            Ok((reply, content_encoding))
        }
        Err(dead) => {
            drop(stream);
            let mut fresh = connect_new(&host, port).map_err(|_| dead)?;
            let (reply, reusable, content_encoding) =
                exchange_once(method, &host, &path, body, accept_gzip, &mut fresh)?;
            if reusable {
                pool_put(&host, port, fresh);
            }
            Ok((reply, content_encoding))
        }
    }
}

static POOLED: std::sync::Mutex<BTreeMap<(String, u16), Vec<std::net::TcpStream>>> =
    std::sync::Mutex::new(BTreeMap::new());

fn pool_take(host: &str, port: u16) -> Option<std::net::TcpStream> {
    POOLED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get_mut(&(host.to_string(), port))
        .and_then(|idle| idle.pop())
}

fn pool_put(host: &str, port: u16, stream: std::net::TcpStream) {
    let mut pool = POOLED.lock().unwrap_or_else(|e| e.into_inner());
    let idle = pool.entry((host.to_string(), port)).or_default();
    if idle.len() < POOL_MAX_PER_HOST {
        idle.push(stream);
    }
}

fn connect_new(host: &str, port: u16) -> Result<std::net::TcpStream, Diagnostic> {
    use std::net::ToSocketAddrs;
    let addr = format!("{host}:{port}");
    let target = addr
        .to_socket_addrs()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot reach registry `{addr}`: {e}")))?
        .next()
        .ok_or_else(|| Diagnostic::new(Code::E108, format!("cannot reach registry `{addr}`")))?;
    std::net::TcpStream::connect_timeout(
        &target,
        std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS),
    )
    .map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot reach registry `{addr}`: {e}"))
    })
}

fn exchange_once(
    method: &str,
    host: &str,
    path: &str,
    body: Option<&[u8]>,
    accept_gzip: bool,
    stream: &mut std::net::TcpStream,
) -> Result<(RegistryReply, bool, Option<String>), Diagnostic> {
    use std::io::Write;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS)))
        .map_err(|e| Diagnostic::new(Code::E108, format!("registry request failed: {e}")))?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS)))
        .map_err(|e| Diagnostic::new(Code::E108, format!("registry request failed: {e}")))?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: keep-alive\r\n");
    if method == "POST" {
        head.push_str("Content-Type: application/json\r\nAccept: application/json\r\n");
    } else {
        head.push_str("Accept: application/json\r\n");
    }
    // The chunk manifest is served compressed to a client that asks for it,
    // and the manifest read is the only read that decodes a compressed
    // body, so only it asks for one. Every other read stays on the
    // identity encoding.
    if accept_gzip {
        head.push_str("Accept-Encoding: gzip\r\n");
    }
    let len = body.map(|b| b.len()).unwrap_or(0);
    head.push_str(&format!("Content-Length: {len}\r\n\r\n"));
    stream
        .write_all(head.as_bytes())
        .map_err(|e| Diagnostic::new(Code::E108, format!("registry request failed: {e}")))?;
    if let Some(payload) = body {
        stream
            .write_all(payload)
            .map_err(|e| Diagnostic::new(Code::E108, format!("registry request failed: {e}")))?;
    }
    read_http_response(stream)
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

#[cfg(not(target_arch = "wasm32"))]
fn reqwest_chain(error: &reqwest::Error) -> String {
    use std::error::Error;
    let mut out = error.to_string();
    let mut next = error.source();
    while let Some(source) = next {
        out.push_str(": ");
        out.push_str(&source.to_string());
        next = source.source();
    }
    out
}

fn is_fast_fail_offline(message: &str) -> bool {
    let lower = message.to_ascii_lowercase();
    [
        "cannot reach registry",
        "connection refused",
        "dns error",
        "failed to resolve",
        "network is unreachable",
        "no route to host",
        "name or service not known",
        "temporary failure in name resolution",
        "no such host is known",
    ]
    .iter()
    .any(|s| lower.contains(s))
}

fn offline_diagnostic(what: &str, detail: &str) -> Diagnostic {
    Diagnostic::new(
        Code::E113,
        format!("{what} failed: the registry is unreachable ({detail}); check the network or seed the cache"),
    )
    .with_hint("run `rnx download` to seed the cache (or check the network)")
}

fn offline_package_error(pkg: &str, base: &str, detail: &str) -> Diagnostic {
    Diagnostic::new(
        Code::E113,
        format!("cannot fetch `{pkg}` from registry `{base}` while offline: {detail}"),
    )
    .with_hint("run `rnx download` to seed the cache (or check the network)")
}

fn with_offline_package_context(err: Diagnostic, pkg: &str, base: &str) -> Diagnostic {
    if err.code == Code::E113 || is_fast_fail_offline(&err.message) {
        return offline_package_error(pkg, base, &err.message);
    }
    err
}

pub fn registry_get(url: &str, what: &str) -> Result<RegistryReply, Diagnostic> {
    http_get(url, what)
}

fn http_get(url: &str, what: &str) -> Result<RegistryReply, Diagnostic> {
    http_get_via(&*transport(), url, what)
}

fn http_get_via(
    transport: &dyn RegistryTransport,
    url: &str,
    what: &str,
) -> Result<RegistryReply, Diagnostic> {
    let mut attempt = 0u32;
    loop {
        match transport.get(url) {
            Ok(reply) => {
                if retryable_status(reply.status) && attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Ok(reply);
            }
            Err(e) => {
                if is_fast_fail_offline(&e.message) {
                    return Err(offline_diagnostic(what, &e.message));
                }
                if attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Err(Diagnostic::new(Code::E108, format!("{what} failed: {}", e.message)));
            }
        }
    }
}

fn http_manifest_get(url: &str, what: &str) -> Result<ManifestReply, Diagnostic> {
    http_manifest_get_via(&*transport(), url, what)
}

fn http_manifest_get_via(
    transport: &dyn RegistryTransport,
    url: &str,
    what: &str,
) -> Result<ManifestReply, Diagnostic> {
    let mut attempt = 0u32;
    loop {
        match transport.manifest_get(url) {
            Ok(reply) => {
                if retryable_status(reply.reply.status) && attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Ok(reply);
            }
            Err(e) => {
                if is_fast_fail_offline(&e.message) {
                    return Err(offline_diagnostic(what, &e.message));
                }
                if attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Err(Diagnostic::new(Code::E108, format!("{what} failed: {}", e.message)));
            }
        }
    }
}

fn http_post_json(url: &str, body: &str, what: &str) -> Result<RegistryReply, Diagnostic> {
    http_post_json_via(&*transport(), url, body, what)
}

fn http_post_json_via(
    transport: &dyn RegistryTransport,
    url: &str,
    body: &str,
    what: &str,
) -> Result<RegistryReply, Diagnostic> {
    let mut attempt = 0u32;
    loop {
        match transport.post_json(url, body) {
            Ok(reply) => {
                if retryable_status(reply.status) && attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Ok(reply);
            }
            Err(e) => {
                if is_fast_fail_offline(&e.message) {
                    return Err(offline_diagnostic(what, &e.message));
                }
                if attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Err(Diagnostic::new(Code::E108, format!("{what} failed: {}", e.message)));
            }
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum JsonValue {
    Null,
    Bool(bool),
    Number(f64),
    Str(String),
    Array(Vec<JsonValue>),
    Object(Vec<(String, JsonValue)>),
}

impl JsonValue {
    pub fn get(&self, key: &str) -> Option<&JsonValue> {
        match self {
            JsonValue::Object(fields) => fields.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            JsonValue::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_u64(&self) -> Option<u64> {
        match self {
            JsonValue::Number(n) if n.is_finite() && *n >= 0.0 && n.fract() == 0.0 => {
                Some(*n as u64)
            }
            _ => None,
        }
    }

    pub fn as_bool(&self) -> Option<bool> {
        match self {
            JsonValue::Bool(b) => Some(*b),
            _ => None,
        }
    }

    pub fn as_array(&self) -> Option<&[JsonValue]> {
        match self {
            JsonValue::Array(items) => Some(items),
            _ => None,
        }
    }
}

struct JsonParser<'a> {
    bytes: &'a [u8],
    pos: usize,
}

fn json_bad() -> Diagnostic {
    Diagnostic::new(Code::E108, "registry returned malformed JSON")
}

pub fn parse_json(body: &[u8]) -> Result<JsonValue, Diagnostic> {
    let mut parser = JsonParser { bytes: body, pos: 0 };
    parser.skip_ws();
    let value = parser.value()?;
    parser.skip_ws();
    if parser.pos != parser.bytes.len() {
        return Err(json_bad());
    }
    Ok(value)
}

pub fn json_escape(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{:04x}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out
}

impl<'a> JsonParser<'a> {
    fn skip_ws(&mut self) {
        while self.pos < self.bytes.len()
            && matches!(self.bytes[self.pos], b' ' | b'\t' | b'\n' | b'\r')
        {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Result<u8, Diagnostic> {
        self.bytes.get(self.pos).copied().ok_or_else(json_bad)
    }

    fn value(&mut self) -> Result<JsonValue, Diagnostic> {
        match self.peek()? {
            b'n' => self.literal("null", JsonValue::Null),
            b't' => self.literal("true", JsonValue::Bool(true)),
            b'f' => self.literal("false", JsonValue::Bool(false)),
            b'"' => Ok(JsonValue::Str(self.string()?)),
            b'[' => self.array(),
            b'{' => self.object(),
            b'-' | b'0'..=b'9' => self.number(),
            _ => Err(json_bad()),
        }
    }

    fn literal(&mut self, word: &str, value: JsonValue) -> Result<JsonValue, Diagnostic> {
        if self.bytes.len() >= self.pos + word.len()
            && &self.bytes[self.pos..self.pos + word.len()] == word.as_bytes()
        {
            self.pos += word.len();
            Ok(value)
        } else {
            Err(json_bad())
        }
    }

    fn string(&mut self) -> Result<String, Diagnostic> {
        if self.peek()? != b'"' {
            return Err(json_bad());
        }
        self.pos += 1;
        let mut out = String::new();
        loop {
            let b = self.bytes.get(self.pos).copied().ok_or_else(json_bad)?;
            self.pos += 1;
            match b {
                b'"' => return Ok(out),
                b'\\' => {
                    let e = self.bytes.get(self.pos).copied().ok_or_else(json_bad)?;
                    self.pos += 1;
                    match e {
                        b'"' => out.push('"'),
                        b'\\' => out.push('\\'),
                        b'/' => out.push('/'),
                        b'b' => out.push('\u{0008}'),
                        b'f' => out.push('\u{000C}'),
                        b'n' => out.push('\n'),
                        b'r' => out.push('\r'),
                        b't' => out.push('\t'),
                        b'u' => {
                            if self.pos + 4 > self.bytes.len() {
                                return Err(json_bad());
                            }
                            let hex = std::str::from_utf8(&self.bytes[self.pos..self.pos + 4])
                                .map_err(|_| json_bad())?;
                            let code = u32::from_str_radix(hex, 16).map_err(|_| json_bad())?;
                            self.pos += 4;
                            match char::from_u32(code) {
                                Some(c) => out.push(c),
                                None => return Err(json_bad()),
                            }
                        }
                        _ => return Err(json_bad()),
                    }
                }
                0x00..=0x1F => return Err(json_bad()),
                _ => {
                    let start = self.pos - 1;
                    let mut end = start + 1;
                    while end < self.bytes.len() && self.bytes[end] >= 0x80 {
                        end += 1;
                    }
                    let chunk =
                        std::str::from_utf8(&self.bytes[start..end]).map_err(|_| json_bad())?;
                    out.push_str(chunk);
                    self.pos = end;
                }
            }
        }
    }

    fn array(&mut self) -> Result<JsonValue, Diagnostic> {
        self.pos += 1;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek()? == b']' {
            self.pos += 1;
            return Ok(JsonValue::Array(items));
        }
        loop {
            self.skip_ws();
            items.push(self.value()?);
            self.skip_ws();
            match self.peek()? {
                b',' => {
                    self.pos += 1;
                }
                b']' => {
                    self.pos += 1;
                    return Ok(JsonValue::Array(items));
                }
                _ => return Err(json_bad()),
            }
        }
    }

    fn object(&mut self) -> Result<JsonValue, Diagnostic> {
        self.pos += 1;
        let mut fields = Vec::new();
        self.skip_ws();
        if self.peek()? == b'}' {
            self.pos += 1;
            return Ok(JsonValue::Object(fields));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            if self.peek()? != b':' {
                return Err(json_bad());
            }
            self.pos += 1;
            self.skip_ws();
            fields.push((key, self.value()?));
            self.skip_ws();
            match self.peek()? {
                b',' => {
                    self.pos += 1;
                }
                b'}' => {
                    self.pos += 1;
                    return Ok(JsonValue::Object(fields));
                }
                _ => return Err(json_bad()),
            }
        }
    }

    fn number(&mut self) -> Result<JsonValue, Diagnostic> {
        let start = self.pos;
        if self.bytes[self.pos] == b'-' {
            self.pos += 1;
        }
        if self.bytes.get(self.pos) == Some(&b'0') {
            self.pos += 1;
            if self.bytes.get(self.pos).is_some_and(|b| b.is_ascii_digit()) {
                return Err(json_bad());
            }
        } else {
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if self.pos == start || (self.bytes[start] == b'-' && self.pos == start + 1) {
            return Err(json_bad());
        }
        if self.bytes.get(self.pos) == Some(&b'.') {
            self.pos += 1;
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        if matches!(self.bytes.get(self.pos), Some(b'e') | Some(b'E')) {
            self.pos += 1;
            if matches!(self.bytes.get(self.pos), Some(b'+') | Some(b'-')) {
                self.pos += 1;
            }
            while self.pos < self.bytes.len() && self.bytes[self.pos].is_ascii_digit() {
                self.pos += 1;
            }
        }
        let text = std::str::from_utf8(&self.bytes[start..self.pos]).map_err(|_| json_bad())?;
        text.parse::<f64>().map(JsonValue::Number).map_err(|_| json_bad())
    }
}

fn spec_mismatch(got: u64) -> Diagnostic {
    Diagnostic::new(
        Code::E108,
        format!("registry speaks spec {got}, this CLI supports {REGISTRY_SPEC}"),
    )
}

pub fn discover(base: &str) -> Result<Discovery, Diagnostic> {
    let url = format!("{}/api/version", base.trim_end_matches('/'));
    let reply = http_get(&url, "registry discovery")?;
    if reply.status != 200 {
        return Err(Diagnostic::new(
            Code::E108,
            format!("registry discovery failed: {url} returned HTTP {}", reply.status),
        ));
    }
    let value = parse_json(&reply.body).map_err(|_| {
        Diagnostic::new(
            Code::E108,
            format!("registry discovery failed: {url} returned invalid JSON"),
        )
    })?;
    let spec = value
        .get("spec")
        .and_then(|v| v.as_u64())
        .ok_or_else(|| {
            Diagnostic::new(
                Code::E108,
                format!("registry discovery failed: {url} returned no `spec` number"),
            )
        })?;
    if spec != REGISTRY_SPEC as u64 {
        return Err(spec_mismatch(spec));
    }
    if let Some(header) = reply.spec_header {
        if let Ok(n) = header.trim().parse::<u64>() {
            if n != REGISTRY_SPEC as u64 {
                return Err(spec_mismatch(n));
            }
        }
    }
    let capabilities = value
        .get("capabilities")
        .and_then(|v| v.as_array())
        .map(|items| {
            items
                .iter()
                .filter_map(|i| i.as_str().map(|s| s.to_string()))
                .collect()
        })
        .unwrap_or_default();
    let registry = value
        .get("registry")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    Ok(Discovery { spec: REGISTRY_SPEC, capabilities, registry })
}

fn json_string(value: &JsonValue, key: &str) -> String {
    value
        .get(key)
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string()
}

fn engine_version() -> &'static str {
    env!("CARGO_PKG_VERSION")
}

fn resolve_request_body(
    requirements: &BTreeMap<String, String>,
    have: &[HaveEntry],
    inline_chunks: bool,
) -> String {
    let mut out = String::from("{\"requirements\": {");
    for (i, (full, range)) in requirements.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!("\"{}\": \"{}\"", json_escape(full), json_escape(range)));
    }
    out.push_str("}, \"have\": [");
    for (i, h) in have.iter().enumerate() {
        if i > 0 {
            out.push(',');
        }
        out.push_str(&format!(
            "{{\"full\": \"{}\", \"version\": \"{}\", \"integrity\": \"{}\"}}",
            json_escape(&h.full),
            json_escape(&h.version),
            json_escape(&h.integrity)
        ));
    }
    out.push(']');
    out.push_str(&format!(", \"engine\": \"{}\"", json_escape(engine_version())));
    if inline_chunks {
        out.push_str(", \"chunks\": true");
    }
    out.push('}');
    out
}

pub fn resolve_retry_delay_ms(attempt: u32) -> u64 {
    150 * 2u64.saturating_pow(attempt.min(4))
}

// Statuses this client can explain. The registry answers with a structured
// body (code plus message) for every one of them, so a failure is read as
// the failure it is.
const RESOLVE_KNOWN_STATUSES: &[u16] = &[400, 404, 409, 410, 422, 429, 500, 502, 503, 504];

// A status outside that list is a dialect this client version does not
// know. The raw status is printed and the client is named as out of date,
// because a client that cannot read the answer has to say so instead of
// failing with a parse or transport error.
fn resolve_status_error(status: u16, body: &[u8]) -> Diagnostic {
    let value = parse_json(body).unwrap_or(JsonValue::Null);
    let code = json_string(&value, "code");
    let message = json_string(&value, "message");
    if !RESOLVE_KNOWN_STATUSES.contains(&status) {
        let mut text = format!(
            "registry replied HTTP {status}, which this client version does not understand - update the client"
        );
        if !message.is_empty() {
            text.push_str(": ");
            text.push_str(&message);
        } else if !code.is_empty() {
            text.push_str(&format!(" (code {code})"));
        }
        return Diagnostic::new(Code::E108, text);
    }
    let text = if !message.is_empty() {
        message
    } else if !code.is_empty() {
        format!("registry resolve failed ({code})")
    } else {
        format!("registry resolve failed with HTTP {status}")
    };
    Diagnostic::new(Code::E108, text)
}

pub fn resolve_requirements(
    base: &str,
    requirements: &BTreeMap<String, String>,
    have: &[HaveEntry],
    inline_chunks: bool,
) -> Result<ResolveResponse, Diagnostic> {
    for (name, range) in requirements {
        if !is_valid_range(range) {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "requirement for `{name}` is not a valid range (*, latest, X.Y.Z, ^, ~, >=)"
                ),
            ));
        }
    }
    let body = resolve_request_body(requirements, have, inline_chunks);
    let url = format!("{}/api/v0/resolve", base.trim_end_matches('/'));
    let reply = http_post_json(&url, &body, "registry resolve")?;
    if reply.status == 200 {
        return parse_resolve(&reply.body);
    }
    Err(resolve_status_error(reply.status, &reply.body))
}

fn parse_resolve(body: &[u8]) -> Result<ResolveResponse, Diagnostic> {
    let bad = || Diagnostic::new(Code::E108, "registry resolve returned malformed JSON");
    let value = parse_json(body).map_err(|_| bad())?;
    let mut resolved = BTreeMap::new();
    match value.get("resolved") {
        Some(JsonValue::Object(map)) => {
            for (k, v) in map {
                match v.as_str() {
                    Some(s) => {
                        resolved.insert(k.clone(), s.to_string());
                    }
                    None => return Err(bad()),
                }
            }
        }
        _ => return Err(bad()),
    }
    let base = value
        .get("base")
        .and_then(|v| v.as_str())
        .unwrap_or_default()
        .to_string();
    let mut levels: Vec<Vec<ResolveNode>> = Vec::new();
    match value.get("levels") {
        Some(JsonValue::Array(rows)) => {
            for row in rows {
                let items = row.as_array().ok_or_else(bad)?;
                let mut out = Vec::with_capacity(items.len());
                for item in items {
                    out.push(parse_node(item)?);
                }
                levels.push(out);
            }
        }
        _ => return Err(bad()),
    }
    Ok(ResolveResponse { resolved, base, levels })
}

fn parse_node(value: &JsonValue) -> Result<ResolveNode, Diagnostic> {
    let bad = || Diagnostic::new(Code::E108, "registry resolve returned malformed JSON");
    let obj = match value {
        JsonValue::Object(fields) => fields,
        _ => return Err(bad()),
    };
    let req = |key: &str| {
        obj.iter()
            .find(|(k, _)| k == key)
            .and_then(|(_, v)| v.as_str())
            .map(|s| s.to_string())
            .ok_or_else(bad)
    };
    let deps = match obj.iter().find(|(k, _)| k.as_str() == "deps").map(|(_, v)| v) {
        Some(JsonValue::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                out.push(item.as_str().ok_or_else(bad)?.to_string());
            }
            out
        }
        Some(_) => return Err(bad()),
        None => Vec::new(),
    };
    let yanked = obj
        .iter()
        .find(|(k, _)| k.as_str() == "yanked")
        .and_then(|(_, v)| v.as_bool())
        .unwrap_or(false);
    // Unknown to a registry without the capability, so the field is simply
    // absent and the download path falls back to the chunk manifest.
    let full = req("full")?;
    let version = req("version")?;
    let entries = match obj.iter().find(|(k, _)| k.as_str() == "entries").map(|(_, v)| v) {
        Some(JsonValue::Array(items)) => Some(parse_entries(
            items,
            &full,
            &version,
            "registry resolve returned malformed chunk entries",
        )?),
        _ => None,
    };
    Ok(ResolveNode {
        full,
        version,
        path: req("path")?,
        integrity: req("integrity")?,
        engine_range: obj
            .iter()
            .find(|(k, _)| k.as_str() == "engineRange")
            .and_then(|(_, v)| v.as_str())
            .unwrap_or_default()
            .to_string(),
        deps,
        yanked,
        artifact_digest: obj
            .iter()
            .find(|(k, _)| k.as_str() == "artifactDigest")
            .and_then(|(_, v)| v.as_str())
            .map(|s| s.to_string()),
        entries,
    })
}

pub(crate) fn host_of(base: &str) -> String {
    let without_scheme = match base.split_once("://") {
        Some((_, rest)) => rest,
        None => base,
    };
    let host = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = host.split('@').last().unwrap_or(host);
    sanitize_segment(host)
}

pub(crate) fn sanitize_segment(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.') {
            out.push(c);
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("registry");
    }
    out
}

pub(crate) fn sanitize_full(full: &str) -> String {
    let mut out = String::with_capacity(full.len());
    for c in full.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '@') {
            out.push(c);
        } else if c == '/' {
            out.push('/');
        } else {
            out.push('_');
        }
    }
    if out.is_empty() {
        out.push_str("pkg");
    }
    out
}

pub fn registry_cache_root() -> PathBuf {
    crate::cache::global_cache_dir().join("registry")
}

pub fn store_root() -> PathBuf {
    crate::cache::global_cache_dir().join("store")
}

fn store_cas_dir() -> PathBuf {
    store_root().join("cas")
}

fn store_pkgs_dir() -> PathBuf {
    store_root().join("pkgs")
}

fn store_index_path() -> PathBuf {
    store_root().join("registry.index.json")
}

fn normalize_hex64(raw: &str) -> Option<String> {
    let hex = raw.trim().to_ascii_lowercase();
    if hex.len() == 64 && hex.bytes().all(|b| b.is_ascii_hexdigit()) {
        Some(hex)
    } else {
        None
    }
}

pub fn cas_blob_path(hash: &str) -> Option<PathBuf> {
    let hex = normalize_hex64(hash)?;
    Some(store_cas_dir().join(&hex[..2]).join(&hex[2..]))
}

pub fn store_pkg_dir(integrity: &str) -> Option<PathBuf> {
    let hex = normalize_hex64(integrity)?;
    Some(store_pkgs_dir().join(hex))
}

pub fn cas_get(hash: &str) -> Result<Option<Vec<u8>>, Diagnostic> {
    let Some(path) = cas_blob_path(hash) else {
        return Err(Diagnostic::new(Code::E108, format!("bad chunk hash `{hash}`")));
    };
    let bytes = match std::fs::read(&path) {
        Ok(bytes) => bytes,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => {
            return Err(Diagnostic::new(
                Code::E108,
                format!("cannot read chunk `{hash}`: {e}"),
            ));
        }
    };
    if bytes.len() > MAX_CHUNK_BYTES {
        return Err(Diagnostic::new(
            Code::E108,
            format!("chunk {hash} exceeds size limits"),
        ));
    }
    if Sha256::hexdigest(&bytes) != hash.to_ascii_lowercase() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("chunk {hash} failed its integrity check"),
        ));
    }
    Ok(Some(bytes))
}

fn unique_tmp_name(tag: &str) -> String {
    static NEXT: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
    let n = NEXT.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
    let nanos = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_nanos())
        .unwrap_or(0);
    format!(".tmp-{tag}-{}-{nanos}-{n}", std::process::id())
}

#[cfg(unix)]
fn set_store_file_mode(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = if path.is_dir() { 0o755 } else { 0o644 };
    let _ = std::fs::set_permissions(path, std::fs::Permissions::from_mode(mode));
}

#[cfg(not(unix))]
fn set_store_file_mode(_path: &Path) {}

fn write_store_modes(root: &Path) {
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        set_store_file_mode(&dir);
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
        paths.sort();
        for path in paths {
            if path.is_dir() {
                stack.push(path);
            } else if path.is_file() {
                set_store_file_mode(&path);
            }
        }
    }
}

pub fn cas_put(hash: &str, bytes: &[u8]) -> Result<(), Diagnostic> {
    let Some(path) = cas_blob_path(hash) else {
        return Err(Diagnostic::new(Code::E108, format!("bad chunk hash `{hash}`")));
    };
    if bytes.len() > MAX_CHUNK_BYTES {
        return Err(Diagnostic::new(
            Code::E108,
            format!("chunk {hash} exceeds size limits"),
        ));
    }
    if Sha256::hexdigest(bytes) != hash.to_ascii_lowercase() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("chunk {hash} failed its integrity check"),
        ));
    }
    if matches!(cas_get(hash), Ok(Some(_))) {
        return Ok(());
    }
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", parent.display()))
        })?;
    }
    let tmp = path.with_file_name(format!("{}.{}", unique_tmp_name("cas"), "part"));
    std::fs::write(&tmp, bytes).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", tmp.display()))
    })?;
    set_store_file_mode(&tmp);
    match std::fs::rename(&tmp, &path) {
        Ok(()) => Ok(()),
        Err(_) => {
            if matches!(cas_get(hash), Ok(Some(_))) {
                let _ = std::fs::remove_file(&tmp);
                Ok(())
            } else {
                std::fs::rename(&tmp, &path).map_err(|e| {
                    Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", path.display()))
                })
            }
        }
    }
}

fn store_integrity_matches(dir: &Path, full: &str, version: &str, integrity: &str) -> bool {
    if !dir.join(crate::project::MANIFEST_FILE).is_file() {
        return false;
    }
    let text = match std::fs::read_to_string(integrity_path(dir)) {
        Ok(text) => text,
        Err(_) => return false,
    };
    let mut lines = text.lines();
    let ok = matches!((lines.next(), lines.next(), lines.next()), (Some(f), Some(v), Some(d))
        if f == full && v == version && d.trim().eq_ignore_ascii_case(integrity));
    if !ok {
        return false;
    }
    match dir.file_name().and_then(|n| n.to_str()) {
        Some(name) => name.eq_ignore_ascii_case(integrity),
        None => false,
    }
}

fn store_hit_for_node(node: &ResolveNode) -> Option<PathBuf> {
    let dir = store_pkg_dir(&node.integrity)?;
    if !store_integrity_matches(&dir, &node.full, &node.version, &node.integrity) {
        return None;
    }
    if !node_artifacts_ready(&dir, node.artifact_digest.as_deref()) {
        return None;
    }
    Some(dir)
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StoreIndexEntry {
    base: String,
    full: String,
    version: String,
    integrity: String,
}

fn read_store_index() -> Vec<StoreIndexEntry> {
    let text = match std::fs::read_to_string(store_index_path()) {
        Ok(text) => text,
        Err(_) => return Vec::new(),
    };
    let value = match parse_json(text.as_bytes()) {
        Ok(value) => value,
        Err(_) => return Vec::new(),
    };
    let Some(items) = value.get("entries").and_then(|v| v.as_array()) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for item in items {
        let field = |key: &str| item.get(key).and_then(|v| v.as_str()).unwrap_or_default();
        let (base, full, version, integrity) =
            (field("base"), field("full"), field("version"), field("integrity"));
        if base.is_empty() || full.is_empty() || version.is_empty() {
            continue;
        }
        let Some(hex) = normalize_hex64(integrity) else {
            continue;
        };
        let Some(dir) = store_pkg_dir(&hex) else {
            continue;
        };
        if !store_integrity_matches(&dir, full, version, &hex) {
            continue;
        }
        out.push(StoreIndexEntry {
            base: base.to_string(),
            full: full.to_string(),
            version: version.to_string(),
            integrity: hex,
        });
    }
    out.sort_by(|a, b| (&a.base, &a.full, &a.version).cmp(&(&b.base, &b.full, &b.version)));
    out.dedup_by(|a, b| a.base == b.base && a.full == b.full && a.version == b.version);
    out
}

fn note_store_install(base: &str, full: &str, version: &str, integrity: &str) {
    let Some(hex) = normalize_hex64(integrity) else {
        return;
    };
    let mut entries = read_store_index();
    if let Some(hit) =
        entries.iter_mut().find(|e| e.base == base && e.full == full && e.version == version)
    {
        hit.integrity = hex;
    } else {
        entries.push(StoreIndexEntry {
            base: base.to_string(),
            full: full.to_string(),
            version: version.to_string(),
            integrity: hex,
        });
    }
    entries.sort_by(|a, b| (&a.base, &a.full, &a.version).cmp(&(&b.base, &b.full, &b.version)));
    let mut body = String::from("{\"version\": 1, \"entries\": [");
    for (i, e) in entries.iter().enumerate() {
        if i > 0 {
            body.push_str(", ");
        }
        body.push_str(&format!(
            "{{\"base\": \"{}\", \"full\": \"{}\", \"version\": \"{}\", \"integrity\": \"{}\"}}",
            json_escape(&e.base),
            json_escape(&e.full),
            json_escape(&e.version),
            e.integrity
        ));
    }
    body.push_str("]}");
    let path = store_index_path();
    if let Some(parent) = path.parent() {
        if std::fs::create_dir_all(parent).is_err() {
            return;
        }
    }
    let tmp = path.with_file_name(unique_tmp_name("index"));
    if std::fs::write(&tmp, body.as_bytes()).is_err() {
        return;
    }
    set_store_file_mode(&tmp);
    if std::fs::rename(&tmp, &path).is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
}

pub fn find_store_pkg(base: &str, full: &str, version: &str) -> Option<PathBuf> {
    for entry in read_store_index() {
        if entry.base == base && entry.full == full && entry.version == version {
            let dir = store_pkg_dir(&entry.integrity)?;
            if store_integrity_matches(&dir, full, version, &entry.integrity) {
                return Some(dir);
            }
        }
    }
    let root = store_pkgs_dir();
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(_) => return None,
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    for name in names {
        let Some(hex) = normalize_hex64(&name) else {
            continue;
        };
        let Some(dir) = store_pkg_dir(&hex) else {
            continue;
        };
        if store_integrity_matches(&dir, full, version, &hex) {
            return Some(dir);
        }
    }
    None
}

pub fn store_have() -> Vec<HaveEntry> {
    let root = store_pkgs_dir();
    let entries = match std::fs::read_dir(&root) {
        Ok(entries) => entries,
        Err(_) => return Vec::new(),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .map(|e| e.file_name().to_string_lossy().into_owned())
        .collect();
    names.sort();
    let mut out = Vec::new();
    for name in names {
        let Some(hex) = normalize_hex64(&name) else {
            continue;
        };
        let Some(dir) = store_pkg_dir(&hex) else {
            continue;
        };
        let text = match std::fs::read_to_string(integrity_path(&dir)) {
            Ok(text) => text,
            Err(_) => continue,
        };
        let mut lines = text.lines();
        match (lines.next(), lines.next(), lines.next()) {
            (Some(full), Some(version), Some(integrity))
                if !full.is_empty() && !version.is_empty() && !integrity.is_empty() =>
            {
                if !integrity.trim().eq_ignore_ascii_case(&hex) {
                    continue;
                }
                out.push(HaveEntry {
                    full: full.to_string(),
                    version: version.to_string(),
                    integrity: integrity.trim().to_string(),
                });
            }
            _ => continue,
        }
    }
    out.sort_by(|a, b| (&a.full, &a.version).cmp(&(&b.full, &b.version)));
    out.dedup_by(|a, b| a.full == b.full && a.version == b.version);
    out
}

pub fn find_cached_package_dir(base: &str, full: &str, version: &str) -> Option<PathBuf> {
    find_store_pkg(base, full, version)
}

fn dir_tree_usage_bytes(root: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![root.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = match std::fs::read_dir(&dir) {
            Ok(entries) => entries,
            Err(_) => continue,
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let meta = match std::fs::symlink_metadata(&path) {
                Ok(meta) => meta,
                Err(_) => continue,
            };
            if meta.file_type().is_symlink() {
                continue;
            }
            if meta.file_type().is_dir() {
                stack.push(path);
            } else if meta.file_type().is_file() {
                total += meta.len();
            }
        }
    }
    total
}

pub fn prune_old_registry_tree() -> u64 {
    let root = registry_cache_root();
    let freed = dir_tree_usage_bytes(&root);
    if std::fs::remove_dir_all(&root).is_err() {
        return 0;
    }
    freed
}

pub fn store_usage_bytes() -> u64 {
    dir_tree_usage_bytes(&store_root())
}

fn integrity_path(dir: &Path) -> PathBuf {
    dir.join(".rnx-integrity")
}

fn artifact_marker_path(dir: &Path) -> PathBuf {
    dir.join(".rnx-artifact")
}

pub fn cached_artifact_digest(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(artifact_marker_path(dir)).ok()?;
    let digest = text.lines().next().unwrap_or_default().trim().to_string();
    if crate::oci::is_artifact_digest(&digest) {
        Some(digest)
    } else {
        None
    }
}

fn node_artifacts_ready(dir: &Path, expected: Option<&str>) -> bool {
    match expected {
        None => true,
        Some(digest) => {
            cached_artifact_digest(dir).as_deref() == Some(digest)
                && crate::oci::project_artifact_cache(dir).is_dir()
        }
    }
}

fn cached_artifacts_present(dir: &Path) -> bool {
    cached_artifact_digest(dir).is_some() && crate::oci::project_artifact_cache(dir).is_dir()
}

fn ensure_node_artifacts(dir: &Path, node: &ResolveNode, asserted: &str) -> Result<(), Diagnostic> {
    warn_if_yanked(node);
    let cfg = ProjectConfig::load_from_dir(dir)?.ok_or_else(|| {
        Diagnostic::new(
            Code::E108,
            format!("package `{}@{}` has no Project.config", node.full, node.version),
        )
    })?;
    let bare = match cfg.artifacts.as_deref() {
        Some(bare) => bare,
        None => {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "package `{}@{}` pins artifact {asserted} but declares no `artifacts`",
                    node.full, node.version
                ),
            ));
        }
    };
    crate::oci::ensure_artifacts(dir, bare, asserted, std::env::consts::OS, std::env::consts::ARCH)?;
    std::fs::write(artifact_marker_path(dir), format!("{asserted}\n")).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", artifact_marker_path(dir).display()))
    })
}

pub fn cached_integrity(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(integrity_path(dir)).ok()?;
    let mut lines = text.lines();
    lines.next()?;
    lines.next()?;
    lines.next().map(|s| s.trim().to_string())
}

pub fn scan_cache_have() -> Vec<HaveEntry> {
    store_have()
}

fn engine_error(full: &str, version: &str, range: &str) -> Diagnostic {
    let current = env!("CARGO_PKG_VERSION");
    Diagnostic::new(
        Code::E114,
        format!("`{full}@{version}` needs engine {range}, have {current}"),
    )
    .with_hint("run `rnx update` to move to a compatible version, or `rnx download` to re-seed the cache")
}

fn check_engine(node: &ResolveNode) -> Result<(), Diagnostic> {
    let range = node.engine_range.trim();
    if range.is_empty() {
        return Ok(());
    }
    if engine_satisfied(range, env!("CARGO_PKG_VERSION")) {
        return Ok(());
    }
    Err(engine_error(&node.full, &node.version, range))
}

fn check_cached_engine(cfg: &ProjectConfig, full: &str, version: &str) -> Result<(), Diagnostic> {
    let range = cfg.engine.trim();
    if range.is_empty() {
        return Ok(());
    }
    if engine_satisfied(range, env!("CARGO_PKG_VERSION")) {
        return Ok(());
    }
    Err(engine_error(full, version, range))
}

fn warn_if_yanked(node: &ResolveNode) {
    if node.yanked {
        warn_yanked(&node.full, &node.version);
    }
}

fn warn_yanked(full: &str, version: &str) {
    eprintln!("warning: package `{full}@{version}` is yanked and should be avoided");
}

fn yanked_marker_path(dir: &Path) -> PathBuf {
    dir.join(".rnx-yanked")
}

fn cached_is_yanked(dir: &Path) -> bool {
    yanked_marker_path(dir).is_file()
}

fn warn_if_cached_yanked(dir: &Path, full: &str, version: &str) {
    if cached_is_yanked(dir) {
        warn_yanked(full, version);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChunkEntry {
    name: String,
    size: u64,
    dir: bool,
    chunks: Vec<String>,
}

fn decode_chunk_manifest_body(
    body: &[u8],
    encoding: Option<&str>,
    full: &str,
    version: &str,
) -> Result<Vec<u8>, Diagnostic> {
    let tag = encoding.unwrap_or("").trim();
    if tag.is_empty() || tag.eq_ignore_ascii_case("identity") {
        return Ok(body.to_vec());
    }
    if tag.eq_ignore_ascii_case("gzip") {
        let out = crate::gzip::decompress_gzip(body).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!(
                    "chunk manifest of `{full}@{version}` claims gzip content encoding but cannot be decoded: {}",
                    e.message
                ),
            )
        })?;
        if out.len() > MAX_DOWNLOAD_BYTES {
            return Err(Diagnostic::new(
                Code::E108,
                format!("chunk manifest of `{full}@{version}` exceeds size limits"),
            ));
        }
        return Ok(out);
    }
    Err(Diagnostic::new(
        Code::E108,
        format!("chunk manifest of `{full}@{version}` uses unsupported content encoding `{tag}`"),
    ))
}

fn check_chunk_entry_cap(count: usize, full: &str, version: &str) -> Result<(), Diagnostic> {
    if count > MAX_CHUNK_ENTRIES {
        return Err(Diagnostic::new(
            Code::E116,
            format!(
                "chunk manifest of `{full}@{version}` lists {count} entries, over the {MAX_CHUNK_ENTRIES}-per-version limit"
            ),
        )
        .with_hint("the registry refuses versions over 4096 entries; run `rnx update` to move past this version or ask the maintainer to split the package"));
    }
    Ok(())
}

fn parse_entries(items: &[JsonValue], full: &str, version: &str, bad: &str) -> Result<Vec<ChunkEntry>, Diagnostic> {
    check_chunk_entry_cap(items.len(), full, version)?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let obj = match item {
            JsonValue::Object(fields) => fields,
            _ => return Err(Diagnostic::new(Code::E108, bad)),
        };
        let str_field = |key: &str| {
            obj.iter()
                .find(|(k, _)| k == key)
                .and_then(|(_, v)| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| Diagnostic::new(Code::E108, bad))
        };
        let size = obj
            .iter()
            .find(|(k, _)| k == "size")
            .and_then(|(_, v)| v.as_u64())
            .ok_or_else(|| Diagnostic::new(Code::E108, bad))?;
        let dir = obj
            .iter()
            .find(|(k, _)| k == "dir")
            .and_then(|(_, v)| v.as_bool())
            .unwrap_or(false);
        let hashes = match obj.iter().find(|(k, _)| k == "chunks").map(|(_, v)| v) {
            Some(JsonValue::Array(hashes)) => {
                let mut out = Vec::with_capacity(hashes.len());
                for h in hashes {
                    out.push(
                        h.as_str()
                            .ok_or_else(|| Diagnostic::new(Code::E108, bad))?
                            .to_string(),
                    );
                }
                out
            }
            _ => return Err(Diagnostic::new(Code::E108, bad)),
        };
        out.push(ChunkEntry { name: str_field("name")?, size, dir, chunks: hashes });
    }
    Ok(out)
}

fn fetch_chunks_manifest(url: &str, full: &str, version: &str, direct: bool) -> Result<Vec<ChunkEntry>, Diagnostic> {
    let bad = format!("chunk manifest of `{full}@{version}` is malformed");
    let fetched =
        http_manifest_get(&direct_url(url, direct), &format!("chunk manifest of `{full}@{version}`"))?;
    if fetched.reply.status != 200 {
        return Err(chunk_fetch_error(fetched.reply.status, &fetched.reply.body, full, version));
    }
    let raw = decode_chunk_manifest_body(
        &fetched.reply.body,
        fetched.content_encoding.as_deref(),
        full,
        version,
    )?;
    let value = parse_json(&raw).map_err(|_| Diagnostic::new(Code::E108, bad.clone()))?;
    let items = value
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| Diagnostic::new(Code::E108, bad.clone()))?;
    parse_entries(items, full, version, &bad)
}

fn fetch_chunk(url: &str, hash: &str, full: &str, version: &str, direct: bool) -> Result<Vec<u8>, Diagnostic> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Diagnostic::new(Code::E108, format!("bad chunk hash `{hash}`")));
    }
    let reply = http_get(&direct_url(url, direct), &format!("chunk {hash} of `{full}@{version}`"))?;
    if reply.status != 200 {
        return Err(Diagnostic::new(
            Code::E108,
            format!("chunk {hash} of `{full}@{version}` is missing"),
        ));
    }
    if reply.body.len() > MAX_CHUNK_BYTES {
        return Err(Diagnostic::new(
            Code::E108,
            format!("chunk {hash} of `{full}@{version}` exceeds size limits"),
        ));
    }
    if Sha256::hexdigest(&reply.body) != hash.to_ascii_lowercase() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("chunk {hash} of `{full}@{version}` failed its integrity check"),
        ));
    }
    Ok(reply.body)
}

struct ChunkBudget {
    slots: std::sync::Mutex<usize>,
    changed: std::sync::Condvar,
}

impl ChunkBudget {
    fn new(limit: usize) -> ChunkBudget {
        ChunkBudget { slots: std::sync::Mutex::new(limit.max(1)), changed: std::sync::Condvar::new() }
    }

    fn acquire(&self) -> BudgetTicket<'_> {
        let mut slots = self.slots.lock().unwrap_or_else(|e| e.into_inner());
        while *slots == 0 {
            slots = self.changed.wait(slots).unwrap_or_else(|e| e.into_inner());
        }
        *slots -= 1;
        BudgetTicket { budget: self }
    }
}

struct BudgetTicket<'a> {
    budget: &'a ChunkBudget,
}

impl Drop for BudgetTicket<'_> {
    fn drop(&mut self) {
        let mut slots = self.budget.slots.lock().unwrap_or_else(|e| e.into_inner());
        *slots += 1;
        self.budget.changed.notify_one();
    }
}

struct FetchShare {
    chunks: std::sync::Mutex<BTreeMap<String, Vec<u8>>>,
    budget: ChunkBudget,
}

impl FetchShare {
    fn new() -> FetchShare {
        FetchShare {
            chunks: std::sync::Mutex::new(BTreeMap::new()),
            budget: ChunkBudget::new(GLOBAL_CHUNK_INFLIGHT),
        }
    }

    fn fetch_chunk_shared(
        &self,
        chunk_base: &str,
        hash: &str,
        full: &str,
        version: &str,
        direct: bool,
    ) -> Result<Vec<u8>, Diagnostic> {
        if let Some(hit) = self.chunks.lock().unwrap_or_else(|e| e.into_inner()).get(hash) {
            return Ok(hit.clone());
        }
        let _ticket = self.budget.acquire();
        if let Some(hit) = self.chunks.lock().unwrap_or_else(|e| e.into_inner()).get(hash) {
            return Ok(hit.clone());
        }
        match cas_get(hash) {
            Ok(Some(bytes)) => {
                self.chunks
                    .lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .insert(hash.to_string(), bytes.clone());
                return Ok(bytes);
            }
            Ok(None) => {}
            Err(_) => {
                let _ = cas_blob_path(hash).map(std::fs::remove_file);
            }
        }
        let url = format!("{chunk_base}/chunk/{hash}");
        let bytes = fetch_chunk(&url, hash, full, version, direct)?;
        let _ = cas_put(hash, &bytes);
        self.chunks
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(hash.to_string(), bytes.clone());
        Ok(bytes)
    }
}

fn fetch_chunks_parallel(
    chunk_base: &str,
    hashes: &[String],
    full: &str,
    version: &str,
    direct: bool,
    share: &FetchShare,
) -> Result<BTreeMap<String, Vec<u8>>, Diagnostic> {
    if hashes.is_empty() {
        return Ok(BTreeMap::new());
    }
    let width = CHUNK_FETCH_CONCURRENCY.min(hashes.len()).max(1);
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Result<Vec<u8>, Diagnostic>>>> =
        (0..hashes.len()).map(|_| std::sync::Mutex::new(None)).collect();
    std::thread::scope(|s| {
        for _ in 0..width {
            s.spawn(|| loop {
                let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(hash) = hashes.get(i) else {
                    break;
                };
                let got = share.fetch_chunk_shared(chunk_base, hash, full, version, direct);
                let mut slot = slots[i].lock().unwrap_or_else(|e| e.into_inner());
                *slot = Some(got);
            });
        }
    });
    let mut out: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    for (i, hash) in hashes.iter().enumerate() {
        let mut slot = slots[i].lock().unwrap_or_else(|e| e.into_inner());
        match slot.take() {
            Some(Ok(bytes)) => {
                out.insert(hash.clone(), bytes);
            }
            Some(Err(e)) => return Err(e),
            None => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("chunk {hash} of `{full}@{version}` was not fetched"),
                ));
            }
        }
    }
    Ok(out)
}

fn chunk_fetch_error(status: u16, body: &[u8], full: &str, version: &str) -> Diagnostic {
    match status {
        404 => {
            let value = parse_json(body).unwrap_or(JsonValue::Null);
            if json_string(&value, "code") == "no-file-index" {
                return Diagnostic::new(
                    Code::E108,
                    format!("package `{full}@{version}` ships no file index"),
                );
            }
            Diagnostic::new(
                Code::E108,
                format!("package `{full}@{version}` does not exist"),
            )
        }
        410 => {
            let value = parse_json(body).unwrap_or(JsonValue::Null);
            let reason = json_string(&value, "reason");
            if reason.is_empty() {
                Diagnostic::new(
                    Code::E108,
                    format!("package `{full}@{version}` was withdrawn; upgrade to a newer version"),
                )
            } else {
                Diagnostic::new(
                    Code::E108,
                    format!("package `{full}@{version}` was withdrawn ({reason}); upgrade to a newer version"),
                )
            }
        }
        _ => {
            let value = parse_json(body).unwrap_or(JsonValue::Null);
            let message = json_string(&value, "message");
            if message.is_empty() {
                Diagnostic::new(
                    Code::E108,
                    format!("fetch of `{full}@{version}` failed with HTTP {status}"),
                )
            } else {
                Diagnostic::new(Code::E108, message)
            }
        }
    }
}

fn write_tar_octal(buf: &mut [u8], off: usize, len: usize, value: u64) {
    let digits = format!("{value:o}");
    let pad = len.saturating_sub(1 + digits.len());
    for i in 0..pad {
        buf[off + i] = b'0';
    }
    for (i, c) in digits.bytes().enumerate() {
        buf[off + pad + i] = c;
    }
    buf[off + len - 1] = 0;
}

fn tar_entry_header(entry: &ChunkEntry, full: &str, version: &str) -> Result<[u8; 512], Diagnostic> {
    let name = entry.name.as_bytes();
    if name.len() > 100 {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "chunk manifest of `{full}@{version}` has an overlong file name `{}`",
                entry.name
            ),
        ));
    }
    let mut head = [0u8; 512];
    head[..name.len()].copy_from_slice(name);
    write_tar_octal(&mut head, 100, 8, if entry.dir { 0o755 } else { 0o644 });
    write_tar_octal(&mut head, 108, 8, 0);
    write_tar_octal(&mut head, 116, 8, 0);
    write_tar_octal(&mut head, 124, 12, entry.size);
    write_tar_octal(&mut head, 136, 12, 0);
    for b in head.iter_mut().skip(148).take(8) {
        *b = b' ';
    }
    head[156] = if entry.dir { b'5' } else { b'0' };
    head[257..263].copy_from_slice(b"ustar\0");
    head[263..265].copy_from_slice(b"00");
    head[265..268].copy_from_slice(b"rnx");
    head[297..300].copy_from_slice(b"rnx");
    let sum: u64 = head.iter().map(|b| *b as u64).sum();
    let sum_text = format!("{sum:06o}");
    head[148..154].copy_from_slice(&sum_text.as_bytes()[..6]);
    head[154] = 0;
    head[155] = b' ';
    Ok(head)
}

struct DirectTree {
    files: BTreeMap<String, Vec<u8>>,
    dirs: BTreeSet<String>,
    digest: String,
}

fn feed_capped(
    hash: &mut Sha256,
    bytes: &[u8],
    total: &mut u64,
    full: &str,
    version: &str,
) -> Result<(), Diagnostic> {
    *total += bytes.len() as u64;
    if *total > MAX_DOWNLOAD_BYTES as u64 {
        return Err(Diagnostic::new(
            Code::E108,
            format!("download of `{full}@{version}` exceeds size limits"),
        ));
    }
    hash.update(bytes);
    Ok(())
}

fn verify_manifest_digest(entries: &[ChunkEntry], node: &ResolveNode) -> Result<(), Diagnostic> {
    let digest_entries: Vec<crate::checksum::DigestEntry> = entries
        .iter()
        .map(|e| crate::checksum::DigestEntry {
            name: e.name.clone(),
            size: e.size,
            dir: e.dir,
            chunks: e.chunks.clone(),
        })
        .collect();
    let digest = crate::checksum::manifest_digest_hex(&digest_entries).map_err(|_| {
        Diagnostic::new(
            Code::E108,
            format!(
                "chunk manifest of `{}@{}` carries a malformed chunk hash",
                node.full, node.version
            ),
        )
    })?;
    if !digest.eq_ignore_ascii_case(&node.integrity) {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "checksum mismatch for package `{}@{}`: download does not match registry integrity",
                node.full, node.version
            ),
        ));
    }
    Ok(())
}

fn rehydrate_manifest_version(dir: &Path, full: &str, version: &str) -> Result<(), Diagnostic> {
    let path = dir.join(crate::project::MANIFEST_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display()))
    })?;
    let rehydrated =
        crate::project::redact::rehydrate_project_version(&text, version).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("package `{full}` ships a Project.config without the published version marker"),
            )
            .with_hint(e.message.clone())
        })?;
    std::fs::write(&path, rehydrated).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", path.display()))
    })?;
    Ok(())
}

fn assemble_direct_tree(
    entries: &[ChunkEntry],
    by_hash: &BTreeMap<String, Vec<u8>>,
    full: &str,
    version: &str,
) -> Result<DirectTree, Diagnostic> {
    let mut files: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    let mut hash = Sha256::new();
    let mut total: u64 = 0;
    const ZEROS: [u8; 1024] = [0; 1024];
    for e in entries {
        if e.name.is_empty() {
            return Err(Diagnostic::new(
                Code::E108,
                format!("chunk manifest of `{full}@{version}` has a file entry with an empty name"),
            ));
        }
        let path = tar_safe_rel(&e.name).map_err(|_| {
            Diagnostic::new(
                Code::E108,
                format!("unsafe file path `{}` in chunk manifest of `{full}@{version}`", e.name),
            )
        })?;
        if e.dir {
            if !e.chunks.is_empty() {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "checksum mismatch for file `{path}` of `{full}@{version}`: directory entry lists content chunks"
                    ),
                ));
            }
            let head = tar_entry_header(e, full, version)?;
            feed_capped(&mut hash, &head, &mut total, full, version)?;
            dirs.insert(path);
            continue;
        }
        let mut data = Vec::new();
        for h in &e.chunks {
            match by_hash.get(h) {
                Some(bytes) => data.extend_from_slice(bytes),
                None => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!(
                            "chunk manifest of `{full}@{version}` failed to reassemble file `{path}`: chunk `{h}` is missing"
                        ),
                    ));
                }
            }
        }
        if data.len() as u64 != e.size {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "checksum mismatch for file `{path}` of `{full}@{version}`: manifest says {} bytes but chunks hold {}",
                    e.size,
                    data.len()
                ),
            ));
        }
        let head = tar_entry_header(e, full, version)?;
        feed_capped(&mut hash, &head, &mut total, full, version)?;
        feed_capped(&mut hash, &data, &mut total, full, version)?;
        let pad = (512 - (e.size % 512)) % 512;
        feed_capped(&mut hash, &ZEROS[..pad as usize], &mut total, full, version)?;
        files.insert(path, data);
    }
    feed_capped(&mut hash, &ZEROS, &mut total, full, version)?;
    Ok(DirectTree { files, dirs, digest: crate::checksum::hex_of(&hash.finish()) })
}

fn write_direct_tree(tree: &DirectTree, full: &str, dest: &Path) -> Result<(), Diagnostic> {
    for dir in &tree.dirs {
        std::fs::create_dir_all(dest.join(dir)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dest.display()))
        })?;
    }
    let mut parents: BTreeSet<&str> = BTreeSet::new();
    for path in tree.files.keys() {
        if let Some((parent, _)) = path.rsplit_once('/') {
            parents.insert(parent);
        }
    }
    for parent in parents {
        std::fs::create_dir_all(dest.join(parent)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dest.display()))
        })?;
    }
    for (path, data) in &tree.files {
        std::fs::write(dest.join(path), data).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot write `{}`: {e}", dest.join(path).display()),
            )
        })?;
    }
    if !dest.join(crate::project::MANIFEST_FILE).is_file() {
        let _ = std::fs::remove_dir_all(dest);
        return Err(Diagnostic::new(
            Code::E108,
            format!("package `{full}` from the registry has no Project.config"),
        ));
    }
    Ok(())
}

// The accepted-name rule, as the client applies it. The server applies the
// same rule to the same names (`website/src/lib/server/chunks.ts::
// checkManifestEntryName`); the two implementations must stay textually
// identical.
fn tar_safe_rel(name: &str) -> Result<String, Diagnostic> {
    let bad = |reason: &str| {
        Diagnostic::new(Code::E108, format!("unsafe tar path `{name}`: {reason}"))
    };
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() {
        return Err(bad("the name is empty"));
    }
    if trimmed.len() > 255 {
        return Err(bad("the name is longer than 255 bytes"));
    }
    if trimmed.contains('\\') {
        return Err(bad("the name holds a backslash"));
    }
    if trimmed.starts_with('/') {
        return Err(bad("the name is absolute"));
    }
    if trimmed.len() >= 2 && trimmed.as_bytes()[1] == b':' && trimmed.as_bytes()[0].is_ascii_alphabetic() {
        return Err(bad("the name carries a drive prefix"));
    }
    if trimmed.chars().any(|c| c.is_control()) {
        return Err(bad("the name holds a control character"));
    }
    for part in trimmed.split('/') {
        if part.is_empty() {
            return Err(bad("the name holds an empty path component"));
        }
        if part == "." || part == ".." {
            return Err(bad(&format!("the name holds a `{part}` path component")));
        }
    }
    Ok(trimmed.to_string())
}

fn origin_of(url: &str) -> String {
    match url.split_once("://") {
        Some((scheme, rest)) => {
            let host = rest.split('/').next().unwrap_or(rest);
            format!("{scheme}://{host}")
        }
        None => url.to_string(),
    }
}

fn api_base_for(response_base: &str, request_base: &str) -> String {
    let b = response_base.trim();
    if b.is_empty() {
        return format!("{}/api/v0/packages", request_base.trim_end_matches('/'));
    }
    if let Some(path) = b.strip_prefix('/') {
        return format!(
            "{}/{}",
            origin_of(request_base).trim_end_matches('/'),
            path.trim_matches('/')
        );
    }
    b.trim_end_matches('/').to_string()
}

fn install_lock() -> &'static std::sync::Mutex<()> {
    static LOCK: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    LOCK.get_or_init(|| std::sync::Mutex::new(()))
}

fn read_store_identity(dir: &Path) -> Option<(String, String, String)> {
    let text = std::fs::read_to_string(integrity_path(dir)).ok()?;
    let mut lines = text.lines();
    match (lines.next(), lines.next(), lines.next()) {
        (Some(full), Some(version), Some(integrity))
            if !full.is_empty() && !version.is_empty() && !integrity.is_empty() =>
        {
            Some((full.to_string(), version.to_string(), integrity.trim().to_string()))
        }
        _ => None,
    }
}

fn adopt_store_tree(node: &ResolveNode, dest: &Path, base: &str) -> Result<PathBuf, Diagnostic> {
    let path = dest.join(crate::project::MANIFEST_FILE);
    let text = std::fs::read_to_string(&path).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display()))
    })?;
    let restamped = crate::project::redact::set_project_version(&text, &node.version).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("package `{}` ships a Project.config without the published version marker", node.full),
        )
        .with_hint(e.message.clone())
    })?;
    std::fs::write(&path, restamped).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", path.display()))
    })?;
    set_store_file_mode(&path);
    std::fs::write(integrity_path(dest), format!("{}\n{}\n{}\n", node.full, node.version, node.integrity))
        .map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dest.display()))
        })?;
    set_store_file_mode(&integrity_path(dest));
    if node.yanked {
        std::fs::write(yanked_marker_path(dest), "yanked\n").map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dest.display()))
        })?;
    } else {
        let _ = std::fs::remove_file(yanked_marker_path(dest));
    }
    note_store_install(base, &node.full, &node.version, &node.integrity);
    if let Some(digest) = node.artifact_digest.as_deref() {
        ensure_node_artifacts(dest, node, digest)?;
    }
    canonical_cached(dest)
}

fn install_store_tree(node: &ResolveNode, tree: &DirectTree, base: &str) -> Result<PathBuf, Diagnostic> {
    let Some(dest) = store_pkg_dir(&node.integrity) else {
        return Err(Diagnostic::new(
            Code::E108,
            format!("registry sent a bad integrity digest for `{}@{}`", node.full, node.version),
        ));
    };
    let _guard = install_lock().lock().unwrap_or_else(|e| e.into_inner());
    if let Some(hit) = store_hit_for_node(node) {
        return canonical_cached(&hit);
    }
    if dest.is_dir() {
        match read_store_identity(&dest) {
            Some((full, _, integrity)) if integrity.eq_ignore_ascii_case(&node.integrity) => {
                if full != node.full {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!(
                            "checksum mismatch for package `{}@{}`: the store holds different bytes under this integrity",
                            node.full, node.version
                        ),
                    ));
                }
                if dest.join(crate::project::MANIFEST_FILE).is_file() {
                    return adopt_store_tree(node, &dest, base);
                }
            }
            _ => {}
        }
        let _ = std::fs::remove_dir_all(&dest);
    }
    let parent = store_pkgs_dir();
    std::fs::create_dir_all(&parent).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", parent.display()))
    })?;
    let tmp = parent.join(unique_tmp_name("pkg"));
    write_direct_tree(tree, &node.full, &tmp)?;
    rehydrate_manifest_version(&tmp, &node.full, &node.version)?;
    std::fs::write(integrity_path(&tmp), format!("{}\n{}\n{}\n", node.full, node.version, node.integrity))
        .map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", tmp.display()))
        })?;
    if node.yanked {
        std::fs::write(yanked_marker_path(&tmp), "yanked\n").map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", tmp.display()))
        })?;
    }
    write_store_modes(&tmp);
    if store_hit_for_node(node).is_some() {
        let _ = std::fs::remove_dir_all(&tmp);
        return canonical_cached(&dest);
    }
    match std::fs::rename(&tmp, &dest) {
        Ok(()) => {}
        Err(_) => {
            if let Some(hit) = store_hit_for_node(node) {
                let _ = std::fs::remove_dir_all(&tmp);
                return canonical_cached(&hit);
            }
            let _ = std::fs::remove_dir_all(&tmp);
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "checksum mismatch for package `{}@{}`: the store holds different bytes under this integrity",
                    node.full, node.version
                ),
            ));
        }
    }
    if node.yanked {
        std::fs::write(yanked_marker_path(&dest), "yanked\n").map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dest.display()))
        })?;
    } else {
        let _ = std::fs::remove_file(yanked_marker_path(&dest));
    }
    note_store_install(base, &node.full, &node.version, &node.integrity);
    if let Some(digest) = node.artifact_digest.as_deref() {
        ensure_node_artifacts(&dest, node, digest)?;
    }
    canonical_cached(&dest)
}

fn ensure_node(
    cache_root: &Path,
    base: &str,
    api_base: &str,
    node: &ResolveNode,
    share: &FetchShare,
) -> Result<PathBuf, Diagnostic> {
    let global = cache_root == store_root();
    if global {
        if let Some(hit) = store_hit_for_node(node) {
            check_engine(node)?;
            warn_if_yanked(node);
            return canonical_cached(&hit);
        }
    } else {
        let dir = package_dir_under(cache_root, base, &node.full, &node.version);
        if dir.join(crate::project::MANIFEST_FILE).is_file()
            && cached_integrity(&dir).as_deref() == Some(node.integrity.as_str())
            && node_artifacts_ready(&dir, node.artifact_digest.as_deref())
        {
            check_engine(node)?;
            warn_if_yanked(node);
            return canonical_cached(&dir);
        }
    }
    check_engine(node)?;
    warn_if_yanked(node);
    let manifest_url = format!(
        "{}/{}",
        api_base.trim_end_matches('/'),
        node.path.trim_start_matches('/')
    );
    let chunk_base = manifest_url
        .strip_suffix("/chunks")
        .ok_or_else(|| {
            Diagnostic::new(
                Code::E108,
                format!("registry sent a bad chunk manifest path for `{}@{}`", node.full, node.version),
            )
        })?
        .to_string();
    let outer_direct = origin_direct();
    let download = |retry_direct: bool| -> Result<DirectTree, Diagnostic> {
        // The chunk manifest is the generation-sensitive read: a stale edge
        // copy lists chunk hashes from a generation that no longer matches
        // the integrity resolve just returned, so it always follows the
        // origin-direct guard. Chunk bodies are not generation-sensitive:
        // each is addressed by its own SHA-256, so any cached copy of
        // `{base}/chunk/{hash}` is byte-identical to the origin row and
        // `fetch_chunk` rejects anything that is not. Reading them through
        // the edge namespace keeps them on the immutable cache instead of
        // paying a no-store origin read per chunk, which is most of a cold
        // seed. A wrong or missing edge chunk still fails the hash check,
        // which the guarded retry below re-reads origin-direct.
        let manifest_direct = outer_direct || retry_direct;
        let chunk_direct = retry_direct;
        // Inline entries ride along with resolve and cost no request; the
        // guarded retry always re-reads the chunk manifest origin-direct, so
        // an entry list from a stale generation still fails into that path
        // instead of silently installing the wrong bytes.
        let inlined = if retry_direct { None } else { node.entries.as_ref() };
        let entries: Vec<ChunkEntry> = match inlined {
            Some(entries) => entries.clone(),
            None => fetch_chunks_manifest(&manifest_url, &node.full, &node.version, manifest_direct)?,
        };
        // A version with no file index is not a broken download: the
        // registry never shipped one. Saying that keeps it apart from the
        // checksum error a corrupt body would produce.
        if entries.is_empty() {
            return Err(Diagnostic::new(
                Code::E108,
                format!("package `{}@{}` ships no file index", node.full, node.version),
            ));
        }
        let mut uniq: BTreeSet<String> = BTreeSet::new();
        for entry in &entries {
            for hash in &entry.chunks {
                uniq.insert(hash.clone());
            }
        }
        let ordered: Vec<String> = uniq.into_iter().collect();
        let by_hash = fetch_chunks_parallel(&chunk_base, &ordered, &node.full, &node.version, chunk_direct, share)?;
        verify_manifest_digest(&entries, node)?;
        let tree = assemble_direct_tree(&entries, &by_hash, &node.full, &node.version)?;
        Ok(tree)
    };
    let downloaded = match download(false) {
        Err(first) if is_generation_error(&first) => {
            download(true)
        }
        other => other,
    }?;
    if global {
        return install_store_tree(node, &downloaded, base);
    }
    let dir = package_dir_under(cache_root, base, &node.full, &node.version);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", parent.display()))
        })?;
    }
    write_direct_tree(&downloaded, &node.full, &dir)?;
    rehydrate_manifest_version(&dir, &node.full, &node.version)?;
    std::fs::write(integrity_path(&dir), format!("{}\n{}\n{}\n", node.full, node.version, node.integrity))
        .map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dir.display()))
        })?;
    if node.yanked {
        std::fs::write(yanked_marker_path(&dir), "yanked\n").map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dir.display()))
        })?;
    } else {
        let _ = std::fs::remove_file(yanked_marker_path(&dir));
    }
    if let Some(digest) = node.artifact_digest.as_deref() {
        ensure_node_artifacts(&dir, node, digest)?;
    }
    canonical_cached(&dir)
}

fn canonical_cached(dir: &Path) -> Result<PathBuf, Diagnostic> {
    std::fs::canonicalize(dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", dir.display())))
}

pub fn ensure_registry_requirements(
    requirements: &BTreeMap<String, String>,
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
    have: &[HaveEntry],
) -> Result<Vec<FetchedPackage>, Diagnostic> {
    ensure_registry_requirements_in(
        &store_root(),
        requirements,
        default_registry,
        overrides,
        have,
    )
}

pub fn ensure_registry_requirements_in(
    cache_root: &Path,
    requirements: &BTreeMap<String, String>,
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
    have: &[HaveEntry],
) -> Result<Vec<FetchedPackage>, Diagnostic> {
    let mut by_base: BTreeMap<String, BTreeMap<String, String>> = BTreeMap::new();
    for (full, range) in requirements {
        if !is_valid_range(range) {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "requirement for `{full}` is not a valid range (*, latest, X.Y.Z, ^, ~, >=)"
                ),
            ));
        }
        by_base
            .entry(registry_base_for(full, default_registry, overrides))
            .or_default()
            .insert(full.clone(), range.clone());
    }
    let mut out: BTreeMap<String, FetchedPackage> = BTreeMap::new();
    for (base, reqs) in &by_base {
        let discovery = discover(base)?;
        let inline_chunks = discovery.capabilities.iter().any(|c| c == RESOLVE_INLINE_CHUNKS);
        let mut response = resolve_requirements(base, reqs, have, inline_chunks)?;
        if !inline_chunks {
            for level in &mut response.levels {
                for node in level {
                    node.entries = None;
                }
            }
        }
        let api_base = api_base_for(&response.base, base);
        let mut nodes: BTreeMap<(String, String), &ResolveNode> = BTreeMap::new();
        for level in &response.levels {
            for node in level {
                nodes.insert((node.full.clone(), node.version.clone()), node);
            }
        }
        let ordered: Vec<&ResolveNode> = nodes.values().copied().collect();
        let slots: Vec<std::sync::Mutex<Option<Result<PathBuf, Diagnostic>>>> =
            (0..ordered.len()).map(|_| std::sync::Mutex::new(None)).collect();
        let next = std::sync::atomic::AtomicUsize::new(0);
        let width = PACKAGE_FETCH_CONCURRENCY.min(ordered.len().max(1));
        let share = FetchShare::new();
        std::thread::scope(|s| {
            for _ in 0..width {
                s.spawn(|| loop {
                    let i = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                    let Some(node) = ordered.get(i) else {
                        break;
                    };
                let got = ensure_node(cache_root, base, &api_base, node, &share);
                    let mut slot = slots[i].lock().unwrap_or_else(|e| e.into_inner());
                    *slot = Some(got);
                });
            }
        });
        for (i, node) in ordered.iter().enumerate() {
            let mut slot = slots[i].lock().unwrap_or_else(|e| e.into_inner());
            match slot.take() {
                Some(Ok(dir)) => {
                    out.insert(
                        node.full.clone(),
                        FetchedPackage { name: node.full.clone(), version: node.version.clone(), dir },
                    );
                }
                Some(Err(e)) => return Err(e),
                None => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("package `{}@{}` was not fetched", node.full, node.version),
                    ));
                }
            }
        }
        for (full, version) in &response.resolved {
            if !out.contains_key(full) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("registry resolved `{full}@{version}` but sent no download for it"),
                ));
            }
        }
    }
    Ok(out.into_values().collect())
}

fn pinned_cache_hit(dir: &Path, _pkg: &str, pin: &str) -> Option<(PathBuf, ProjectConfig, String)> {
    if !(dir.join(crate::project::MANIFEST_FILE).is_file() && cached_integrity(dir).is_some()) {
        return None;
    }
    let canonical = std::fs::canonicalize(dir).ok()?;
    let cfg = ProjectConfig::load_from_dir(&canonical).ok()??;
    if cfg.artifacts.is_none() || cached_artifacts_present(&canonical) {
        return Some((canonical, cfg, pin.to_string()));
    }
    None
}

pub fn resolve_registry_package(
    pkg: &str,
    range: &str,
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
    pinned: Option<String>,
    have: &[HaveEntry],
    anchor: Option<&Path>,
) -> Result<(PathBuf, ProjectConfig, String), Diagnostic> {
    if let Some(root) = anchor {
        if let Some(found) = resolve_local_override(pkg, range, pinned.as_deref(), root)? {
            return Ok(found);
        }
    }
    if let Some(pin) = pinned.as_deref() {
        // Pinned versions skip the network when a verified copy is cached:
        // the integrity file is written only after a verified download, so
        // its presence means completeness. The project-local registry (from
        // `rnx download --local`) wins over the shared global cache. This
        // keeps locked builds (and repeat installs) entirely off the wire.
        // The cached copy still enforces the compat checks: the engine range
        // comes from the cached manifest and the yank flag from the marker
        // the download wrote, so a stale toolchain or a yanked pin fails or
        // warns here exactly as on the fresh-download path.
        let base = registry_base_for(pkg, default_registry, overrides);
        if let Some(root) = anchor {
            let dir = project_package_dir(root, &base, pkg, pin);
            if let Some(found) = pinned_cache_hit(&dir, pkg, pin) {
                check_cached_engine(&found.1, pkg, pin)?;
                warn_if_cached_yanked(&found.0, pkg, pin);
                return Ok(found);
            }
        }
        if let Some(dir) = find_store_pkg(&base, pkg, pin)
            && let Some(found) = pinned_cache_hit(&dir, pkg, pin)
        {
            check_cached_engine(&found.1, pkg, pin)?;
            warn_if_cached_yanked(&found.0, pkg, pin);
            return Ok(found);
        }
    }
    let base = registry_base_for(pkg, default_registry, overrides);
    let requirement = pinned.unwrap_or_else(|| range.to_string());
    let mut reqs = BTreeMap::new();
    reqs.insert(pkg.to_string(), requirement);
    let fetched = ensure_registry_requirements(&reqs, default_registry, overrides, have)
        .map_err(|e| with_offline_package_context(e, pkg, &base))?;
    let found = fetched.iter().find(|f| f.name == pkg).ok_or_else(|| {
        Diagnostic::new(Code::E108, format!("registry resolved no version for package `{pkg}`"))
    })?;
    let cfg = ProjectConfig::load_from_dir(&found.dir)?.ok_or_else(|| {
        Diagnostic::new(Code::E108, format!("package `{pkg}` has no Project.config"))
    })?;
    Ok((found.dir.clone(), cfg, found.version.clone()))
}

pub fn fetch_all_registry_deps(
    scope_root: &Path,
) -> Result<Vec<(String, String)>, Diagnostic> {
    let manifest = project::load_manifest(scope_root)?.ok_or_else(|| {
        Diagnostic::new(Code::E108, "No file specified and no Project.config found")
    })?;
    let mut cfgs: Vec<(String, ProjectConfig)> = Vec::new();
    match manifest.workspace {
        Some(ws) => {
            let members = project::resolve_workspace_members(scope_root, &ws)?;
            for (name, (_, cfg)) in members {
                cfgs.push((name, cfg));
            }
        }
        None => {
            let cfg = manifest.project.ok_or_else(|| {
                Diagnostic::new(Code::E108, "No file specified and no Project.config found")
            })?;
            cfgs.push((cfg.name.clone(), cfg));
        }
    }
    cfgs.sort_by(|a, b| a.0.cmp(&b.0));
    let mut have = scan_cache_have();
    let mut out: BTreeMap<String, String> = BTreeMap::new();
    for (_, cfg) in &cfgs {
        let mut reqs: BTreeMap<String, String> = BTreeMap::new();
        for (dep_name, spec) in &cfg.dependencies {
            if let crate::project::DependencySpec::Semver { version, .. } = spec {
                reqs.insert(dep_name.clone(), version.clone());
            }
        }
        if reqs.is_empty() {
            continue;
        }
        let fetched =
            ensure_registry_requirements(&reqs, cfg.registry.as_ref(), &cfg.registries, &have)?;
        for f in fetched {
            out.insert(f.name, f.version);
        }
        have = scan_cache_have();
    }
    Ok(out.into_iter().collect())
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex, OnceLock};

    pub struct LoggedRequest {
        pub method: String,
        pub path: String,
        pub headers: Vec<(String, String)>,
        pub body: Vec<u8>,
    }

    pub fn version_body(spec: u32, capabilities: &[&str]) -> String {
        let list = capabilities
            .iter()
            .map(|c| format!("\"{c}\""))
            .collect::<Vec<_>>()
            .join(", ");
        format!("{{\"spec\": {spec}, \"capabilities\": [{list}], \"registry\": \"test\"}}")
    }

    pub struct MockConfig {
        pub version_spec: u32,
        pub capabilities: Vec<&'static str>,
        pub resolve_status: u16,
        pub resolve_body: String,
        pub manifest_status: u16,
        pub fallback_tarball: Vec<u8>,
        pub manifest_error: String,
        pub manifest_override: Option<String>,
        pub chunk_override: Option<Vec<(String, Vec<u8>)>>,
    }

    pub struct MockRegistry {
        pub base: String,
        pub log: Arc<Mutex<Vec<LoggedRequest>>>,
    }

    static ENV_LOCK: OnceLock<Mutex<()>> = OnceLock::new();

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    pub struct EnvGuard {
        lock: Option<std::sync::MutexGuard<'static, ()>>,
        prev_cache: Option<String>,
        prev_vendor: Option<String>,
        prev_proxies: Vec<(String, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prev_cache.clone() {
                    Some(v) => std::env::set_var("RNX_CACHE_HOME", v),
                    None => std::env::remove_var("RNX_CACHE_HOME"),
                }
                match self.prev_vendor.clone() {
                    Some(v) => std::env::set_var(crate::fetch::LOCAL_VENDOR_ENV, v),
                    None => std::env::remove_var(crate::fetch::LOCAL_VENDOR_ENV),
                }
            }
            for (key, prev) in self.prev_proxies.drain(..) {
                unsafe {
                    match prev {
                        Some(v) => std::env::set_var(&key, v),
                        None => std::env::remove_var(&key),
                    }
                }
            }
            self.lock.take();
        }
    }

    pub fn isolate_cache(tag: &str) -> (EnvGuard, crate::scratch::Scratch) {
        let lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        crate::modules::clear_std_overlays();
        let dir = crate::scratch::dir(&format!("reg-{tag}"));
        let prev_cache = std::env::var("RNX_CACHE_HOME").ok();
        let prev_vendor = std::env::var(crate::fetch::LOCAL_VENDOR_ENV).ok();
        unsafe {
            std::env::set_var("RNX_CACHE_HOME", dir.path());
            std::env::remove_var(crate::fetch::LOCAL_VENDOR_ENV);
        }
        let mut prev_proxies = Vec::new();
        for key in [
            "HTTP_PROXY",
            "HTTPS_PROXY",
            "ALL_PROXY",
            "http_proxy",
            "https_proxy",
            "all_proxy",
        ] {
            prev_proxies.push((key.to_string(), std::env::var(key).ok()));
            unsafe {
                std::env::remove_var(key);
            }
        }
        (
            EnvGuard { lock: Some(lock), prev_cache, prev_vendor, prev_proxies },
            dir,
        )
    }

    pub struct VendorDirGuard {
        prev: Option<String>,
    }

    impl Drop for VendorDirGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prev.clone() {
                    Some(v) => std::env::set_var(crate::fetch::LOCAL_VENDOR_ENV, v),
                    None => std::env::remove_var(crate::fetch::LOCAL_VENDOR_ENV),
                }
            }
        }
    }

    pub fn set_vendor_dir(dir: &std::path::Path) -> VendorDirGuard {
        let prev = std::env::var(crate::fetch::LOCAL_VENDOR_ENV).ok();
        unsafe {
            std::env::set_var(crate::fetch::LOCAL_VENDOR_ENV, dir);
        }
        VendorDirGuard { prev }
    }

    fn status_text(status: u16) -> &'static str {
        match status {
            200 => "OK",
            400 => "Bad Request",
            404 => "Not Found",
            410 => "Gone",
            422 => "Unprocessable Entity",
            _ => "Internal Server Error",
        }
    }

    fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String, Vec<(String, String)>, Vec<u8>)> {
        let mut head = Vec::new();
        let mut byte = [0u8; 1];
        while !head.ends_with(b"\r\n\r\n") {
            match stream.read(&mut byte) {
                Ok(0) => return None,
                Ok(_) => head.push(byte[0]),
                Err(_) => return None,
            }
            if head.len() > 65536 {
                return None;
            }
        }
        let text = String::from_utf8_lossy(&head).into_owned();
        let mut lines = text.lines();
        let request_line = lines.next().unwrap_or_default().to_string();
        let mut parts = request_line.split_whitespace();
        let method = parts.next().unwrap_or_default().to_string();
        let path = parts.next().unwrap_or_default().to_string();
        let mut length = 0usize;
        let mut headers = Vec::new();
        for line in lines {
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                length = rest.trim().parse::<usize>().unwrap_or(0);
            }
            if let Some(rest) = line.strip_prefix("content-length:") {
                length = rest.trim().parse::<usize>().unwrap_or(0);
            }
            if let Some(i) = line.find(':') {
                headers.push((line[..i].trim().to_string(), line[i + 1..].trim().to_string()));
            }
        }
        let mut body = vec![0u8; length.min(16 * 1024 * 1024)];
        let mut read = 0usize;
        while read < body.len() {
            match stream.read(&mut body[read..]) {
                Ok(0) => break,
                Ok(n) => read += n,
                Err(_) => break,
            }
        }
        body.truncate(read);
        Some((method, path, headers, body))
    }

    fn respond(
        stream: &mut std::net::TcpStream,
        status: u16,
        content_type: &str,
        body: &[u8],
        extra: &[(&str, &str)],
    ) {
        let mut head = format!(
            "HTTP/1.1 {status} {}\r\nContent-Type: {content_type}\r\nContent-Length: {}\r\nConnection: close\r\n",
            status_text(status),
            body.len()
        );
        for (k, v) in extra {
            head.push_str(&format!("{k}: {v}\r\n"));
        }
        head.push_str("\r\n");
        let _ = stream.write_all(head.as_bytes());
        let _ = stream.write_all(body);
        let _ = stream.flush();
    }

    impl MockRegistry {
        pub fn start(config: MockConfig) -> MockRegistry {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            let log: Arc<Mutex<Vec<LoggedRequest>>> = Arc::new(Mutex::new(Vec::new()));
            let config = Arc::new(config);
            let seen = log.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let Some((method, path, headers, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    // Origin-direct reads carry `?origin-direct=1`; match on
                    // the path without query so salted and plain reads behave
                    // identically here.
                    let route = path.split('?').next().unwrap_or_default().to_string();
                    seen.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(LoggedRequest { method: method.clone(), path: path.clone(), headers, body });
                    let spec = config.version_spec.to_string();
                    let with_spec: &[(&str, &str)] = &[("rnx-registry-spec", Box::leak(spec.into_boxed_str()) as &str)];
                    if method == "GET" && route == "/api/version" {
                        let body = version_body(config.version_spec, &config.capabilities);
                        respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                    } else if method == "POST" && route == "/api/v0/resolve" {
                        respond(
                            &mut stream,
                            config.resolve_status,
                            "application/json",
                            config.resolve_body.as_bytes(),
                            with_spec,
                        );
                    } else if method == "GET" && route.ends_with("/chunks") {
                        if config.manifest_status == 200 {
                            let body = match &config.manifest_override {
                                Some(m) => m.clone(),
                                None => fallback_manifest(&config.fallback_tarball),
                            };
                            respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                        } else {
                            respond(
                                &mut stream,
                                config.manifest_status,
                                "application/json",
                                config.manifest_error.as_bytes(),
                                with_spec,
                            );
                        }
                    } else if method == "GET" && route.contains("/chunk/") {
                        let hash = route.rsplit('/').next().unwrap_or_default().to_string();
                        let mut hit: Option<Vec<u8>> = None;
                        if let Some(pairs) = &config.chunk_override {
                            for (h, b) in pairs {
                                if *h == hash {
                                    hit = Some(b.clone());
                                    break;
                                }
                            }
                        }
                        if hit.is_none() && !config.fallback_tarball.is_empty() {
                            let digest = Sha256::hexdigest(&config.fallback_tarball);
                            if digest == hash {
                                hit = Some(config.fallback_tarball.clone());
                            }
                        }
                        match hit {
                            Some(bytes) => respond(&mut stream, 200, "application/octet-stream", &bytes, with_spec),
                            None => respond(
                                &mut stream,
                                404,
                                "application/json",
                                b"{\"code\": \"not-found\", \"message\": \"chunk not found\"}",
                                with_spec,
                            ),
                        }
                    } else {
                        respond(
                            &mut stream,
                            404,
                            "application/json",
                            b"{\"code\": \"not-found\", \"message\": \"unknown\"}",
                            with_spec,
                        );
                    }
                }
            });
            MockRegistry { base, log }
        }

        pub fn requests(&self) -> Vec<(String, String, Vec<u8>)> {
            self.log
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|r| (r.method.clone(), r.path.clone(), r.body.clone()))
                .collect()
        }

        pub fn count_get(&self, suffix: &str) -> usize {
            self.requests()
                .iter()
                .filter(|(m, p, _)| m == "GET" && p.split('?').next().unwrap_or_default().ends_with(suffix))
                .count()
        }

        pub fn requests_with_headers(
            &self,
        ) -> Vec<(String, String, Vec<(String, String)>, Vec<u8>)> {
            self.log
                .lock()
                .unwrap_or_else(|e| e.into_inner())
                .iter()
                .map(|r| (r.method.clone(), r.path.clone(), r.headers.clone(), r.body.clone()))
                .collect()
        }

        pub fn start_encoded(
            config: MockConfig,
            manifest_body: Vec<u8>,
            manifest_encoding: Option<&'static str>,
        ) -> MockRegistry {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            let log: Arc<Mutex<Vec<LoggedRequest>>> = Arc::new(Mutex::new(Vec::new()));
            let config = Arc::new(config);
            let manifest_body = Arc::new(manifest_body);
            let seen = log.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let Some((method, path, headers, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    let route = path.split('?').next().unwrap_or_default().to_string();
                    seen.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(LoggedRequest { method: method.clone(), path: path.clone(), headers, body });
                    let spec = config.version_spec.to_string();
                    let with_spec: &[(&str, &str)] = &[("rnx-registry-spec", Box::leak(spec.into_boxed_str()) as &str)];
                    if method == "GET" && route == "/api/version" {
                        let body = version_body(config.version_spec, &config.capabilities);
                        respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                    } else if method == "POST" && route == "/api/v0/resolve" {
                        respond(
                            &mut stream,
                            200,
                            "application/json",
                            config.resolve_body.as_bytes(),
                            with_spec,
                        );
                    } else if method == "GET" && route.ends_with("/chunks") {
                        if config.manifest_status == 200 {
                            match manifest_encoding {
                                Some(enc) => respond(
                                    &mut stream,
                                    200,
                                    "application/json",
                                    &manifest_body,
                                    &[
                                        ("rnx-registry-spec", with_spec[0].1),
                                        ("Content-Encoding", enc),
                                    ],
                                ),
                                None => respond(
                                    &mut stream,
                                    200,
                                    "application/json",
                                    &manifest_body,
                                    with_spec,
                                ),
                            }
                        } else {
                            respond(
                                &mut stream,
                                config.manifest_status,
                                "application/json",
                                config.manifest_error.as_bytes(),
                                with_spec,
                            );
                        }
                    } else if method == "GET" && route.contains("/chunk/") {
                        let hash = route.rsplit('/').next().unwrap_or_default().to_string();
                        let mut hit: Option<Vec<u8>> = None;
                        if let Some(pairs) = &config.chunk_override {
                            for (h, b) in pairs {
                                if *h == hash {
                                    hit = Some(b.clone());
                                    break;
                                }
                            }
                        }
                        if hit.is_none() && !config.fallback_tarball.is_empty() {
                            let digest = Sha256::hexdigest(&config.fallback_tarball);
                            if digest == hash {
                                hit = Some(config.fallback_tarball.clone());
                            }
                        }
                        match hit {
                            Some(bytes) => respond(&mut stream, 200, "application/octet-stream", &bytes, with_spec),
                            None => respond(
                                &mut stream,
                                404,
                                "application/json",
                                b"{\"code\": \"not-found\", \"message\": \"chunk not found\"}",
                                with_spec,
                            ),
                        }
                    } else {
                        respond(
                            &mut stream,
                            404,
                            "application/json",
                            b"{\"code\": \"not-found\", \"message\": \"unknown\"}",
                            with_spec,
                        );
                    }
                }
            });
            MockRegistry { base, log }
        }

        pub fn resolve_bodies(&self) -> Vec<JsonValue> {
            self.requests()
                .iter()
                .filter(|(m, p, _)| m == "POST" && p == "/api/v0/resolve")
                .filter_map(|(_, _, b)| parse_json(b).ok())
                .collect()
        }

        pub fn start_dynamic(
            chunks: BTreeMap<String, (String, Vec<(String, Vec<u8>)>)>,
            resolve: impl Fn(BTreeMap<String, String>) -> String + Send + Sync + 'static,
        ) -> MockRegistry {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            let log: Arc<Mutex<Vec<LoggedRequest>>> = Arc::new(Mutex::new(Vec::new()));
            let chunks = Arc::new(chunks);
            let resolve = Arc::new(resolve);
            let seen = log.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(mut stream) = stream else {
                        continue;
                    };
                    let Some((method, path, headers, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    let route = path.split('?').next().unwrap_or_default().to_string();
                    seen.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(LoggedRequest { method: method.clone(), path: path.clone(), headers, body: body.clone() });
                    let with_spec: &[(&str, &str)] = &[("rnx-registry-spec", "0")];
                    if method == "GET" && route == "/api/version" {
                        let body = version_body(0, &["tombstones"]);
                        respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                    } else if method == "POST" && route == "/api/v0/resolve" {
                        let mut reqs = BTreeMap::new();
                        if let Ok(value) = parse_json(&body) {
                            if let JsonValue::Object(fields) =
                                value.get("requirements").cloned().unwrap_or(JsonValue::Null)
                            {
                                for (k, v) in fields {
                                    if let Some(s) = v.as_str() {
                                        reqs.insert(k, s.to_string());
                                    }
                                }
                            }
                        }
                        let out = resolve(reqs);
                        respond(&mut stream, 200, "application/json", out.as_bytes(), with_spec);
                    } else if method == "GET" && route.ends_with("/chunks") {
                        let hit = chunks.iter().find(|(k, _)| route.contains(k.as_str()));
                        match hit {
                            Some((_, (manifest, _))) => respond(
                                &mut stream,
                                200,
                                "application/json",
                                manifest.as_bytes(),
                                with_spec,
                            ),
                            None => respond(
                                &mut stream,
                                404,
                                "application/json",
                                b"{\"code\": \"not-found\", \"message\": \"unknown\"}",
                                with_spec,
                            ),
                        }
                    } else if method == "GET" && route.contains("/chunk/") {
                        let hash = route.rsplit('/').next().unwrap_or_default().to_string();
                        let mut hit: Option<Vec<u8>> = None;
                        for (_, pairs) in chunks.values() {
                            for (h, b) in pairs {
                                if *h == hash {
                                    hit = Some(b.clone());
                                    break;
                                }
                            }
                        }
                        match hit {
                            Some(bytes) => respond(&mut stream, 200, "application/octet-stream", &bytes, with_spec),
                            None => respond(
                                &mut stream,
                                404,
                                "application/json",
                                b"{\"code\": \"not-found\", \"message\": \"chunk not found\"}",
                                with_spec,
                            ),
                        }
                    } else {
                        respond(
                            &mut stream,
                            404,
                            "application/json",
                            b"{\"code\": \"not-found\", \"message\": \"unknown\"}",
                            with_spec,
                        );
                    }
                }
            });
            MockRegistry { base, log }
        }

        pub fn start_routed(config: RoutedConfig) -> MockRegistry {
            let listener = TcpListener::bind("127.0.0.1:0").unwrap();
            let addr = listener.local_addr().unwrap();
            let base = format!("http://{addr}");
            let log: Arc<Mutex<Vec<LoggedRequest>>> = Arc::new(Mutex::new(Vec::new()));
            let spec: &'static str =
                Box::leak(config.version_spec.to_string().into_boxed_str());
            let config = Arc::new(config);
            let seen = log.clone();
            std::thread::spawn(move || {
                for stream in listener.incoming() {
                    let Ok(stream) = stream else {
                        continue;
                    };
                    let config = config.clone();
                    let seen = seen.clone();
                    std::thread::spawn(move || {
                        let mut stream = stream;
                        let Some((method, path, headers, body)) = read_request(&mut stream) else {
                            return;
                        };
                        let route = path.split('?').next().unwrap_or_default().to_string();
                        seen.lock()
                            .unwrap_or_else(|e| e.into_inner())
                            .push(LoggedRequest { method: method.clone(), path: path.clone(), headers, body });
                        let with_spec: &[(&str, &str)] = &[("rnx-registry-spec", spec)];
                        if method == "GET" && route == "/api/version" {
                            let body = format!(
                                "{{\"spec\": {}, \"capabilities\": [\"tombstones\"], \"registry\": \"test\"}}",
                                config.version_spec
                            );
                            respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                        } else if method == "POST" && route == "/api/v0/resolve" {
                            respond(
                                &mut stream,
                                200,
                                "application/json",
                                config.resolve_body.as_bytes(),
                                with_spec,
                            );
                        } else if method == "GET" && route.ends_with("/chunks") {
                            let pkg = config
                                .packages
                                .iter()
                                .find(|p| route.contains(&format!("{}@{}", p.full, p.version)));
                            match pkg {
                                Some(p) => {
                                    sleep_ms(p.latency_ms);
                                    let body = match &p.manifest_override {
                                        Some(m) => m.clone(),
                                        None => fallback_manifest(&p.tarball),
                                    };
                                    respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                                }
                                None => respond(
                                    &mut stream,
                                    404,
                                    "application/json",
                                    b"{\"code\": \"not-found\", \"message\": \"unknown\"}",
                                    with_spec,
                                ),
                            }
                        } else if method == "GET" && route.contains("/chunk/") {
                            let hash = route.rsplit('/').next().unwrap_or_default().to_string();
                            let mut hit: Option<(Vec<u8>, u64)> = None;
                            for p in &config.packages {
                                if let Some(pairs) = &p.chunk_override {
                                    for (h, b) in pairs {
                                        if *h == hash {
                                            hit = Some((b.clone(), p.latency_ms));
                                            break;
                                        }
                                    }
                                }
                                if hit.is_some() {
                                    break;
                                }
                                if !p.tarball.is_empty() && Sha256::hexdigest(&p.tarball) == hash {
                                    hit = Some((p.tarball.clone(), p.latency_ms));
                                    break;
                                }
                            }
                            match hit {
                                Some((bytes, latency)) => {
                                    sleep_ms(latency);
                                    respond(&mut stream, 200, "application/octet-stream", &bytes, with_spec);
                                }
                                None => respond(
                                    &mut stream,
                                    404,
                                    "application/json",
                                    b"{\"code\": \"not-found\", \"message\": \"chunk not found\"}",
                                    with_spec,
                                ),
                            }
                        } else {
                            respond(
                                &mut stream,
                                404,
                                "application/json",
                                b"{\"code\": \"not-found\", \"message\": \"unknown\"}",
                                with_spec,
                            );
                        }
                    });
                }
            });
            MockRegistry { base, log }
        }
    }

    pub struct RoutedPackage {
        pub full: String,
        pub version: String,
        pub tarball: Vec<u8>,
        pub manifest_override: Option<String>,
        pub chunk_override: Option<Vec<(String, Vec<u8>)>>,
        pub latency_ms: u64,
    }

    pub struct RoutedConfig {
        pub version_spec: u32,
        pub resolve_body: String,
        pub packages: Vec<RoutedPackage>,
    }

    fn sleep_ms(ms: u64) {
        if ms > 0 {
            std::thread::sleep(std::time::Duration::from_millis(ms));
        }
    }

    pub fn routed_fixture(full: &str, version: &str, latency_ms: u64) -> (RoutedPackage, String) {
        let (manifest, pairs, digest, _, _) = fixture_tree(full, version);
        (
            RoutedPackage {
                full: full.to_string(),
                version: version.to_string(),
                tarball: Vec::new(),
                manifest_override: Some(manifest),
                chunk_override: Some(pairs),
                latency_ms,
            },
            digest,
        )
    }

    pub fn fallback_manifest(tarball: &[u8]) -> String {
        let digest = Sha256::hexdigest(tarball);
        format!(
            "{{\"entries\": [{{\"name\": \"\", \"size\": {}, \"dir\": false, \"chunks\": [\"{digest}\"]}}]}}",
            tarball.len()
        )
    }

    pub fn fixture_tarball(full: &str, version: &str) -> (Vec<u8>, String) {        let manifest = format!(
            "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\"\n    }}\n}}\n"
        );
        let mut buf = Vec::new();
        {
            let mut tar = crate::tar::TarWriter::new(&mut buf);
            tar.add_file("Project.config", manifest.as_bytes()).unwrap();
            tar.add_file("src/main.rnx", b"export fn hello(): Int { return 1; }\n")
                .unwrap();
            tar.finish().unwrap();
        }
        let gz = crate::gzip::compress_gzip(&buf);
        let sha = Sha256::hexdigest(&gz);
        (gz, sha)
    }

    pub fn node_json(full: &str, version: &str, integrity: &str, yanked: bool) -> String {
        node_engine_json(full, version, integrity, yanked, "")
    }

    pub fn node_artifact_json(full: &str, version: &str, integrity: &str, artifact_digest: &str) -> String {
        format!(
            "{{\"full\": \"{full}\", \"version\": \"{version}\", \"path\": \"{full}@{version}/chunks\", \"integrity\": \"{integrity}\", \"engineRange\": \"\", \"deps\": [], \"yanked\": false, \"artifactDigest\": \"{artifact_digest}\"}}"
        )
    }

    // A publish-shaped file tree: the stored Project.config carries the
    // published version placeholder, exactly as the registry stores it.
    // Installs rehydrate it back to the resolved version. Returns the
    // served manifest, the chunk pairs, the manifest digest, and the
    // installed config and main bytes.
    pub fn fixture_tree(
        full: &str,
        version: &str,
    ) -> (String, Vec<(String, Vec<u8>)>, String, Vec<u8>, Vec<u8>) {
        fixture_tree_with_main(full, version, b"export fn hello(): Int { return 1; }\n")
    }

    pub fn fixture_tree_with_main(
        full: &str,
        version: &str,
        main: &[u8],
    ) -> (String, Vec<(String, Vec<u8>)>, String, Vec<u8>, Vec<u8>) {
        let original = format!(
            "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\"\n    }}\n}}\n"
        );
        let stored = original.replace(
            &format!("version: \"{version}\""),
            "version: \"0.0.0-published\"",
        );
        let main = main.to_vec();
        let files = vec![
            ("Project.config".to_string(), stored.into_bytes()),
            ("src/main.rnx".to_string(), main.clone()),
        ];
        let mut entries = Vec::new();
        let mut pairs = Vec::new();
        for (name, bytes) in &files {
            let h = Sha256::hexdigest(bytes);
            entries.push(format!(
                "{{\"name\": \"{name}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{h}\"]}}",
                bytes.len()
            ));
            pairs.push((h, bytes.clone()));
        }
        let manifest = format!("{{\"entries\": [{}]}}", entries.join(", "));
        let digest = manifest_digest_of(&manifest);
        let installed = original.into_bytes();
        (manifest, pairs, digest, installed, main)
    }

    pub fn manifest_digest_of(manifest_json: &str) -> String {
        let value = parse_json(manifest_json.as_bytes()).unwrap();
        let items = value.get("entries").and_then(|v| v.as_array()).unwrap();
        let entries: Vec<crate::checksum::DigestEntry> = items
            .iter()
            .map(|item| crate::checksum::DigestEntry {
                name: item.get("name").and_then(|v| v.as_str()).unwrap_or_default().to_string(),
                size: item.get("size").and_then(|v| v.as_u64()).unwrap_or(0),
                dir: item.get("dir").and_then(|v| v.as_bool()).unwrap_or(false),
                chunks: item
                    .get("chunks")
                    .and_then(|v| v.as_array())
                    .map(|a| {
                        a.iter().map(|h| h.as_str().unwrap_or_default().to_string()).collect()
                    })
                    .unwrap_or_default(),
            })
            .collect();
        crate::checksum::manifest_digest_hex(&entries).unwrap()
    }

    // A resolve node carrying the chunk list the server would otherwise make
    // the client fetch from `{path}`. `manifest_json` is the `{entries:[...]}`
    // document `/chunks` serves; the resolve node carries the bare array.
    pub fn node_json_inline(
        full: &str,
        version: &str,
        integrity: &str,
        manifest_json: &str,
    ) -> String {
        let entries = inline_entries_json(manifest_json);
        format!(
            "{{\"full\": \"{full}\", \"version\": \"{version}\", \"path\": \"{full}@{version}/chunks\", \"integrity\": \"{integrity}\", \"engineRange\": \"\", \"deps\": [], \"yanked\": false, \"entries\": {entries}}}"
        )
    }

    // The testkit has no JSON writer, so hand-render the entry objects.
    fn inline_entries_json(manifest_json: &str) -> String {
        let value = parse_json(manifest_json.as_bytes()).unwrap();
        let Some(items) = value.get("entries").and_then(|v| v.as_array()) else {
            return "null".to_string();
        };
        let rendered: Vec<String> = items
            .iter()
            .map(|item| {
                let name = item.get("name").and_then(|v| v.as_str()).unwrap_or_default();
                let size = item.get("size").and_then(|v| v.as_u64()).unwrap_or(0);
                let dir = item.get("dir").and_then(|v| v.as_bool()).unwrap_or(false);
                let chunks = match item.get("chunks").and_then(|v| v.as_array()) {
                    Some(list) => list
                        .iter()
                        .map(|h| format!("\"{}\"", h.as_str().unwrap_or_default()))
                        .collect::<Vec<_>>()
                        .join(","),
                    None => String::new(),
                };
                format!(
                    "{{\"name\":\"{}\",\"size\":{size},\"dir\":{dir},\"chunks\":[{chunks}]}}",
                    json_escape(name)
                )
            })
            .collect();
        format!("[{}]", rendered.join(","))
    }

    pub fn node_engine_json(
        full: &str,
        version: &str,
        integrity: &str,
        yanked: bool,
        engine_range: &str,
    ) -> String {
        format!(
            "{{\"full\": \"{full}\", \"version\": \"{version}\", \"path\": \"{full}@{version}/chunks\", \"integrity\": \"{integrity}\", \"engineRange\": \"{engine_range}\", \"deps\": [], \"yanked\": {yanked}}}"
        )
    }

    pub fn resolve_json_static(nodes: &[String]) -> String {
        resolve_json_chunks(nodes, None)
    }

    pub fn resolve_json_chunks(nodes: &[String], chunks: Option<&str>) -> String {
        let mut resolved = String::new();
        for node in nodes {
            let value = parse_json(node.as_bytes()).unwrap();
            let full = value.get("full").and_then(|v| v.as_str()).unwrap_or_default();
            let version = value.get("version").and_then(|v| v.as_str()).unwrap_or_default();
            if !resolved.is_empty() {
                resolved.push_str(", ");
            }
            resolved.push_str(&format!("\"{full}\": \"{version}\""));
        }
        let switch = chunks.map(|c| format!(", \"chunks\": \"{c}\"")).unwrap_or_default();
        format!(
            "{{\"resolved\": {{{resolved}}}, \"base\": \"\", \"levels\": [[{}]]{switch}}}",
            nodes.join(", ")
        )
    }

    pub fn requirement(body: &JsonValue, full: &str) -> String {
        body.get("requirements")
            .and_then(|r| r.get(full))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    }

    pub fn registry_cfg(url: &str) -> RegistryConfig {
        RegistryConfig { url: url.to_string(), token_env: None, ca_cert: None }
    }

    pub fn plant_store_package(
        full: &str,
        version: &str,
        integrity: &str,
        engine: &str,
        main: &str,
    ) -> PathBuf {
        let dir = super::store_pkg_dir(integrity).expect("planted package needs a hex digest");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join(crate::project::MANIFEST_FILE),
            format!(
                "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\",\n        engine: \"{engine}\"\n    }},\n    entries: {{\n        main: \"src/main.rnx\"\n    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), main).unwrap();
        std::fs::write(dir.join(".rnx-integrity"), format!("{full}\n{version}\n{integrity}\n"))
            .unwrap();
        dir
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::*;
    use super::*;

    #[test]
    fn short_rev_sanitizes() {
        assert_eq!(short_rev("v0.1.0"), "v0.1.0");
        assert_eq!(short_rev("abc123def456789"), "abc123def456");
        assert_eq!(short_rev("///"), "rev");
    }

    #[test]
    fn base_expansion_rules() {
        assert_eq!(
            expand_registry_base("rnx.mycompany.internal"),
            "https://rnx.mycompany.internal"
        );
        assert_eq!(
            expand_registry_base("rnx.mycompany.internal:8080"),
            "https://rnx.mycompany.internal:8080"
        );
        assert_eq!(
            expand_registry_base("https://example.com/custom/"),
            "https://example.com/custom"
        );
        assert_eq!(
            expand_registry_base("http://127.0.0.1:9"),
            "http://127.0.0.1:9"
        );
        assert_eq!(
            expand_registry_base(""),
            format!("https://{DEFAULT_REGISTRY_DOMAIN}")
        );
    }

    #[test]
    fn base_prefers_scope_override_then_default() {
        let mut overrides = BTreeMap::new();
        overrides.insert("@acme".to_string(), registry_cfg("acme.internal"));
        let default = registry_cfg("default.internal");
        assert_eq!(
            registry_base_for("@acme/widget", Some(&default), &overrides),
            "https://acme.internal"
        );
        assert_eq!(
            registry_base_for("plain", Some(&default), &overrides),
            "https://default.internal"
        );
        assert_eq!(
            registry_base_for("@other/pkg", Some(&default), &overrides),
            "https://default.internal"
        );
        assert_eq!(
            registry_base_for("plain", None, &overrides),
            format!("https://{DEFAULT_REGISTRY_DOMAIN}")
        );
    }

    #[test]
    fn range_grammar_matches_server() {
        for good in ["*", "latest", "1.2.3", "^1.2.3", "~1.2.3", ">=1.2.3", "  ^0.4.0  "] {
            assert!(is_valid_range(good), "{good}");
        }
        for bad in ["", "banana", "^", "~", ">=", "1.2", "1.2.3.4", "=>1.2.3", "^1.2.3-beta!!"] {
            if bad.is_empty() {
                continue;
            }
            assert!(!is_valid_range(bad), "{bad}");
        }
    }

    #[test]
    fn json_roundtrip_shapes() {
        let value = parse_json(br#"{"spec": 1, "caps": ["a", "b"], "nested": {"x": true, "y": null}, "n": 1.5}"#).unwrap();
        assert_eq!(value.get("spec").and_then(|v| v.as_u64()), Some(1));
        let caps = value.get("caps").and_then(|v| v.as_array()).unwrap();
        assert_eq!(caps.len(), 2);
        assert_eq!(value.get("nested").and_then(|v| v.get("x")).and_then(|v| v.as_bool()), Some(true));
        assert_eq!(value.get("n"), Some(&JsonValue::Number(1.5)));
        assert_eq!(json_escape("a\"b\\c\nd"), "a\\\"b\\\\c\\nd");
    }

    #[test]
    fn json_rejects_malformed() {
        for bad in [
            "",
            "{",
            "{\"a\": }",
            "{\"a\": tru}",
            "[1, 2,]",
            "{\"a\": \"\\x\"}",
            "{\"a\": \"bad \u{0001} raw\"}",
            "[01]",
            "{\"a\": 1} trailing",
        ] {
            assert!(parse_json(bad.as_bytes()).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn invalid_range_rejected_before_network() {
        let (_guard, _dir) = isolate_cache("badrange");
        let mut reqs = BTreeMap::new();
        reqs.insert("widget".to_string(), "banana".to_string());
        let err = ensure_registry_requirements(&reqs, None, &BTreeMap::new(), &[]).unwrap_err();
        assert!(err.message.contains("not a valid range"), "{}", err.message);
    }

    fn start_ok(
        version_spec: u32,
        nodes: &[String],
        tarball: Vec<u8>,
        manifest_status: u16,
        manifest_error: &str,
    ) -> MockRegistry {
        MockRegistry::start(MockConfig {
            version_spec,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(nodes),
            manifest_status,
            fallback_tarball: tarball,
            manifest_error: manifest_error.to_string(),
            manifest_override: None,
            chunk_override: None,
        })
    }

    fn start_tree(nodes: &[String], manifest: &str, pairs: Vec<(String, Vec<u8>)>) -> MockRegistry {
        MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(nodes),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest.to_string()),
            chunk_override: Some(pairs),
        })
    }

    fn requirement(body: &JsonValue, full: &str) -> String {
        body.get("requirements")
            .and_then(|r| r.get(full))
            .and_then(|v| v.as_str())
            .unwrap_or_default()
            .to_string()
    }

    fn have_entry(body: &JsonValue, index: usize) -> (String, String, String) {
        let item = body
            .get("have")
            .and_then(|h| h.as_array())
            .and_then(|items| items.get(index));
        let field = |key: &str| {
            item.and_then(|v| v.get(key))
                .and_then(|v| v.as_str())
                .unwrap_or_default()
                .to_string()
        };
        (field("full"), field("version"), field("integrity"))
    }

    fn requirements_for(full: &str, range: &str) -> BTreeMap<String, String> {
        let mut reqs = BTreeMap::new();
        reqs.insert(full.to_string(), range.to_string());
        reqs
    }

    #[test]
    fn vendor_stdlib_falls_back_to_checkout_sources_offline() {
        let (_guard, _dir) = isolate_cache("vendorstd");
        let src = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src");
        assert!(src.join("prelude").join("lib.rnx").is_file(), "{}", src.display());
        let scope = scope_scratch("checkout");
        let dead = RegistryConfig {
            url: "http://127.0.0.1:9".to_string(),
            token_env: None,
            ca_cert: None,
        };
        let dest = scope.join("staged");
        let done = vendor_packages(
            &scope,
            &dest,
            &["@std/fs".to_string()],
            Some(&dead),
            &BTreeMap::new(),
        )
        .expect("checkout sources vendor with no network");
        let cli = env!("CARGO_PKG_VERSION");
        assert_eq!(done, vec![format!("@std/fs@{cli}")]);
        let staged = dest.join("@std/fs");
        let want = std::fs::read(src.join("fs").join("lib.rnx")).unwrap();
        assert_eq!(std::fs::read(staged.join("src").join("lib.rnx")).unwrap(), want);
        let cfg = crate::project::ProjectConfig::load_from_dir(&staged)
            .unwrap()
            .expect("staged stdlib package carries a manifest");
        assert_eq!(cfg.name, "@std/fs");
        assert_eq!(cfg.version, cli);
    }

    fn scope_scratch(tag: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("vendor-{tag}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"vendee\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/main.rnx\" }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int {\n    return 0;\n}\n").unwrap();
        dir
    }

    fn tree_server(full: &str, version: &str) -> (MockRegistry, RegistryConfig) {
        let (manifest, pairs, digest, _, _) = fixture_tree(full, version);
        let server = start_tree(&[node_json(full, version, &digest, false)], &manifest, pairs);
        let cfg = registry_cfg(&server.base);
        (server, cfg)
    }

    #[test]
    fn vendor_downloads_a_missing_registry_package_then_copies() {
        let (_guard, _dir) = isolate_cache("vendordl");
        let (_server, cfg) = tree_server("@acme/widget", "1.2.0");
        let scope = scope_scratch("missing");
        let dest = scope.join("staged");
        let done = vendor_packages(
            &scope,
            &dest,
            &["@acme/widget".to_string()],
            Some(&cfg),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(done, vec!["@acme/widget@1.2.0".to_string()]);
        let staged = dest.join("@acme/widget");
        let main = std::fs::read(staged.join("src").join("main.rnx")).unwrap();
        assert_eq!(main, b"export fn hello(): Int { return 1; }\n");
        let manifest = std::fs::read_to_string(staged.join("Project.config")).unwrap();
        assert!(manifest.contains("@acme/widget"), "{manifest}");
        assert!(manifest.contains("1.2.0"), "{manifest}");
    }

    #[test]
    fn vendor_honors_a_semver_selector() {
        let (_guard, _dir) = isolate_cache("vendorsem");
        let (_server, cfg) = tree_server("@acme/widget", "1.2.0");
        let scope = scope_scratch("semver");
        let dest = scope.join("staged");
        let done = vendor_packages(
            &scope,
            &dest,
            &["@acme/widget@^1.0.0".to_string()],
            Some(&cfg),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(done, vec!["@acme/widget@1.2.0".to_string()]);
        assert!(dest.join("@acme/widget").join("Project.config").is_file());
        let bad = vendor_packages(
            &scope,
            &dest,
            &["@acme/widget@bogus!!".to_string()],
            Some(&cfg),
            &BTreeMap::new(),
        )
        .unwrap_err();
        assert_eq!(bad.code, Code::E108);
    }

    #[test]
    fn vendor_prefers_the_local_cache_with_no_network() {
        let (_guard, _dir) = isolate_cache("vendorcache");
        let (server, cfg) = tree_server("@acme/widget", "1.2.0");
        let scope = scope_scratch("cached");
        let dest = scope.join("staged");
        vendor_packages(&scope, &dest, &["@acme/widget".to_string()], Some(&cfg), &BTreeMap::new())
            .unwrap();
        assert_eq!(server.resolve_bodies().len(), 1);
        let again = vendor_packages(
            &scope,
            &scope.join("staged-two"),
            &["@acme/widget".to_string()],
            Some(&cfg),
            &BTreeMap::new(),
        )
        .unwrap();
        assert_eq!(again, vec!["@acme/widget@1.2.0".to_string()]);
        assert!(scope.join("staged-two").join("@acme/widget").join("src").join("main.rnx").is_file());
        assert_eq!(
            server.resolve_bodies().len(),
            1,
            "the second vendor must serve from cache with no new resolve call"
        );
    }

    #[test]
    fn download_seeds_the_global_cache_by_default() {
        let (_guard, _dir) = isolate_cache("dlglobal");
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs.clone());
        let cfg = registry_cfg(&server.base);
        let fetched = download_packages(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&cfg),
            &BTreeMap::new(),
            None,
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].name, "@acme/widget");
        assert_eq!(fetched[0].version, "1.2.0");
        let _ = server;
        let dir = store_pkg_dir(&digest).expect("store dir for the manifest digest");
        assert!(dir.join("Project.config").is_file(), "{}", dir.display());
        assert_eq!(fetched[0].dir, std::fs::canonicalize(&dir).unwrap());
        assert!(
            !package_dir_under(&registry_cache_root(), &cfg.url, "@acme/widget", "1.2.0")
                .join("Project.config")
                .is_file(),
            "new downloads land in store/, not in the old registry/ tree"
        );
        assert_eq!(cached_integrity(&dir).as_deref(), Some(digest.as_str()));
        for (hash, _) in &pairs {
            assert!(cas_blob_path(hash).expect("cas path").is_file(), "{hash}");
        }
    }

    #[test]
    fn download_local_targets_the_project_cache() {
        let (_guard, _dir) = isolate_cache("dllocal");
        let (server, cfg) = tree_server("@acme/widget", "1.2.0");
        let scope = scope_scratch("local");
        let fetched = download_packages(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&cfg),
            &BTreeMap::new(),
            Some(&scope),
        )
        .unwrap();
        assert_eq!(fetched[0].version, "1.2.0");
        assert_eq!(server.resolve_bodies().len(), 1);
        let local = project_package_dir(&scope, &cfg.url, "@acme/widget", "1.2.0");
        assert!(local.join("Project.config").is_file(), "{}", local.display());
        assert!(
            !package_dir_under(&registry_cache_root(), &cfg.url, "@acme/widget", "1.2.0")
                .join("Project.config")
                .is_file(),
            "local download must not write the global cache"
        );
        let (dir, _, version) = resolve_registry_package(
            "@acme/widget",
            "1.2.0",
            Some(&cfg),
            &BTreeMap::new(),
            Some("1.2.0".to_string()),
            &[],
            Some(&scope),
        )
        .expect("project cache serves the pinned resolve");
        assert_eq!(version, "1.2.0");
        assert_eq!(dir, std::fs::canonicalize(&local).unwrap());
        assert_eq!(
            server.resolve_bodies().len(),
            1,
            "the project cache hit must not place a new resolve call"
        );
        let bare = scope_scratch("local-bare");
        let (global_dir, _, _) = resolve_registry_package(
            "@acme/widget",
            "1.2.0",
            Some(&cfg),
            &BTreeMap::new(),
            Some("1.2.0".to_string()),
            &[],
            Some(&bare),
        )
        .expect("an anchor without the project cache falls back to the network");
        let have = scan_cache_have();
        let want = have
            .iter()
            .find(|h| h.full == "@acme/widget" && h.version == "1.2.0")
            .expect("the network fetch lands in the global store");
        assert_eq!(
            global_dir,
            std::fs::canonicalize(store_pkg_dir(&want.integrity).unwrap()).unwrap()
        );
    }

    #[test]
    fn store_roundtrip_serves_offline_from_store_with_dead_registry() {
        let (_guard, _dir) = isolate_cache("storeround");
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let cfg = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&cfg),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        let store_dir = store_pkg_dir(&digest).expect("store dir for the manifest digest");
        assert_eq!(fetched[0].dir, std::fs::canonicalize(&store_dir).unwrap());
        assert!(
            !package_dir_under(&registry_cache_root(), &cfg.url, "@acme/widget", "1.2.0").exists(),
            "new downloads must not land in the old registry/ tree"
        );
        assert!(store_index_path().is_file(), "installs record the pointer map");
        assert_eq!(
            find_store_pkg(&cfg.url, "@acme/widget", "1.2.0"),
            Some(store_dir.clone())
        );
        let old_root = registry_cache_root();
        let _ = std::fs::remove_dir_all(&old_root);
        assert!(!old_root.exists());
        let dead = registry_cfg("http://127.0.0.1:9");
        let have = scan_cache_have();
        assert!(
            have.iter().any(|h| h.full == "@acme/widget"
                && h.version == "1.2.0"
                && h.integrity == digest),
            "the store scan feeds the have list"
        );
        let (dir, _, version) = resolve_registry_package(
            "@acme/widget",
            "^1.0.0",
            Some(&dead),
            &BTreeMap::new(),
            Some("1.2.0".to_string()),
            &have,
            None,
        )
        .expect("the store serves the pinned resolve with no network");
        assert_eq!(version, "1.2.0");
        assert_eq!(dir, std::fs::canonicalize(&store_dir).unwrap());
        assert!(dir.join("src").join("main.rnx").is_file());
        assert_eq!(cached_integrity(&dir).as_deref(), Some(digest.as_str()));
        let _ = server;
    }

    #[test]
    fn old_registry_tree_is_never_read() {
        let (_guard, _dir) = isolate_cache("storefallback");
        let old = package_dir_under(
            &registry_cache_root(),
            "http://127.0.0.1:9",
            "@acme/widget",
            "1.2.0",
        );
        std::fs::create_dir_all(old.join("src")).unwrap();
        std::fs::write(
            old.join(crate::project::MANIFEST_FILE),
            "export default {\n    project: {\n        name: \"@acme/widget\",\n        version: \"1.2.0\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(old.join("src").join("main.rnx"), "export fn hello(): Int { return 1; }\n")
            .unwrap();
        std::fs::write(old.join(".rnx-integrity"), "@acme/widget\n1.2.0\ntest\n").unwrap();
        assert!(find_store_pkg("http://127.0.0.1:9", "@acme/widget", "1.2.0").is_none());
        assert!(find_cached_package_dir("http://127.0.0.1:9", "@acme/widget", "1.2.0").is_none());
        assert!(scan_cache_have().is_empty());
        let dead = registry_cfg("http://127.0.0.1:9");
        let err = resolve_registry_package(
            "@acme/widget",
            "^1.0.0",
            Some(&dead),
            &BTreeMap::new(),
            Some("1.2.0".to_string()),
            &[],
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E113);
    }

    #[test]
    fn store_concurrent_first_use_writes_once() {
        use std::collections::BTreeSet;
        let (_guard, _dir) = isolate_cache("storeconcurrent");
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = MockRegistry::start_routed(RoutedConfig {
            version_spec: 0,
            resolve_body: resolve_json_static(&[node]),
            packages: vec![RoutedPackage {
                full: "@acme/widget".to_string(),
                version: "1.2.0".to_string(),
                tarball: Vec::new(),
                manifest_override: Some(manifest),
                chunk_override: Some(pairs),
                latency_ms: 0,
            }],
        });
        let cfg = registry_cfg(&server.base);
        std::thread::scope(|s| {
            let handles: Vec<_> = (0..8)
                .map(|_| {
                    s.spawn(|| {
                        ensure_registry_requirements(
                            &requirements_for("@acme/widget", "1.2.0"),
                            Some(&cfg),
                            &BTreeMap::new(),
                            &[],
                        )
                        .unwrap()
                    })
                })
                .collect();
            let mut dirs = BTreeSet::new();
            for h in handles {
                let fetched = h.join().unwrap();
                assert_eq!(fetched.len(), 1);
                dirs.insert(fetched[0].dir.clone());
            }
            assert_eq!(dirs.len(), 1, "concurrent first use converges on one store tree");
        });
        let store_dir = store_pkg_dir(&digest).expect("store dir for the manifest digest");
        assert_eq!(cached_integrity(&store_dir).as_deref(), Some(digest.as_str()));
        assert!(store_dir.join("src").join("main.rnx").is_file());
        let have = scan_cache_have();
        assert_eq!(have.len(), 1);
        assert_eq!(have[0].integrity, digest);
    }

    #[test]
    fn store_corrupt_blob_is_rejected_and_heals_on_refetch() {
        let (_guard, _dir) = isolate_cache("storecorrupt");
        assert!(cas_blob_path("zz").is_none());
        assert!(cas_blob_path(&"ab".repeat(31)).is_none());
        let bytes = b"chunk bytes".to_vec();
        let hash = Sha256::hexdigest(&bytes);
        cas_put(&hash, &bytes).unwrap();
        assert_eq!(cas_get(&hash).unwrap(), Some(bytes.clone()));
        assert!(cas_put(&hash, b"other bytes").is_err());
        let path = cas_blob_path(&hash).expect("cas path");
        std::fs::write(&path, b"tampered").unwrap();
        let err = cas_get(&hash).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("failed its integrity check"), "{}", err.message);
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/widget", "1.2.0");
        let (first_hash, first_bytes) = pairs[0].clone();
        let corrupt_at = cas_blob_path(&first_hash).expect("cas path");
        if let Some(parent) = corrupt_at.parent() {
            std::fs::create_dir_all(parent).unwrap();
        }
        std::fs::write(&corrupt_at, b"tampered").unwrap();
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let cfg = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&cfg),
            &BTreeMap::new(),
            &[],
        )
        .expect("the corrupt blob is dropped and refetched from the network");
        assert_eq!(fetched.len(), 1);
        assert_eq!(cas_get(&first_hash).unwrap(), Some(first_bytes));
        assert!(fetched[0].dir.join("src").join("main.rnx").is_file());
    }

    #[test]
    fn prune_removes_old_tree_but_keeps_the_store() {
        let (_guard, _dir) = isolate_cache("storeprune");
        for (full, version) in [("@acme/old", "1.0.0"), ("@acme/older", "2.0.0")] {
            let dir = package_dir_under(&registry_cache_root(), "http://127.0.0.1:9", full, version);
            std::fs::create_dir_all(dir.join("src")).unwrap();
            std::fs::write(
                dir.join(crate::project::MANIFEST_FILE),
                format!(
                    "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\"\n    }}\n}}\n"
                ),
            )
            .unwrap();
            std::fs::write(dir.join("src").join("main.rnx"), vec![b'x'; 64]).unwrap();
            std::fs::write(dir.join(".rnx-integrity"), format!("{full}\n{version}\ntest\n")).unwrap();
        }
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let cfg = registry_cfg(&server.base);
        ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&cfg),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        let _ = server;
        let store_dir = store_pkg_dir(&digest).expect("store dir for the manifest digest");
        assert!(registry_cache_root().is_dir());
        let freed = prune_old_registry_tree();
        assert!(freed > 0);
        assert!(!registry_cache_root().exists());
        assert_eq!(prune_old_registry_tree(), 0);
        assert!(
            !package_dir_under(&registry_cache_root(), "http://127.0.0.1:9", "@acme/old", "1.0.0")
                .exists(),
            "prune removes the old registry/ tree wholesale"
        );
        assert!(
            store_dir.join("Project.config").is_file(),
            "prune must not touch the content-addressed store"
        );
        assert!(store_usage_bytes() > 0);
    }

    fn artifact_fixture_tree(
        full: &str,
        version: &str,
        bare: &str,
    ) -> (String, Vec<(String, Vec<u8>)>, String) {
        let original = format!(
            "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\"\n    }},\n    entries: {{ main: \"src/main.rnx\" }},\n    artifacts: \"{bare}\"\n}}\n"
        );
        let stored = original.replace(
            &format!("version: \"{version}\""),
            "version: \"0.0.0-published\"",
        );
        let main = b"export fn hello(): Int { return 1; }\n".to_vec();
        let files = vec![
            ("Project.config".to_string(), stored.into_bytes()),
            ("src/main.rnx".to_string(), main),
        ];
        let mut entries = Vec::new();
        let mut pairs = Vec::new();
        for (name, bytes) in &files {
            let h = Sha256::hexdigest(bytes);
            entries.push(format!(
                "{{\"name\": \"{name}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{h}\"]}}",
                bytes.len()
            ));
            pairs.push((h, bytes.clone()));
        }
        let manifest = format!("{{\"entries\": [{}]}}", entries.join(", "));
        let digest = manifest_digest_of(&manifest);
        (manifest, pairs, digest)
    }

    fn artifact_index_for(hash: &str, annotations: &str) -> Vec<u8> {
        format!(
            "{{\"schemaVersion\": 2, \"manifests\": [{{\"digest\": \"sha256:{hash}\", \"annotations\": {{{annotations}}}}}]}}"
        )
        .into_bytes()
    }

    fn runtime_annotations() -> String {
        format!(
            "\"run.rasmalai.kind\": \"binary\", \"run.rasmalai.role\": \"runtime\", \
             \"run.rasmalai.target\": \"x86_64-unknown-linux-gnu\", \
             \"run.rasmalai.generator\": \"rnx-bindgen 0.1.0\", \
             \"run.rasmalai.upstream\": \"sqlite@sha256:{}\", \
             \"org.opencontainers.image.title\": \"libnative.so\"",
            "cd".repeat(32)
        )
    }

    #[test]
    fn tar_header_matches_server_byte_layout() {
        let entry = ChunkEntry {
            name: "a.txt".to_string(),
            size: 5,
            dir: false,
            chunks: vec!["H".repeat(64)],
        };
        let head = tar_entry_header(&entry, "pkg", "1.0.0").unwrap();
        let hex: String = head[..64].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "612e7478740000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );
        let hex2: String = head[256..320].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex2,
            "007573746172003030726e780000000000000000000000000000000000000000000000000000000000726e780000000000000000000000000000000000000000"
        );
        assert_eq!(&head[124..136], b"00000000005\0");
        assert_eq!(head[156], b'0');
        let dir_head = tar_entry_header(
            &ChunkEntry { name: "src".to_string(), size: 0, dir: true, chunks: Vec::new() },
            "pkg",
            "1.0.0",
        )
        .unwrap();
        assert_eq!(dir_head[156], b'5');
        assert!(tar_entry_header(
            &ChunkEntry { name: "x".repeat(101), size: 0, dir: false, chunks: Vec::new() },
            "pkg",
            "1.0.0"
        )
        .is_err());
    }

    #[test]
    fn resolve_retry_backoff_is_capped() {
        assert_eq!(resolve_retry_delay_ms(0), 150);
        assert_eq!(resolve_retry_delay_ms(1), 300);
        assert_eq!(resolve_retry_delay_ms(2), 600);
        assert_eq!(resolve_retry_delay_ms(9), 2400);
        assert_eq!(resolve_retry_delay_ms(u32::MAX), 2400);
    }

    struct FailTransport {
        calls: std::sync::Mutex<u32>,
        message: String,
    }

    impl FailTransport {
        fn refusing() -> FailTransport {
            FailTransport {
                calls: std::sync::Mutex::new(0),
                message: "cannot reach registry `127.0.0.1:9`: connection refused".to_string(),
            }
        }

        fn attempts(&self) -> u32 {
            *self.calls.lock().unwrap_or_else(|e| e.into_inner())
        }
    }

    impl RegistryTransport for FailTransport {
        fn get(&self, _url: &str) -> Result<RegistryReply, Diagnostic> {
            *self.calls.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Err(Diagnostic::new(Code::E108, self.message.clone()))
        }

        fn post_json(&self, _url: &str, _body: &str) -> Result<RegistryReply, Diagnostic> {
            *self.calls.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Err(Diagnostic::new(Code::E108, self.message.clone()))
        }
    }

    #[test]
    fn offline_fast_fail_classification() {
        for offline in [
            "cannot reach registry `127.0.0.1:9`: connection refused",
            "registry request failed: connection refused",
            "registry request failed: dns error: failed to resolve `example.invalid`",
            "registry request failed: network is unreachable",
            "registry resolve failed: No route to host",
            "cannot reach registry: temporary failure in name resolution",
        ] {
            assert!(is_fast_fail_offline(offline), "{offline}");
        }
        for retry in [
            "registry request failed: timed out after 30s",
            "registry request failed: connection reset by peer",
            "boom",
        ] {
            assert!(!is_fast_fail_offline(retry), "{retry}");
        }
    }

    #[test]
    fn offline_connection_refused_fails_fast_with_one_attempt() {
        let fake = FailTransport::refusing();
        let err = http_get_via(&fake, "http://127.0.0.1:9/api/version", "registry discovery")
            .unwrap_err();
        assert_eq!(err.code, Code::E113);
        assert!(err.hint.as_deref().unwrap_or_default().contains("rnx download"), "{err:?}");
        let err = http_post_json_via(&fake, "http://127.0.0.1:9/api/v0/resolve", "{}", "registry resolve")
            .unwrap_err();
        assert_eq!(err.code, Code::E113);
        let err = http_manifest_get_via(&fake, "http://127.0.0.1:9/x/chunks", "chunk manifest of `a@1.0.0`")
            .unwrap_err();
        assert_eq!(err.code, Code::E113);
        assert_eq!(fake.attempts(), 3, "one attempt per call, no doomed retries");
    }

    struct StatusTransport {
        calls: std::sync::Mutex<u32>,
        status: u16,
    }

    impl RegistryTransport for StatusTransport {
        fn get(&self, _url: &str) -> Result<RegistryReply, Diagnostic> {
            *self.calls.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(RegistryReply { status: self.status, spec_header: None, body: Vec::new() })
        }

        fn post_json(&self, _url: &str, _body: &str) -> Result<RegistryReply, Diagnostic> {
            *self.calls.lock().unwrap_or_else(|e| e.into_inner()) += 1;
            Ok(RegistryReply { status: self.status, spec_header: None, body: Vec::new() })
        }
    }

    #[test]
    fn retryable_status_keeps_retrying() {
        let fake = StatusTransport { calls: std::sync::Mutex::new(0), status: 503 };
        let reply = http_post_json_via(&fake, "http://127.0.0.1:9/api/v0/resolve", "{}", "registry resolve").unwrap();
        assert_eq!(reply.status, 503);
        assert_eq!(*fake.calls.lock().unwrap_or_else(|e| e.into_inner()), 4);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn http2_transport_refused_connection_fails_fast_with_one_attempt() {
        struct Counting<'a> {
            inner: &'a dyn RegistryTransport,
            calls: std::sync::Mutex<u32>,
        }
        impl RegistryTransport for Counting<'_> {
            fn get(&self, url: &str) -> Result<RegistryReply, Diagnostic> {
                *self.calls.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                self.inner.get(url)
            }
            fn post_json(&self, url: &str, body: &str) -> Result<RegistryReply, Diagnostic> {
                *self.calls.lock().unwrap_or_else(|e| e.into_inner()) += 1;
                self.inner.post_json(url, body)
            }
        }
        let (_guard, _dir) = isolate_cache("h2offline");
        let h2 = H2Transport::new().unwrap();
        let fake = Counting { inner: &h2, calls: std::sync::Mutex::new(0) };
        let err =
            http_get_via(&fake, "http://127.0.0.1:9/api/version", "registry discovery")
                .unwrap_err();
        assert_eq!(err.code, Code::E113);
        assert!(
            err.message.to_ascii_lowercase().contains("connection refused"),
            "{}",
            err.message
        );
        assert!(
            err.hint.as_deref().unwrap_or_default().contains("rnx download"),
            "{err:?}"
        );
        assert_eq!(
            *fake.calls.lock().unwrap_or_else(|e| e.into_inner()),
            1,
            "one attempt, no doomed retries"
        );
    }

    #[test]
    fn cold_cache_offline_names_package_base_and_remedy() {
        let (_guard, _dir) = isolate_cache("coldoffline");
        let dead = registry_cfg("http://127.0.0.1:9");
        let err = resolve_registry_package(
            "@acme/widget",
            "1.2.0",
            Some(&dead),
            &BTreeMap::new(),
            None,
            &[],
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E113);
        assert!(err.message.contains("@acme/widget"), "{}", err.message);
        assert!(err.message.contains("127.0.0.1:9"), "{}", err.message);
        assert!(
            err.hint.as_deref().unwrap_or_default().contains("rnx download"),
            "{err:?}"
        );
    }

    fn plant_pinned_cache(full: &str, version: &str, engine: &str) -> PathBuf {
        plant_store_package(
            full,
            version,
            &"cd".repeat(32),
            engine,
            "export fn hello(): Int { return 1; }\n",
        )
    }

    #[test]
    fn pinned_cache_hit_enforces_engine_after_upgrade() {
        let (_guard, _dir) = isolate_cache("pinengine");
        let dead = registry_cfg("http://127.0.0.1:9");
        plant_pinned_cache("@acme/widget", "1.2.0", ">=99.0.0");
        let err = resolve_registry_package(
            "@acme/widget",
            "^1.0.0",
            Some(&dead),
            &BTreeMap::new(),
            Some("1.2.0".to_string()),
            &[],
            None,
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E114);
        assert!(err.message.contains("needs engine"), "{}", err.message);
        assert!(err.message.contains("@acme/widget@1.2.0"), "{}", err.message);
        let hint = err.hint.as_deref().unwrap_or_default();
        assert!(hint.contains("rnx update"), "{err:?}");
        assert!(hint.contains("rnx download"), "{err:?}");
    }

    #[test]
    fn pinned_cache_hit_serves_yanked_versions_without_network() {
        let (_guard, _dir) = isolate_cache("pinyank");
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_engine_json("@acme/widget", "1.2.0", &digest, true, "");
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let empty = BTreeMap::new();
        let (dir, _, ver) = resolve_registry_package(
            "@acme/widget",
            "^1.0.0",
            Some(&default),
            &empty,
            None,
            &[],
            None,
        )
        .unwrap();
        assert_eq!(ver, "1.2.0");
        assert!(cached_is_yanked(&dir));
        let served = server.requests().len();
        assert!(served > 0);
        let (dir2, _, ver2) = resolve_registry_package(
            "@acme/widget",
            "^1.0.0",
            Some(&default),
            &empty,
            Some("1.2.0".to_string()),
            &[],
            None,
        )
        .unwrap();
        assert_eq!(ver2, "1.2.0");
        assert_eq!(dir2, dir);
        assert_eq!(server.requests().len(), served, "the pinned hit stays off the wire");
    }

    fn big_entries_array(count: usize) -> String {
        let mut out = String::from('[');
        for i in 0..count {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!(
                "{{\"name\": \"src/f{i:05}.rnx\", \"size\": 1, \"dir\": false, \"chunks\": []}}"
            ));
        }
        out.push(']');
        out
    }

    fn big_entries_json(count: usize) -> String {
        format!("{{\"entries\": {}}}", big_entries_array(count))
    }

    #[test]
    fn chunk_entry_count_over_limit_fails_closed_on_inline_entries() {
        let body = format!(
            "{{\"resolved\": {{\"a\": \"1.0.0\"}}, \"base\": \"\", \"levels\": [[{{\"full\": \"a\", \"version\": \"1.0.0\", \"path\": \"a@1.0.0/chunks\", \"integrity\": \"x\", \"engineRange\": \"\", \"deps\": [], \"yanked\": false, \"entries\": {entries}}}]]}}",
            entries = big_entries_array(MAX_CHUNK_ENTRIES + 1)
        );
        let err = parse_resolve(body.as_bytes()).unwrap_err();
        assert_eq!(err.code, Code::E116);
        assert!(err.message.contains("a@1.0.0"), "{}", err.message);
        assert!(err.message.contains("4096"), "{}", err.message);
        assert!(
            err.hint.as_deref().unwrap_or_default().contains("rnx update"),
            "{err:?}"
        );
    }

    #[test]
    fn chunk_entry_count_over_limit_fails_closed_on_manifest() {
        let (_guard, _dir) = isolate_cache("entrycap");
        let (_manifest, pairs, digest, _, _) = fixture_tree("@acme/widget", "1.2.0");
        let over = big_entries_json(MAX_CHUNK_ENTRIES + 1);
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(over),
            chunk_override: Some(pairs),
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E116);
        assert!(err.message.contains("@acme/widget@1.2.0"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
    }

    #[test]
    fn resolve_503_retries_before_failing() {
        let (_guard, _dir) = isolate_cache("retry503");
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 503,
            resolve_body: "store unavailable".to_string(),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "9.9.9"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert!(err.message.contains("503"), "{}", err.message);
        assert_eq!(server.resolve_bodies().len(), 4);
    }

    // A registry that answers with a status this client does not know must
    // not turn into a parse or transport error: the raw status is printed and
    // the client is named as out of date.
    #[test]
    fn resolve_unknown_status_prints_the_status_and_names_the_outdated_client() {
        let (_guard, _dir) = isolate_cache("unknownstatus");
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 599,
            resolve_body: "{\"code\": \"teapot\", \"message\": \"the registry is a teapot\"}"
                .to_string(),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let err = resolve_requirements(
            &server.base,
            &requirements_for("@acme/widget", "*"),
            &[],
            false,
        )
        .unwrap_err();
        assert!(err.message.contains("599"), "{}", err.message);
        assert!(
            err.message.contains("does not understand"),
            "{}",
            err.message
        );
        assert!(
            err.message.contains("update the client"),
            "{}",
            err.message
        );
        // The structured body of the server rides along.
        assert!(
            err.message.contains("the registry is a teapot"),
            "{}",
            err.message
        );
        assert!(!err.message.contains("malformed"), "{}", err.message);
        assert!(!err.message.contains("failed with HTTP"), "{}", err.message);
    }

    #[test]
    fn resolve_unknown_status_without_a_body_still_names_the_status() {
        let (_guard, _dir) = isolate_cache("unknownstatusbare");
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 460,
            resolve_body: String::new(),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let err = resolve_requirements(
            &server.base,
            &requirements_for("@acme/widget", "*"),
            &[],
            false,
        )
        .unwrap_err();
        assert!(err.message.contains("460"), "{}", err.message);
        assert!(
            err.message.contains("update the client"),
            "{}",
            err.message
        );
    }

    // 422 is a status this client knows: the registry explains the failure
    // in the body and the client reports that, not an outdated client.
    #[test]
    fn resolve_422_budget_is_reported_as_the_registry_explained_it() {
        let (_guard, _dir) = isolate_cache("known422");
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 422,
            resolve_body: "{\"code\": \"budget-exceeded\", \"message\": \"resolve budget exceeded: closure over 512 nodes\", \"requestId\": \"r-1\"}".to_string(),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let err = resolve_requirements(
            &server.base,
            &requirements_for("@acme/widget", "*"),
            &[],
            false,
        )
        .unwrap_err();
        assert!(
            err.message.contains("resolve budget exceeded"),
            "{}",
            err.message
        );
        assert!(
            !err.message.contains("does not understand"),
            "{}",
            err.message
        );
        assert!(!err.message.contains("update the client"), "{}", err.message);
    }

    #[test]
    fn resolve_404_not_found_is_reported_as_the_registry_explained_it() {
        let (_guard, _dir) = isolate_cache("known404");
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 404,
            resolve_body: "{\"code\": \"not-found\", \"message\": \"package @acme/widget does not exist\"}"
                .to_string(),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let err = resolve_requirements(
            &server.base,
            &requirements_for("@acme/widget", "*"),
            &[],
            false,
        )
        .unwrap_err();
        assert!(
            err.message.contains("package @acme/widget does not exist"),
            "{}",
            err.message
        );
        assert!(
            !err.message.contains("does not understand"),
            "{}",
            err.message
        );
    }

    #[test]
    fn pinned_cache_hit_skips_network() {
        let (_guard, _dir) = isolate_cache("pinhit");
        let (manifest, pairs, digest, _pc, _main) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let empty = BTreeMap::new();
        let (dir, _, ver) =
            resolve_registry_package("@acme/widget", "^1.0.0", Some(&default), &empty, None, &[], None).unwrap();
        assert_eq!(ver, "1.2.0");
        assert!(dir.is_dir());
        let served = server.requests().len();
        assert!(served > 0);
        let (dir2, _, ver2) =
            resolve_registry_package("@acme/widget", "^1.0.0", Some(&default), &empty, Some("1.2.0".to_string()), &[], None).unwrap();
        assert_eq!(ver2, "1.2.0");
        assert_eq!(dir2, dir);
        assert_eq!(server.requests().len(), served);
    }

    fn write_vendor_package(root: &Path, full: &str, version: &str, main: &str) -> PathBuf {
        let dir = root.join(full);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join(crate::project::MANIFEST_FILE),
            format!(
                "export default {{\n    project: {{\n        name: \"{full}\",\n        version: \"{version}\"\n    }},\n    entries: {{\n        main: \"src/main.rnx\"\n    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), main).unwrap();
        dir
    }

    #[test]
    fn vendor_override_beats_warm_cache_with_no_network() {
        let (_guard, _dir) = isolate_cache("vendorbeat");
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/vendor-a", "1.2.0");
        let node = node_json("@acme/vendor-a", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let empty = BTreeMap::new();
        let (cached, _, _) = resolve_registry_package(
            "@acme/vendor-a",
            "^1.0.0",
            Some(&default),
            &empty,
            None,
            &[],
            None,
        )
        .unwrap();
        let served = server.requests().len();
        assert!(served > 0);
        let anchor = crate::scratch::dir("vendorbeat-anchor");
        let vendor =
            write_vendor_package(&anchor.join(crate::fetch::VENDOR_DIR), "@acme/vendor-a", "1.2.0", "vendored\n");
        let (dir, _, ver) = resolve_registry_package(
            "@acme/vendor-a",
            "^1.0.0",
            Some(&default),
            &empty,
            Some("1.2.0".to_string()),
            &[],
            Some(&anchor),
        )
        .unwrap();
        assert_eq!(ver, "1.2.0");
        assert_eq!(dir, std::fs::canonicalize(&vendor).unwrap());
        assert_ne!(dir, cached);
        assert!(local_override_used("@acme/vendor-a"));
        assert_eq!(server.requests().len(), served);
    }

    #[test]
    fn vendor_env_dir_wins_over_project_vendor() {
        let (_guard, _dir) = isolate_cache("vendorenv");
        let dead = registry_cfg("http://127.0.0.1:9");
        let empty = BTreeMap::new();
        let anchor = crate::scratch::dir("vendorenv-anchor");
        write_vendor_package(&anchor.join(crate::fetch::VENDOR_DIR), "@acme/vendor-b", "9.9.9", "stale\n");
        let env_root = crate::scratch::dir("vendorenv-env");
        let env_pkg = write_vendor_package(&env_root, "@acme/vendor-b", "1.2.0", "env\n");
        let _env = set_vendor_dir(env_root.path());
        let (dir, _, ver) = resolve_registry_package(
            "@acme/vendor-b",
            "^1.0.0",
            Some(&dead),
            &empty,
            Some("1.2.0".to_string()),
            &[],
            Some(&anchor),
        )
        .unwrap();
        assert_eq!(ver, "1.2.0");
        assert_eq!(dir, std::fs::canonicalize(&env_pkg).unwrap());
        assert!(local_override_used("@acme/vendor-b"));
    }

    #[test]
    fn vendor_version_mismatch_fails_closed() {
        let (_guard, _dir) = isolate_cache("vendormismatch");
        let dead = registry_cfg("http://127.0.0.1:9");
        let empty = BTreeMap::new();
        let anchor = crate::scratch::dir("vendormismatch-anchor");
        write_vendor_package(&anchor.join(crate::fetch::VENDOR_DIR), "@acme/vendor-c", "2.0.0", "new\n");
        let err = resolve_registry_package(
            "@acme/vendor-c",
            "^1.0.0",
            Some(&dead),
            &empty,
            Some("1.2.0".to_string()),
            &[],
            Some(&anchor),
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("2.0.0"), "{}", err.message);
        assert!(err.message.contains("1.2.0"), "{}", err.message);
        let err = resolve_registry_package(
            "@acme/vendor-c",
            "^1.0.0",
            Some(&dead),
            &empty,
            None,
            &[],
            Some(&anchor),
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("does not satisfy"), "{}", err.message);
    }

    #[test]
    fn vendor_name_mismatch_and_artifacts_fail_closed() {
        let (_guard, _dir) = isolate_cache("vendorbad");
        let dead = registry_cfg("http://127.0.0.1:9");
        let empty = BTreeMap::new();
        let anchor = crate::scratch::dir("vendorbad-anchor");
        write_vendor_package(&anchor.join(crate::fetch::VENDOR_DIR), "@acme/vendor-d", "1.0.0", "x\n");
        std::fs::write(
            anchor
                .join(crate::fetch::VENDOR_DIR)
                .join("@acme/vendor-d")
                .join(crate::project::MANIFEST_FILE),
            "export default {\n    project: {\n        name: \"@acme/other\",\n        version: \"1.0.0\"\n    }\n}\n",
        )
        .unwrap();
        let err = resolve_registry_package(
            "@acme/vendor-d",
            "*",
            Some(&dead),
            &empty,
            None,
            &[],
            Some(&anchor),
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("@acme/other"), "{}", err.message);
        let art = anchor.join(crate::fetch::VENDOR_DIR).join("@acme/vendor-e");
        std::fs::create_dir_all(art.join("src")).unwrap();
        std::fs::write(
            art.join(crate::project::MANIFEST_FILE),
            "export default {\n    project: {\n        name: \"@acme/vendor-e\",\n        version: \"1.0.0\"\n    },\n    artifacts: \"ghcr.io/org/e\"\n}\n",
        )
        .unwrap();
        std::fs::write(art.join("src").join("main.rnx"), "x\n").unwrap();
        let err = resolve_registry_package(
            "@acme/vendor-e",
            "*",
            Some(&dead),
            &empty,
            None,
            &[],
            Some(&anchor),
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("artifacts"), "{}", err.message);
    }

    #[test]
    fn missing_vendor_falls_through_to_registry() {
        let (_guard, _dir) = isolate_cache("vendorfall");
        let (manifest, pairs, digest, _, _) = fixture_tree("@acme/vendor-f", "1.0.0");
        let node = node_json("@acme/vendor-f", "1.0.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let empty = BTreeMap::new();
        let anchor = crate::scratch::dir("vendorfall-anchor");
        let (dir, _, ver) = resolve_registry_package(
            "@acme/vendor-f",
            "*",
            Some(&default),
            &empty,
            None,
            &[],
            Some(&anchor),
        )
        .unwrap();
        assert_eq!(ver, "1.0.0");
        assert!(dir.is_dir());
        assert!(!local_override_used("@acme/vendor-f"));
        assert!(!server.requests().is_empty());
    }

    #[test]
    fn api_base_prefers_omitted_and_relative() {
        assert_eq!(
            api_base_for("", "https://r.example.com"),
            "https://r.example.com/api/v0/packages"
        );
        assert_eq!(
            api_base_for("   ", "https://r.example.com/"),
            "https://r.example.com/api/v0/packages"
        );
        assert_eq!(
            api_base_for("/api/v0/custom", "https://r.example.com/api/v0/packages"),
            "https://r.example.com/api/v0/custom"
        );
        assert_eq!(
            api_base_for("https://mirror.example.net/x", "https://r.example.com"),
            "https://mirror.example.net/x"
        );
        assert_eq!(origin_of("https://r.example.com/a/b"), "https://r.example.com");
        assert_eq!(origin_of("http://127.0.0.1:9/x"), "http://127.0.0.1:9");
    }

    #[test]
    fn chunks_transport_reconstructs_split_tar() {
        let (_guard, _dir) = isolate_cache("splittar");
        let mut buf = Vec::new();
        {
            let mut tar = crate::tar::TarWriter::new(&mut buf);
            tar.add_file("Project.config", b"export default {}\n").unwrap();
            tar.add_file("src/main.rnx", b"export fn hello(): Int { return 1; }\n").unwrap();
            tar.finish().unwrap();
        }
        let tar_sha = Sha256::hexdigest(&buf);
        let files = vec![
            (
                "Project.config",
                b"export default {\n    project: {\n        name: \"@acme/widget\",\n        version: \"0.0.0-published\"\n    }\n}\n".to_vec(),
            ),
            ("src/main.rnx", b"export fn hello(): Int { return 1; }\n".to_vec()),
        ];
        let mut manifest_entries = Vec::new();
        let mut bodies = Vec::new();
        for (name, bytes) in &files {
            let h = Sha256::hexdigest(bytes);
            manifest_entries.push(format!(
                "{{\"name\": \"{name}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{h}\"]}}",
                bytes.len()
            ));
            bodies.push((h, bytes.clone()));
        }
        let manifest = format!("{{\"entries\": [{}]}}", manifest_entries.join(", "));
        let node = node_json("@acme/widget", "1.2.0", &manifest_digest_of(&manifest), false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(bodies),
        });
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        let main = std::fs::read(fetched[0].dir.join("src").join("main.rnx")).unwrap();
        assert_eq!(main, b"export fn hello(): Int { return 1; }\n");
    }

    // Chunk bodies are read through the edge namespace, so a cached body that
    // does not hash to the hash it was requested under must be rejected, and a
    // guarded run must still salt the generation-sensitive manifest.
    #[test]
    fn edge_chunk_that_fails_its_hash_never_reaches_the_cache() {
        let (_guard, _dir) = isolate_cache("edgechunkhash");
        let good = b"export fn hello(): Int { return 1; }\n".to_vec();
        let hash = Sha256::hexdigest(&good);
        let node = node_json("@acme/widget", "1.0.0", &hash, false);
        let manifest = format!(
            "{{\"entries\": [{{\"name\": \"\", \"size\": {}, \"dir\": false, \"chunks\": [\"{hash}\"]}}]}}",
            good.len()
        );
        let config = MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(vec![(hash.clone(), b"tampered body".to_vec())]),
        };
        let server = MockRegistry::start(config);
        let default = registry_cfg(&server.base);
        let _guard = origin_direct_guard();
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert!(
            err.message.contains("failed its integrity check"),
            "{}",
            err.message
        );
        assert!(
            find_cached_package_dir(&default.url, "@acme/widget", "1.0.0").is_none(),
            "a failed download must leave nothing resolvable in the store"
        );
        let logged = server.requests();
        let chunks: Vec<_> = logged
            .iter()
            .filter(|(m, p, _)| m == "GET" && p.contains("/chunk/"))
            .collect();
        let manifests: Vec<_> = logged
            .iter()
            .filter(|(m, p, _)| m == "GET" && p.ends_with("/chunks?origin-direct=1"))
            .collect();
        assert!(
            chunks.iter().any(|(_, p, _)| !p.contains("origin-direct=1")),
            "a guarded run must read chunk bodies from the edge namespace"
        );
        assert!(
            manifests.iter().all(|(_, p, _)| p.contains("origin-direct=1"))
                && !manifests.is_empty(),
            "the generation-sensitive manifest must stay origin-direct"
        );
    }

    #[test]
    fn parallel_fetch_reconstructs_many_chunks() {
        let (_guard, _dir) = isolate_cache("parachunks");
        let mut files: Vec<(String, Vec<u8>)> = vec![(
            "Project.config".to_string(),
            b"export default {\n    project: {\n        name: \"@acme/widget\",\n        version: \"0.0.0-published\"\n    }\n}\n".to_vec(),
        )];
        for i in 0..12u32 {
            files.push((
                format!("src/f{i:02}.rnx"),
                format!("export fn f{i:02}(): Int {{ return {i}; }}\n").into_bytes(),
            ));
        }
        let mut entries: Vec<ChunkEntry> = Vec::new();
        let mut bodies: Vec<(String, Vec<u8>)> = Vec::new();
        for (name, bytes) in &files {
            let h = Sha256::hexdigest(bytes);
            entries.push(ChunkEntry {
                name: name.clone(),
                size: bytes.len() as u64,
                dir: false,
                chunks: vec![h.clone()],
            });
            bodies.push((h, bytes.clone()));
        }
        let mut by_hash: BTreeMap<String, Vec<u8>> = BTreeMap::new();
        for (h, b) in &bodies {
            by_hash.insert(h.clone(), b.clone());
        }
        let sha = crate::checksum::manifest_digest_hex(
            &entries
                .iter()
                .map(|e| crate::checksum::DigestEntry {
                    name: e.name.clone(),
                    size: e.size,
                    dir: e.dir,
                    chunks: e.chunks.clone(),
                })
                .collect::<Vec<_>>(),
        )
        .unwrap();
        let manifest = format!(
            "{{\"entries\": [{}]}}",
            entries
                .iter()
                .map(|e| format!(
                    "{{\"name\": \"{}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{}\"]}}",
                    e.name, e.size, e.chunks[0]
                ))
                .collect::<Vec<_>>()
                .join(", ")
        );
        let node = node_json("@acme/widget", "1.2.0", &sha, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(bodies),
        });
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        for i in 0..12u32 {
            let got = std::fs::read(fetched[0].dir.join(format!("src/f{i:02}.rnx"))).unwrap();
            assert_eq!(got, format!("export fn f{i:02}(): Int {{ return {i}; }}\n").into_bytes());
        }
        let chunk_gets = server
            .requests()
            .iter()
            .filter(|(m, p, _)| m == "GET" && p.contains("/chunk/"))
            .count();
        assert_eq!(chunk_gets, 13);
    }

    #[test]
    fn parallel_fetch_reports_missing_chunk() {
        let (_guard, _dir) = isolate_cache("paramissing");
        let files = vec![
            ("Project.config", b"export default {}\n".to_vec()),
            ("src/a.rnx", b"export fn a(): Int { return 1; }\n".to_vec()),
            ("src/b.rnx", b"export fn b(): Int { return 2; }\n".to_vec()),
        ];
        let mut manifest_entries = Vec::new();
        let mut bodies = Vec::new();
        for (name, bytes) in &files {
            let h = Sha256::hexdigest(bytes);
            manifest_entries.push(format!(
                "{{\"name\": \"{name}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{h}\"]}}",
                bytes.len()
            ));
            bodies.push((h, bytes.clone()));
        }
        bodies.pop();
        let manifest = format!("{{\"entries\": [{}]}}", manifest_entries.join(", "));
        let node = node_json("@acme/widget", "1.2.0", &"0".repeat(64), false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(bodies),
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("missing"), "{}", err.message);
        let _ = server;
    }

    fn direct_fixture() -> (Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, Vec<u8>, String, String) {
        let installed =
            b"export default {\n    project: {\n        name: \"test/direct\",\n        version: \"1.0.0\"\n    }\n}\n".to_vec();
        let stored = crate::project::redact::redact_project_version(
            std::str::from_utf8(&installed).unwrap(),
        )
        .unwrap()
        .into_bytes();
        let main_a = b"export fn main(): Int {\n".to_vec();
        let main_b = b"    return 40 + 2;\n}\n".to_vec();
        let util = b"export fn util(): Int { return 2; }\n".to_vec();
        let pc_hash = Sha256::hexdigest(&stored);
        let main_a_hash = Sha256::hexdigest(&main_a);
        let main_b_hash = Sha256::hexdigest(&main_b);
        let util_hash = Sha256::hexdigest(&util);
        let mut main = main_a.clone();
        main.extend_from_slice(&main_b);
        let manifest = format!(
            "{{\"entries\": [{{\"name\": \"src/lib/util.rnx\", \"size\": {}, \"dir\": false, \"chunks\": [\"{util_hash}\"]}}, {{\"name\": \"assets\", \"size\": 0, \"dir\": true, \"chunks\": []}}, {{\"name\": \"Project.config\", \"size\": {}, \"dir\": false, \"chunks\": [\"{pc_hash}\"]}}, {{\"name\": \"src/main.rnx\", \"size\": {}, \"dir\": false, \"chunks\": [\"{main_a_hash}\", \"{main_b_hash}\"]}}, {{\"name\": \"src/lib\", \"size\": 0, \"dir\": true, \"chunks\": []}}]}}",
            util.len(),
            stored.len(),
            main.len()
        );
        let digest = manifest_digest_of(&manifest);
        (installed, stored, main_a, main_b, util, manifest, digest)
    }

    #[test]
    fn direct_write_matches_reassembled_tree() {
        let (_guard, _dir) = isolate_cache("directident");
        let (pc, stored, main_a, main_b, util, manifest, digest) = direct_fixture();
        let mut main = main_a.clone();
        main.extend_from_slice(&main_b);
        let node = node_json("test/direct", "1.0.0", &digest, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(vec![
                (Sha256::hexdigest(&stored), stored.clone()),
                (
                    "6799733ffc2c5698ee41581f27ca21cba32a36f8a0aa0806f8669b5efd287046".to_string(),
                    main_a.clone(),
                ),
                (
                    "6a83441593b09c0667afc4db47a86bd12244442c9cd8fb8f43bd7728d3f525e4".to_string(),
                    main_b.clone(),
                ),
                (
                    "2f8af1e91c09e64e5192906fb40a356174bf7b7d3c08f97595776b47a153fda3".to_string(),
                    util.clone(),
                ),
            ]),
        });
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("test/direct", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        let root = &fetched[0].dir;
        assert_eq!(std::fs::read(root.join("Project.config")).unwrap(), pc);
        assert_eq!(std::fs::read(root.join("src").join("main.rnx")).unwrap(), main);
        assert_eq!(std::fs::read(root.join("src").join("lib").join("util.rnx")).unwrap(), util);
        assert_eq!(
            Sha256::hexdigest(&std::fs::read(root.join("Project.config")).unwrap()),
            Sha256::hexdigest(&pc)
        );
        assert_eq!(
            Sha256::hexdigest(&std::fs::read(root.join("src").join("main.rnx")).unwrap()),
            "3321a1c66144f749241a4e83af446d4e3fa56d4731d7bad02e6c14b9d4fdb2b7"
        );
        assert_eq!(
            Sha256::hexdigest(&std::fs::read(root.join("src").join("lib").join("util.rnx")).unwrap()),
            "2f8af1e91c09e64e5192906fb40a356174bf7b7d3c08f97595776b47a153fda3"
        );
        assert!(root.join("assets").is_dir());
        assert!(root.join("src").join("lib").is_dir());
        let have = scan_cache_have();
        assert_eq!(have.len(), 1);
        assert_eq!(have[0].integrity, digest);
    }

    #[test]
    fn direct_write_rejects_tampered_chunk() {
        let (_guard, _dir) = isolate_cache("directtamper");
        let (_pc, stored, _main_a, main_b, util, manifest, digest) = direct_fixture();
        let node = node_json("test/direct", "1.0.0", &digest, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(vec![
                (Sha256::hexdigest(&stored), stored),
                (
                    "6799733ffc2c5698ee41581f27ca21cba32a36f8a0aa0806f8669b5efd287046".to_string(),
                    b"export fn MAIN(): Int {\n".to_vec(),
                ),
                (
                    "6a83441593b09c0667afc4db47a86bd12244442c9cd8fb8f43bd7728d3f525e4".to_string(),
                    main_b,
                ),
                (
                    "2f8af1e91c09e64e5192906fb40a356174bf7b7d3c08f97595776b47a153fda3".to_string(),
                    util,
                ),
            ]),
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/direct", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("6799733ffc2c5698ee41581f27ca21cba32a36f8a0aa0806f8669b5efd287046"), "{}", err.message);
        assert!(err.message.contains("failed its integrity check"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
    }

    #[test]
    fn direct_write_rejects_truncated_chunk() {
        let (_guard, _dir) = isolate_cache("directtrunc");
        let (_pc, stored, main_a, main_b, util, manifest, digest) = direct_fixture();
        let mut cut = main_b.clone();
        cut.pop();
        let node = node_json("test/direct", "1.0.0", &digest, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(vec![
                (Sha256::hexdigest(&stored), stored),
                (
                    "6799733ffc2c5698ee41581f27ca21cba32a36f8a0aa0806f8669b5efd287046".to_string(),
                    main_a,
                ),
                (
                    "6a83441593b09c0667afc4db47a86bd12244442c9cd8fb8f43bd7728d3f525e4".to_string(),
                    cut,
                ),
                (
                    "2f8af1e91c09e64e5192906fb40a356174bf7b7d3c08f97595776b47a153fda3".to_string(),
                    util,
                ),
            ]),
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/direct", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("6a83441593b09c0667afc4db47a86bd12244442c9cd8fb8f43bd7728d3f525e4"), "{}", err.message);
        assert!(err.message.contains("failed its integrity check"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
    }

    #[test]
    fn direct_write_rejects_entry_size_mismatch() {
        let (_guard, _dir) = isolate_cache("directsize");
        let (_installed, stored, main_a, main_b, util, _, _) = direct_fixture();
        let stored_hash = Sha256::hexdigest(&stored);
        let mut main = main_a.clone();
        main.extend_from_slice(&main_b);
        let manifest = format!(
            "{{\"entries\": [{{\"name\": \"Project.config\", \"size\": {}, \"dir\": false, \"chunks\": [\"{stored_hash}\"]}}, {{\"name\": \"src/main.rnx\", \"size\": {}, \"dir\": false, \"chunks\": [\"6799733ffc2c5698ee41581f27ca21cba32a36f8a0aa0806f8669b5efd287046\", \"6a83441593b09c0667afc4db47a86bd12244442c9cd8fb8f43bd7728d3f525e4\"]}}]}}",
            stored.len(),
            main.len() - 1
        );
        let node = node_json("test/direct", "1.0.0", &manifest_digest_of(&manifest), false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(vec![
                (stored_hash, stored),
                (
                    "6799733ffc2c5698ee41581f27ca21cba32a36f8a0aa0806f8669b5efd287046".to_string(),
                    main_a,
                ),
                (
                    "6a83441593b09c0667afc4db47a86bd12244442c9cd8fb8f43bd7728d3f525e4".to_string(),
                    main_b,
                ),
                (
                    "2f8af1e91c09e64e5192906fb40a356174bf7b7d3c08f97595776b47a153fda3".to_string(),
                    util,
                ),
            ]),
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/direct", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("checksum mismatch"), "{}", err.message);
        assert!(err.message.contains("src/main.rnx"), "{}", err.message);
        assert!(err.message.contains("44"), "{}", err.message);
        assert!(err.message.contains("45"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
        let _ = server;
    }

    #[test]
    fn direct_write_rejects_unsafe_path() {
        let (_guard, _dir) = isolate_cache("directunsafe");
        let evil = b"x".to_vec();
        let evil_hash = Sha256::hexdigest(&evil);
        let manifest = format!(
            "{{\"entries\": [{{\"name\": \"../evil.rnx\", \"size\": 1, \"dir\": false, \"chunks\": [\"{evil_hash}\"]}}, {{\"name\": \"Project.config\", \"size\": 17, \"dir\": false, \"chunks\": [\"{evil_hash}\"]}}]}}"
        );
        let node = node_json("test/direct", "1.0.0", &manifest_digest_of(&manifest), false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(vec![(evil_hash, evil)]),
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/direct", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("unsafe file path"), "{}", err.message);
        assert!(err.message.contains("../evil.rnx"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
        let _ = server;
    }

    #[test]
    fn spec_mismatch_refuses_naming_both() {
        let (_guard, _dir) = isolate_cache("spec");
        let (gz, _sha) = fixture_tarball("@acme/widget", "1.2.0");
        let server = start_ok(2, &[], gz, 200, "");
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert!(err.message.contains("spec 2"), "{}", err.message);
        assert!(err.message.contains("supports 0"), "{}", err.message);
    }

    #[test]
    fn resolve_then_fetch_roundtrip() {
        let (_guard, _dir) = isolate_cache("roundtrip");
        let (manifest, pairs, digest, installed_pc, _installed_main) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(fetched[0].name, "@acme/widget");
        assert_eq!(fetched[0].version, "1.2.0");
        let text = std::fs::read_to_string(fetched[0].dir.join("Project.config")).unwrap();
        assert!(text.contains("@acme/widget"), "{text}");
        let main = std::fs::read_to_string(fetched[0].dir.join("src").join("main.rnx")).unwrap();
        assert!(main.contains("hello"), "{main}");
        let have = scan_cache_have();
        assert_eq!(have.len(), 1);
        assert_eq!(have[0].full, "@acme/widget");
        assert_eq!(have[0].integrity, digest);
        let bodies = server.resolve_bodies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(requirement(&bodies[0], "@acme/widget"), "^1.0.0");
    }

    #[test]
    fn have_skips_redownload() {
        let (_guard, _dir) = isolate_cache("delta");
        let (manifest, pairs, digest, _pc, _main) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let reqs = requirements_for("@acme/widget", "^1.0.0");
        ensure_registry_requirements(&reqs, Some(&default), &BTreeMap::new(), &[]).unwrap();
        assert_eq!(server.count_get("chunks"), 1);
        let have = scan_cache_have();
        assert_eq!(have.len(), 1);
        ensure_registry_requirements(&reqs, Some(&default), &BTreeMap::new(), &have).unwrap();
        assert_eq!(server.count_get("chunks"), 1);
        let bodies = server.resolve_bodies();
        assert_eq!(bodies.len(), 2);
        assert_eq!(
            have_entry(&bodies[1], 0),
            ("@acme/widget".to_string(), "1.2.0".to_string(), digest)
        );
    }

    #[test]
    fn manifest_404_maps_to_does_not_exist() {
        let (_guard, _dir) = isolate_cache("notfound");
        let server = start_ok(
            0,
            &[node_json("@acme/widget", "9.9.9", &"0".repeat(64), false)],
            Vec::new(),
            404,
            "{\"code\": \"not-found\", \"message\": \"package @acme/widget@9.9.9 does not exist\"}",
        );
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "9.9.9"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert!(err.message.contains("does not exist"), "{}", err.message);
        assert!(err.message.contains("@acme/widget@9.9.9"), "{}", err.message);
    }

    #[test]
    fn manifest_410_maps_to_withdrawn() {
        let (_guard, _dir) = isolate_cache("withdrawn");
        let server = start_ok(
            0,
            &[node_json("@acme/widget", "1.2.0", &"0".repeat(64), false)],
            Vec::new(),
            410,
            "{\"code\": \"withdrawn\", \"message\": \"version 1.2.0 withdrawn: sec\", \"reason\": \"sec\"}",
        );
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert!(err.message.contains("withdrawn"), "{}", err.message);
        assert!(err.message.contains("upgrade"), "{}", err.message);
    }

    #[test]
    fn integrity_mismatch_aborts_without_cache() {
        let (_guard, _dir) = isolate_cache("integrity");
        let (_gz, sha) = fixture_tarball("@acme/widget", "1.2.0");
        let server = start_ok(
            0,
            &[node_json("@acme/widget", "1.2.0", &sha, false)],
            b"corrupt bytes".to_vec(),
            200,
            "",
        );
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert!(err.message.contains("checksum mismatch"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
    }

    #[test]
    fn engine_range_mismatch_blocks_fetch() {
        let (_guard, _dir) = isolate_cache("engine");
        let (gz, sha) = fixture_tarball("@acme/widget", "1.2.0");
        let server = start_ok(
            0,
            &[node_engine_json("@acme/widget", "1.2.0", &sha, false, ">=99.0.0")],
            gz,
            200,
            "",
        );
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "1.2.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E114);
        assert!(err.message.contains("needs engine"), "{}", err.message);
        assert!(
            err.hint.as_deref().unwrap_or_default().contains("rnx update"),
            "{err:?}"
        );
        assert_eq!(server.count_get("chunks"), 0);
    }

    fn routed_registry_17(latency_ms: u64) -> (MockRegistry, Vec<(String, String, String)>) {
        let mut pkgs = Vec::new();
        let mut nodes = Vec::new();
        let mut meta = Vec::new();
        for i in 0..17u32 {
            let full = format!("test/pkg{i:02}");
            let (pkg, sha) = routed_fixture(&full, "1.0.0", latency_ms);
            nodes.push(node_json(&full, "1.0.0", &sha, false));
            meta.push((full, "1.0.0".to_string(), sha));
            pkgs.push(pkg);
        }
        let server = MockRegistry::start_routed(RoutedConfig {
            version_spec: 0,
            resolve_body: resolve_json_static(&nodes),
            packages: pkgs,
        });
        (server, meta)
    }

    fn chunk_route_gets(server: &MockRegistry) -> usize {
        server
            .requests()
            .iter()
            .filter(|(m, p, _)| m == "GET" && p.split('?').next().unwrap_or_default().contains("chunk"))
            .count()
    }

    fn chunk_body_hashes(server: &MockRegistry) -> std::collections::BTreeSet<String> {
        server
            .requests()
            .iter()
            .filter_map(|(m, p, _)| {
                if m != "GET" {
                    return None;
                }
                let path = p.split('?').next().unwrap_or_default();
                path.rsplit_once("/chunk/").map(|(_, h)| h.to_string())
            })
            .collect()
    }

    #[test]
    fn multipackage_fetch_lands_every_package_byte_identical() {
        let (_guard, _dir) = isolate_cache("multipkg");
        let (server, meta) = routed_registry_17(100);
        let default = registry_cfg(&server.base);
        let reqs: BTreeMap<String, String> =
            meta.iter().map(|(f, _, _)| (f.clone(), "1.0.0".to_string())).collect();
        let start = std::time::Instant::now();
        let fetched = ensure_registry_requirements(&reqs, Some(&default), &BTreeMap::new(), &[])
            .unwrap();
        eprintln!("multipackage fetch of 17 packages took {:?}", start.elapsed());
        assert_eq!(fetched.len(), 17);
        for (full, version, _) in &meta {
            let found = fetched.iter().find(|f| &f.name == full).unwrap();
            assert_eq!(&found.version, version);
            let text = std::fs::read_to_string(found.dir.join("Project.config")).unwrap();
            assert!(text.contains(full), "{text}");
            assert!(text.contains(version), "{text}");
            let main = std::fs::read(found.dir.join("src").join("main.rnx")).unwrap();
            assert_eq!(main, b"export fn hello(): Int { return 1; }\n");
        }
        let have = scan_cache_have();
        assert_eq!(have.len(), 17);
        for (full, version, sha) in &meta {
            assert!(
                have.iter().any(|h| &h.full == full && &h.version == version && &h.integrity == sha),
                "{full}"
            );
        }
        assert_eq!(server.count_get("/chunks"), 17);
        assert_eq!(chunk_body_hashes(&server).len(), 18);
    }

    #[test]
    fn multipackage_reports_sorted_first_error_naming_its_package() {
        let (_guard, _dir) = isolate_cache("multierror");
        let mut pkgs = Vec::new();
        let mut nodes = Vec::new();
        for i in 0..5u32 {
            let full = format!("test/pkg{i:02}");
            let (mut pkg, sha) = routed_fixture(&full, "1.0.0", 0);
            if i == 1 || i == 3 {
                let bad_hash = Sha256::hexdigest(format!("{full}-chunk").as_bytes());
                let manifest = format!(
                    "{{\"entries\": [{{\"name\": \"Project.config\", \"size\": 8, \"dir\": false, \"chunks\": [\"{bad_hash}\"]}}]}}"
                );
                pkg.manifest_override = Some(manifest);
                pkg.chunk_override = Some(vec![(bad_hash, b"corrupt!".to_vec())]);
                nodes.push(node_json(&full, "1.0.0", &"0".repeat(64), false));
            } else {
                nodes.push(node_json(&full, "1.0.0", &sha, false));
            }
            pkgs.push(pkg);
        }
        let server = MockRegistry::start_routed(RoutedConfig {
            version_spec: 0,
            resolve_body: resolve_json_static(&nodes),
            packages: pkgs,
        });
        let default = registry_cfg(&server.base);
        let reqs: BTreeMap<String, String> = (0..5u32)
            .map(|i| (format!("test/pkg{i:02}"), "1.0.0".to_string()))
            .collect();
        let err =
            ensure_registry_requirements(&reqs, Some(&default), &BTreeMap::new(), &[]).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("test/pkg01@1.0.0"), "{}", err.message);
        let _ = server;
    }

#[test]
    fn inline_entries_skip_the_chunk_manifest_get() {
        let (_guard, _dir) = isolate_cache("inline");
        let (manifest, pairs, digest, _pc, _main) = fixture_tree("@acme/widget", "1.2.0");
        let node = node_json_inline("@acme/widget", "1.2.0", &digest, &manifest);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones", RESOLVE_INLINE_CHUNKS],
            resolve_status: 200,
            resolve_body: resolve_json_chunks(&[node], Some("inline")),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(pairs),
        });
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        // No `/chunks` document was fetched: the resolve body carried it.
        assert_eq!(server.count_get("/chunks"), 0);
        let main = std::fs::read_to_string(fetched[0].dir.join("src").join("main.rnx")).unwrap();
        assert!(main.contains("hello"), "{main}");
        // The request advertises the opt-in so the server knows to inline.
        let bodies = server.resolve_bodies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(bodies[0].get("chunks").and_then(|v| v.as_bool()), Some(true));
    }

    #[test]
    fn no_capability_means_no_inline_opt_in() {
        let (_guard, _dir) = isolate_cache("nocap");
        let (manifest, pairs, digest, _pc, _main) = fixture_tree("@acme/widget", "1.2.0");
        // The node carries entries, but discovery never advertised the
        // capability, so the client must ignore them and ask for the document.
        let node = node_json_inline("@acme/widget", "1.2.0", &digest, &manifest);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(server.count_get("/chunks"), 1);
        let bodies = server.resolve_bodies();
        assert_eq!(bodies[0].get("chunks"), None);
    }

    #[test]
    fn links_switch_falls_back_to_the_chunk_manifest_get() {
        let (_guard, _dir) = isolate_cache("links");
        let (manifest, pairs, digest, _pc, _main) = fixture_tree("@acme/widget", "1.2.0");
        // The server overran its inline cap and said "links": the node has no
        // entries at all and the client must read the document.
        let node = node_json("@acme/widget", "1.2.0", &digest, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones", RESOLVE_INLINE_CHUNKS],
            resolve_status: 200,
            resolve_body: resolve_json_chunks(&[node], Some("links")),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(pairs),
        });
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(server.count_get("/chunks"), 1);
        let main = std::fs::read_to_string(fetched[0].dir.join("src").join("main.rnx")).unwrap();
        assert!(main.contains("hello"), "{main}");
    }

    #[test]
    fn inline_entries_that_fail_verification_retry_the_manifest() {
        let (_guard, _dir) = isolate_cache("inlinegen");
        let (manifest, pairs, digest, _pc, _main) = fixture_tree("@acme/widget", "1.2.0");
        // A stale generation: the inlined list names bytes that are not the
        // manifest, so verification must fail and the guarded retry re-read
        // the chunk manifest origin-direct.
        let stale = manifest.replace("src/main.rnx", "src/stale.rnx");
        let node = node_json_inline("@acme/widget", "1.2.0", &digest, &stale);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones", RESOLVE_INLINE_CHUNKS],
            resolve_status: 200,
            resolve_body: resolve_json_chunks(&[node], Some("inline")),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some(pairs),
        });
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(server.count_get("/chunks"), 1, "retry must re-read the manifest");
        let main = std::fs::read_to_string(fetched[0].dir.join("src").join("main.rnx")).unwrap();
        assert!(main.contains("hello"), "{main}");
    }

    #[test]
    fn inline_entries_never_install_wrong_bytes() {
        let (_guard, _dir) = isolate_cache("inlinewrong");
        let (manifest, mut pairs, digest, _pc, _main) = fixture_tree("@acme/widget", "1.2.0");
        // integrity and the inlined list both name the good manifest, but the
        // chunk route serves different bytes: the digest check must abort.
        let node = node_json_inline("@acme/widget", "1.2.0", &digest, &manifest);
        let evil = b"not the package".to_vec();
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones", RESOLVE_INLINE_CHUNKS],
            resolve_status: 200,
            resolve_body: resolve_json_chunks(&[node], Some("inline")),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: Some(manifest),
            chunk_override: Some({
                for pair in pairs.iter_mut() {
                    if pair.0.len() == 64 {
                        pair.1 = evil.clone();
                        break;
                    }
                }
                pairs
            }),
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert!(
            err.message.contains("checksum mismatch") || err.message.contains("integrity check"),
            "{}",
            err.message
        );
        assert!(scan_cache_have().is_empty());
    }

    #[test]
    fn parse_node_ignores_entries_when_absent() {
        let body = br#"{"resolved": {"a": "1.0.0"}, "base": "", "levels": [[{"full": "a", "version": "1.0.0", "path": "a@1.0.0/chunks", "integrity": "x", "engineRange": "", "deps": [], "yanked": false}]]}"#;
        let out = parse_resolve(body).unwrap();
        assert_eq!(out.levels[0][0].entries, None);
    }

    #[test]
    fn parse_node_reads_inline_entries() {
        let body = br#"{"resolved": {"a": "1.0.0"}, "base": "", "levels": [[{"full": "a", "version": "1.0.0", "path": "a@1.0.0/chunks", "integrity": "x", "engineRange": "", "deps": [], "yanked": false, "entries": [{"name": "src/main.rnx", "size": 7, "dir": false, "chunks": ["ab"]}]}]], "chunks": "inline"}"#;
        let out = parse_resolve(body).unwrap();
        let entries = out.levels[0][0].entries.as_ref().unwrap();
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].name, "src/main.rnx");
        assert_eq!(entries[0].size, 7);
        assert!(!entries[0].dir);
        assert_eq!(entries[0].chunks, vec!["ab".to_string()]);
    }

    #[test]
    fn parse_node_rejects_malformed_inline_entries() {
        let body = br#"{"resolved": {"a": "1.0.0"}, "base": "", "levels": [[{"full": "a", "version": "1.0.0", "path": "a@1.0.0/chunks", "integrity": "x", "engineRange": "", "deps": [], "yanked": false, "entries": [{"name": "a", "size": "seven", "dir": false, "chunks": []}]}]]}"#;
        let err = parse_resolve(body).unwrap_err();
        assert!(err.message.contains("chunk entries"), "{}", err.message);
    }

    #[test]
    fn resolve_body_carries_the_inline_flag_only_when_asked() {
        let reqs = requirements_for("a", "*");
        let plain = resolve_request_body(&reqs, &[], false);
        assert!(!plain.contains("chunks"), "{plain}");
        let opted = resolve_request_body(&reqs, &[], true);
        assert!(opted.contains("\"chunks\": true"), "{opted}");
        assert!(parse_json(opted.as_bytes()).is_ok());
        let with_have = resolve_request_body(
            &reqs,
            &[HaveEntry { full: "a".into(), version: "1.0.0".into(), integrity: "s".into() }],
            true,
        );
        let parsed = parse_json(with_have.as_bytes()).unwrap();
        assert_eq!(parsed.get("chunks").and_then(|v| v.as_bool()), Some(true));
        assert_eq!(parsed.get("have").and_then(|v| v.as_array()).map(|a| a.len()), Some(1));
    }

    #[test]
    fn resolve_body_carries_the_toolchain_engine_version() {
        let reqs = requirements_for("a", "*");
        let body = resolve_request_body(&reqs, &[], false);
        let parsed = parse_json(body.as_bytes()).unwrap();
        let engine = parsed.get("engine").and_then(|v| v.as_str()).unwrap_or_default();
        assert_eq!(engine, env!("CARGO_PKG_VERSION"));
        assert!(parse_semver(engine).is_some(), "{engine}");
    }

    #[test]
    fn resolve_request_sends_the_engine_to_the_registry() {
        let (_guard, _dir) = isolate_cache("enginesend");
        let (server, cfg) = tree_server("@acme/widget", "1.2.0");
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&cfg),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        let bodies = server.resolve_bodies();
        assert_eq!(bodies.len(), 1);
        let engine = bodies[0].get("engine").and_then(|v| v.as_str()).unwrap_or_default();
        assert_eq!(engine, env!("CARGO_PKG_VERSION"));
    }

    #[test]
    fn resolve_422_engine_unsatisfiable_is_reported_as_the_registry_explained_it() {
        let (_guard, _dir) = isolate_cache("engine422");
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 422,
            resolve_body: "{\"code\": \"unsatisfiable\", \"message\": \"no version of @acme/widget satisfies ^1.0.0 for engine 0.2.0 (have 1.2.0)\"}"
                .to_string(),
            manifest_status: 200,
            fallback_tarball: Vec::new(),
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let err = resolve_requirements(
            &server.base,
            &requirements_for("@acme/widget", "^1.0.0"),
            &[],
            false,
        )
        .unwrap_err();
        assert!(
            err.message.contains("no version of @acme/widget satisfies ^1.0.0 for engine 0.2.0"),
            "{}",
            err.message
        );
        assert!(
            !err.message.contains("does not understand"),
            "{}",
            err.message
        );
    }

    #[test]
    fn multipackage_cache_hits_issue_zero_chunk_requests() {
        let (_guard, _dir) = isolate_cache("multihit");
        let (server, meta) = routed_registry_17(20);
        let default = registry_cfg(&server.base);
        let reqs: BTreeMap<String, String> =
            meta.iter().map(|(f, _, _)| (f.clone(), "1.0.0".to_string())).collect();
        let first =
            ensure_registry_requirements(&reqs, Some(&default), &BTreeMap::new(), &[]).unwrap();
        assert_eq!(first.len(), 17);
        let before = chunk_route_gets(&server);
        assert_eq!(chunk_body_hashes(&server).len(), 18);
        let have = scan_cache_have();
        assert_eq!(have.len(), 17);
        let second =
            ensure_registry_requirements(&reqs, Some(&default), &BTreeMap::new(), &have).unwrap();
        assert_eq!(second.len(), 17);
        assert_eq!(chunk_route_gets(&server), before);
        for f in &second {
            let text = std::fs::read_to_string(f.dir.join("Project.config")).unwrap();
            assert!(text.contains(&f.name), "{text}");
        }
    }

    #[test]
    fn std_transport_reuses_keepalive_connections() {
        use std::io::{Read, Write};
        use std::net::TcpListener;
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let base = format!("http://{}", listener.local_addr().unwrap());
        let stats = std::sync::Arc::new(std::sync::Mutex::new((0usize, 0usize)));
        let seen = stats.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let seen = seen.clone();
                std::thread::spawn(move || {
                    seen.lock().unwrap_or_else(|e| e.into_inner()).0 += 1;
                    loop {
                        let mut head = Vec::new();
                        let mut byte = [0u8; 1];
                        let mut closed = false;
                        while !head.ends_with(b"\r\n\r\n") {
                            match stream.read(&mut byte) {
                                Ok(0) => {
                                    closed = true;
                                    break;
                                }
                                Ok(_) => head.push(byte[0]),
                                Err(_) => {
                                    closed = true;
                                    break;
                                }
                            }
                            if head.len() > 65536 {
                                closed = true;
                                break;
                            }
                        }
                        if closed || head.is_empty() {
                            return;
                        }
                        let text = String::from_utf8_lossy(&head).into_owned();
                        let mut lines = text.lines();
                        let request = lines.next().unwrap_or_default().to_string();
                        let mut length = 0usize;
                        for line in lines {
                            if let Some(rest) =
                                line.strip_prefix("Content-Length:").or_else(|| line.strip_prefix("content-length:"))
                            {
                                length = rest.trim().parse::<usize>().unwrap_or(0);
                            }
                        }
                        let mut body = vec![0u8; length];
                        let mut read = 0usize;
                        while read < body.len() {
                            match stream.read(&mut body[read..]) {
                                Ok(0) => return,
                                Ok(n) => read += n,
                                Err(_) => return,
                            }
                        }
                        seen.lock().unwrap_or_else(|e| e.into_inner()).1 += 1;
                        let is_resolve = request.contains("POST");
                        let payload = if is_resolve { b"{}".to_vec() } else { b"{\"spec\": 1}".to_vec() };
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: keep-alive\r\n\r\n",
                            payload.len()
                        );
                        if stream.write_all(head.as_bytes()).is_err() {
                            return;
                        }
                        if stream.write_all(&payload).is_err() {
                            return;
                        }
                        if stream.flush().is_err() {
                            return;
                        }
                    }
                });
            }
        });
        let reply = http_get(&format!("{base}/api/version"), "probe").unwrap();
        assert_eq!(reply.status, 200);
        let reply = http_get(&format!("{base}/api/version"), "probe").unwrap();
        assert_eq!(reply.status, 200);
        let (conns, reqs) = *stats.lock().unwrap_or_else(|e| e.into_inner());
        assert_eq!(reqs, 2);
        assert_eq!(conns, 1);
    }

    const INTEROP_PC: &[u8] = b"export default {\n    project: {\n        name: \"test/interop\",\n        version: \"1.0.0\"\n    }\n}\n";
    const INTEROP_MAIN: &[u8] = b"export fn hello(): Int { return 1; }\n";
    const INTEROP_MANIFEST_GZ: &[u8] = &[
        31, 139, 8, 0, 0, 0, 0, 0, 2, 255, 125, 142, 193, 78, 3, 49, 12, 68, 127, 101, 149,
        115, 213, 58, 137, 119, 155, 240, 21, 220, 81, 15, 182, 99, 211, 20, 154, 74, 217, 34,
        33, 170, 254, 59, 129, 11, 61, 113, 155, 167, 145, 102, 222, 205, 105, 187, 246, 170,
        171, 123, 154, 94, 110, 174, 209, 89, 71, 114, 207, 253, 114, 82, 185, 110, 229, 210,
        172, 190, 186, 205, 228, 214, 250, 245, 211, 120, 152, 7, 148, 218, 71, 54, 122, 95,
        117, 144, 28, 63, 218, 219, 239, 128, 43, 128, 166, 70, 0, 144, 67, 33, 13, 172, 132,
        76, 201, 207, 230, 197, 22, 70, 89, 152, 80, 195, 30, 22, 46, 193, 71, 207, 130, 26,
        163, 80, 64, 159, 51, 152, 184, 195, 125, 51, 253, 105, 172, 93, 118, 103, 170, 109,
        219, 219, 231, 131, 68, 220, 255, 231, 48, 4, 210, 120, 47, 140, 196, 179, 47, 201,
        67, 176, 37, 8, 103, 34, 140, 37, 3, 230, 228, 81, 130, 88, 20, 76, 41, 51, 229, 65,
        49, 89, 226, 57, 49, 102, 24, 14, 135, 251, 55, 213, 37, 5, 127, 24, 1, 0, 0,
    ];

    #[test]
    fn manifest_body_decode_absent_means_identity_bytes() {
        let raw = br#"{"entries": []}"#;
        assert_eq!(decode_chunk_manifest_body(raw, None, "pkg", "1.0.0").unwrap(), raw);
        assert_eq!(
            decode_chunk_manifest_body(raw, Some("identity"), "pkg", "1.0.0").unwrap(),
            raw
        );
        assert_eq!(
            decode_chunk_manifest_body(raw, Some("Identity"), "pkg", "1.0.0").unwrap(),
            raw
        );
    }

    #[test]
    fn manifest_body_decode_gzip_roundtrip() {
        let raw = br#"{"entries": [{"name": "a.rnx", "size": 3, "dir": false, "chunks": ["ab"]}]}"#;
        let gz = crate::gzip::compress_gzip(raw);
        assert_eq!(
            decode_chunk_manifest_body(&gz, Some("gzip"), "pkg", "1.0.0").unwrap(),
            raw
        );
        assert_eq!(
            decode_chunk_manifest_body(&gz, Some("GZIP"), "pkg", "1.0.0").unwrap(),
            raw
        );
    }

    #[test]
    fn manifest_body_decode_rejects_corrupt_and_unknown() {
        let bad =
            decode_chunk_manifest_body(b"not gzip bytes at all!!!!", Some("gzip"), "pkg", "9.9.9")
                .unwrap_err();
        assert_eq!(bad.code, Code::E108);
        assert!(bad.message.contains("cannot be decoded"), "{}", bad.message);
        assert!(bad.message.contains("pkg@9.9.9"), "{}", bad.message);
        let gz = crate::gzip::compress_gzip(br#"{"entries": []}"#);
        let mut cut = gz.clone();
        cut.truncate(cut.len() / 2);
        assert!(decode_chunk_manifest_body(&cut, Some("gzip"), "pkg", "1.0.0").is_err());
        for enc in ["zstd", "br", "gzip, br", "compress"] {
            let err =
                decode_chunk_manifest_body(br#"{"entries": []}"#, Some(enc), "pkg", "1.0.0")
                    .unwrap_err();
            assert_eq!(err.code, Code::E108);
            assert!(err.message.contains("unsupported content encoding"), "{}", err.message);
            assert!(err.message.contains(enc), "{}", err.message);
        }
    }

    fn interop_entries() -> (Vec<ChunkEntry>, BTreeMap<String, Vec<u8>>) {
        let stored = crate::project::redact::redact_project_version(
            std::str::from_utf8(INTEROP_PC).unwrap(),
        )
        .unwrap()
        .into_bytes();
        let pc_hash = Sha256::hexdigest(&stored);
        let stored_len = stored.len() as u64;
        let main_hash = Sha256::hexdigest(INTEROP_MAIN);
        let entries = vec![
            ChunkEntry {
                name: "Project.config".to_string(),
                size: stored_len,
                dir: false,
                chunks: vec![pc_hash.clone()],
            },
            ChunkEntry {
                name: "src/main.rnx".to_string(),
                size: INTEROP_MAIN.len() as u64,
                dir: false,
                chunks: vec![main_hash.clone()],
            },
        ];
        let mut by_hash = BTreeMap::new();
        by_hash.insert(pc_hash, stored);
        by_hash.insert(main_hash, INTEROP_MAIN.to_vec());
        (entries, by_hash)
    }

    fn interop_manifest_json(entries: &[ChunkEntry]) -> Vec<u8> {
        let mut out = String::from("{\"entries\": [");
        for (i, e) in entries.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&format!(
                "{{\"name\": \"{}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{}\"]}}",
                e.name, e.size, e.chunks[0]
            ));
        }
        out.push_str("]}");
        out.into_bytes()
    }

    fn start_interop(
        manifest_body: Vec<u8>,
        encoding: Option<&'static str>,
        node: &str,
        bodies: Vec<(String, Vec<u8>)>,
    ) -> MockRegistry {
        MockRegistry::start_encoded(
            MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: resolve_json_static(&[node.to_string()]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: None,
                chunk_override: Some(bodies),
            },
            manifest_body,
            encoding,
        )
    }

    fn assert_accept_encoding_gzip(server: &MockRegistry) {
        let mut manifests = 0;
        for (method, path, headers, _) in server.requests_with_headers() {
            let asks_gzip = headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("accept-encoding") && v.trim().eq_ignore_ascii_case("gzip")
            });
            let route = path.split('?').next().unwrap_or_default();
            if method == "GET" && route.ends_with("/chunks") {
                assert!(asks_gzip, "the chunk manifest read must ask for gzip: {headers:?}");
                manifests += 1;
            } else {
                assert!(
                    !asks_gzip,
                    "only the chunk manifest read asks for gzip: {method} {path} {headers:?}"
                );
            }
        }
        assert!(manifests > 0, "no chunk manifest read was made");
    }

    #[test]
    fn manifest_gzip_end_to_end_with_system_gzip_bytes() {
        let (_guard, _dir) = isolate_cache("mzgzip");
        let (entries, by_hash) = interop_entries();
        let node = node_json(
            "test/interop",
            "1.0.0",
            &manifest_digest_of(&String::from_utf8_lossy(&interop_manifest_json(&entries))),
            false,
        );
        let bodies: Vec<(String, Vec<u8>)> = by_hash.into_iter().collect();
        let server = start_interop(INTEROP_MANIFEST_GZ.to_vec(), Some("gzip"), &node, bodies);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(std::fs::read(fetched[0].dir.join("Project.config")).unwrap(), INTEROP_PC);
        assert_eq!(
            std::fs::read(fetched[0].dir.join("src").join("main.rnx")).unwrap(),
            INTEROP_MAIN
        );
        assert_accept_encoding_gzip(&server);
    }

    #[test]
    fn manifest_gzip_end_to_end_with_intree_bytes() {
        let (_guard, _dir) = isolate_cache("mzintree");
        let (entries, by_hash) = interop_entries();
        let node = node_json(
            "test/interop",
            "1.0.0",
            &manifest_digest_of(&String::from_utf8_lossy(&interop_manifest_json(&entries))),
            false,
        );
        let manifest = interop_manifest_json(&entries);
        let gz = crate::gzip::compress_gzip(&manifest);
        let bodies: Vec<(String, Vec<u8>)> = by_hash.into_iter().collect();
        let server = start_interop(gz, Some("gzip"), &node, bodies);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(std::fs::read(fetched[0].dir.join("Project.config")).unwrap(), INTEROP_PC);
        assert_eq!(
            std::fs::read(fetched[0].dir.join("src").join("main.rnx")).unwrap(),
            INTEROP_MAIN
        );
        assert_accept_encoding_gzip(&server);
    }

    #[test]
    fn manifest_identity_through_manifest_path_asks_for_gzip() {
        let (_guard, _dir) = isolate_cache("mzident");
        let (entries, by_hash) = interop_entries();
        let node = node_json(
            "test/interop",
            "1.0.0",
            &manifest_digest_of(&String::from_utf8_lossy(&interop_manifest_json(&entries))),
            false,
        );
        let manifest = interop_manifest_json(&entries);
        let bodies: Vec<(String, Vec<u8>)> = by_hash.into_iter().collect();
        let server = start_interop(manifest, None, &node, bodies);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(std::fs::read(fetched[0].dir.join("Project.config")).unwrap(), INTEROP_PC);
        assert_accept_encoding_gzip(&server);
    }

    #[test]
    fn manifest_gzip_corrupt_fails_loud_without_cache() {
        let (_guard, _dir) = isolate_cache("mzcorrupt");
        let node = node_json("test/interop", "1.0.0", &"0".repeat(64), false);
        let server = start_interop(
            b"definitely not gzip bytes!!!!".to_vec(),
            Some("gzip"),
            &node,
            Vec::new(),
        );
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("cannot be decoded"), "{}", err.message);
        assert!(err.message.contains("test/interop@1.0.0"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
        let mut cut = INTEROP_MANIFEST_GZ.to_vec();
        cut.truncate(cut.len() / 2);
        let server = start_interop(cut, Some("gzip"), &node, Vec::new());
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("cannot be decoded"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
    }

    #[test]
    fn manifest_unknown_encoding_fails_closed() {
        let (_guard, _dir) = isolate_cache("mzunknown");
        let (entries, by_hash) = interop_entries();
        let manifest = interop_manifest_json(&entries);
        let node = node_json("test/interop", "1.0.0", &"0".repeat(64), false);
        let bodies: Vec<(String, Vec<u8>)> = by_hash.into_iter().collect();
        let server = start_interop(manifest, Some("zstd"), &node, bodies);
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("unsupported content encoding"), "{}", err.message);
        assert!(err.message.contains("`zstd`"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
        let chunk_gets = server
            .requests()
            .iter()
            .filter(|(m, p, _)| m == "GET" && p.split('?').next().unwrap_or_default().contains("/chunk/"))
            .count();
        assert_eq!(chunk_gets, 0);
    }

    // The accepted-name rule, as the client applies it. The server applies
    // the same rule to the same names; see
    // `website/src/lib/server/chunks.ts::checkManifestEntryName`.
    const UNSAFE_NAMES: &[&str] =
        &["../../etc/passwd", "..\\..\\evil", "C:\\evil", "/abs/path", "a/../../b"];

    #[test]
    fn tar_paths_refuse_every_escaping_name() {
        for name in UNSAFE_NAMES {
            let err = tar_safe_rel(name).unwrap_err();
            assert_eq!(err.code, Code::E108, "{name}");
            assert!(err.message.contains("unsafe tar path"), "{}", err.message);
        }
        for name in ["a.txt", "src/main.rnx", "src/", "a/b/c.txt", "Project.config"] {
            assert!(tar_safe_rel(name).is_ok(), "{name}");
        }
    }

    fn manifest_with_name(name: &str, hash: &str, size: usize) -> Vec<u8> {
        let escaped = name.replace('\\', "\\\\");
        format!(
            "{{\"entries\": [{{\"name\": \"{escaped}\", \"size\": {size}, \"dir\": false, \"chunks\": [\"{hash}\"]}}]}}"
        )
        .into_bytes()
    }

    #[test]
    fn manifest_with_an_escaping_name_is_refused() {
        for unsafe_name in UNSAFE_NAMES {
            let (_guard, _dir) = isolate_cache("mzescape");
            let hash = Sha256::hexdigest(INTEROP_PC);
            let manifest = manifest_with_name(unsafe_name, &hash, INTEROP_PC.len());
            let node = node_json(
                "test/interop",
                "1.0.0",
                &manifest_digest_of(&String::from_utf8_lossy(&manifest)),
                false,
            );
            let bodies = vec![(hash, INTEROP_PC.to_vec())];
            let server = start_interop(manifest, None, &node, bodies);
            let default = registry_cfg(&server.base);
            let err = ensure_registry_requirements(
                &requirements_for("test/interop", "1.0.0"),
                Some(&default),
                &BTreeMap::new(),
                &[],
            )
            .unwrap_err();
            assert_eq!(err.code, Code::E108, "{unsafe_name}");
            assert!(err.message.contains("unsafe file path"), "{}", err.message);
            assert!(err.message.contains(unsafe_name), "{}", err.message);
            // Nothing was written: the name is refused before the tree is
            // assembled.
            assert!(scan_cache_have().is_empty(), "{unsafe_name}");
        }
    }

    #[test]
    fn a_version_with_no_file_index_is_not_a_checksum_error() {
        let (_guard, _dir) = isolate_cache("mznoindex");
        let node = node_json("test/interop", "1.0.0", &"0".repeat(64), false);
        // The registry answers 200 with an empty file index.
        let server = start_interop(b"{\"entries\": []}".to_vec(), None, &node, Vec::new());
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("ships no file index"), "{}", err.message);
        assert!(!err.message.contains("checksum mismatch"), "{}", err.message);
    }

    #[test]
    fn a_version_with_no_file_index_is_named_by_the_registry() {
        let (_guard, _dir) = isolate_cache("mznoindex404");
        let node = node_json("test/interop", "1.0.0", &"0".repeat(64), false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 0,
            capabilities: vec!["tombstones"],
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 404,
            fallback_tarball: Vec::new(),
            manifest_error:
                "{\"code\":\"no-file-index\",\"message\":\"package test/interop@1.0.0 ships no file index\"}"
                    .to_string(),
            manifest_override: None,
            chunk_override: None,
        });
        let default = registry_cfg(&server.base);
        let err = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("ships no file index"), "{}", err.message);
        assert!(!err.message.contains("does not exist"), "{}", err.message);
        assert!(scan_cache_have().is_empty());
    }

    #[test]
    fn a_chunk_digest_in_upper_case_is_accepted() {
        let (_guard, _dir) = isolate_cache("mzupper");
        let (entries, by_hash) = interop_entries();
        // The manifest carries the digest in upper case, and the registry
        // answers the chunk under that spelling.
        let upper: Vec<(String, Vec<u8>)> =
            by_hash.into_iter().map(|(h, b)| (h.to_ascii_uppercase(), b)).collect();
        let mut rendered = String::from("{\"entries\": [");
        for (i, (h, _)) in upper.iter().enumerate() {
            if i > 0 {
                rendered.push_str(", ");
            }
            rendered.push_str(&format!(
                "{{\"name\": \"{}\", \"size\": {}, \"dir\": false, \"chunks\": [\"{}\"]}}",
                entries[i].name,
                entries[i].size,
                h
            ));
        }
        rendered.push_str("]}");
        let node = node_json("test/interop", "1.0.0", &manifest_digest_of(&rendered), false);
        let server = start_interop(rendered.into_bytes(), None, &node, upper);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("test/interop", "1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        assert_eq!(std::fs::read(fetched[0].dir.join("Project.config")).unwrap(), INTEROP_PC);
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn the_http2_transport_speaks_the_registry_protocol() {
        let node = node_json("test/interop", "1.0.0", &"0".repeat(64), false);
        let server = MockRegistry::start_encoded(
            MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: resolve_json_static(&[node]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: None,
                chunk_override: None,
            },
            crate::gzip::compress_gzip(br#"{"entries": []}"#),
            Some("gzip"),
        );
        let transport = H2Transport::new().unwrap();

        let version = transport.get(&format!("{}/api/version", server.base)).unwrap();
        assert_eq!(version.status, 200);
        assert_eq!(version.spec_header.as_deref(), Some("0"));
        assert!(String::from_utf8_lossy(&version.body).contains("\"spec\": 0"));

        let resolved = transport.post_json(&format!("{}/api/v0/resolve", server.base), "{}").unwrap();
        assert_eq!(resolved.status, 200);

        let manifest = transport
            .manifest_get(&format!("{}/api/v0/packages/test/interop@1.0.0/chunks", server.base))
            .unwrap();
        assert_eq!(manifest.reply.status, 200);
        assert_eq!(manifest.content_encoding.as_deref(), Some("gzip"));
        let decoded = decode_chunk_manifest_body(
            &manifest.reply.body,
            manifest.content_encoding.as_deref(),
            "test/interop",
            "1.0.0",
        )
        .unwrap();
        assert_eq!(decoded, br#"{"entries": []}"#);

        let mut manifests = 0;
        for (method, path, headers, _) in server.requests_with_headers() {
            let asks_gzip = headers.iter().any(|(k, v)| {
                k.eq_ignore_ascii_case("accept-encoding") && v.trim().eq_ignore_ascii_case("gzip")
            });
            let route = path.split('?').next().unwrap_or_default();
            if method == "GET" && route.ends_with("/chunks") {
                assert!(asks_gzip, "the chunk manifest read must ask for gzip: {headers:?}");
                manifests += 1;
            } else {
                assert!(
                    !asks_gzip,
                    "only the chunk manifest read asks for gzip: {method} {path} {headers:?}"
                );
            }
        }
        assert_eq!(manifests, 1);
    }

    #[test]
    fn artifact_pin_installs_blobs_and_locks_the_digest() {
        use crate::oci::testkit::start_origin;
        let (_guard, _dir) = isolate_cache("artifact-pin");
        let blob = b"fake native bytes for lock test".to_vec();
        let blob_hash = Sha256::hexdigest(&blob);
        let manifest_doc = format!(
            "{{\"schemaVersion\": 2, \"config\": {{\"digest\": \"sha256:{}\"}}, \"layers\": [{{\"digest\": \"sha256:{blob_hash}\"}}]}}",
            "cc".repeat(32)
        );
        let manifest_hash = Sha256::hexdigest(manifest_doc.as_bytes());
        let index = artifact_index_for(&manifest_hash, &runtime_annotations());
        let index_digest = format!("sha256:{}", Sha256::hexdigest(&index));
        let mut files = BTreeMap::new();
        files.insert(format!("sha256:{manifest_hash}"), manifest_doc.into_bytes());
        files.insert(format!("sha256:{blob_hash}"), blob.clone());
        let origin = start_origin(index, files);
        let (manifest, pairs, digest) = artifact_fixture_tree("@acme/widget", "1.2.0", &origin.bare);
        let node = node_artifact_json("@acme/widget", "1.2.0", &digest, &index_digest);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        assert_eq!(fetched.len(), 1);
        let installed = std::fs::read(fetched[0].dir.join(".rnx-cache").join("artifacts").join("runtime").join("libnative.so")).unwrap();
        assert_eq!(installed, blob);
        assert_eq!(cached_artifact_digest(&fetched[0].dir).as_deref(), Some(index_digest.as_str()));
        let dir = crate::scratch::dir("artifact-lock");
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    entries: {{ main: \"src/main.rnx\" }},\n    dependencies: {{\n        \"@acme/widget\": {{ version: \"^1.0.0\" }}\n    }},\n    registry: {{ url: \"{}\" }}\n}}\n",
                server.base
            ),
        )
        .unwrap();
        std::fs::write(root.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        let cfg = crate::project::ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = crate::deplock::ProjectDepLock::resolve(&root, &cfg).unwrap();
        let widget = lock.packages.iter().find(|p| p.name == "@acme/widget").unwrap();
        assert_eq!(widget.artifact_digest.as_deref(), Some(index_digest.as_str()));
        crate::deplock::verify_lock(&root, &cfg, &lock).unwrap();
        let mut tampered = lock.clone();
        tampered
            .packages
            .iter_mut()
            .find(|p| p.name == "@acme/widget")
            .unwrap()
            .artifact_digest = Some(format!("sha256:{}", "ee".repeat(32)));
        let err = crate::deplock::verify_lock(&root, &cfg, &tampered).unwrap_err();
        assert!(err.message.contains("artifact mismatch"), "{}", err.message);
    }

    #[test]
    fn yanked_artifact_versions_warn_and_still_fetch() {
        use crate::oci::testkit::start_origin;
        let (_guard, _dir) = isolate_cache("artifact-yanked");
        let blob = b"fake native bytes for yanked test".to_vec();
        let blob_hash = Sha256::hexdigest(&blob);
        let manifest_doc = format!(
            "{{\"schemaVersion\": 2, \"config\": {{\"digest\": \"sha256:{}\"}}, \"layers\": [{{\"digest\": \"sha256:{blob_hash}\"}}]}}",
            "cc".repeat(32)
        );
        let manifest_hash = Sha256::hexdigest(manifest_doc.as_bytes());
        let index = artifact_index_for(&manifest_hash, &runtime_annotations());
        let index_digest = format!("sha256:{}", Sha256::hexdigest(&index));
        let mut files = BTreeMap::new();
        files.insert(format!("sha256:{manifest_hash}"), manifest_doc.into_bytes());
        files.insert(format!("sha256:{blob_hash}"), blob.clone());
        let origin = start_origin(index, files);
        let (manifest, pairs, digest) = artifact_fixture_tree("@acme/widget", "1.2.0", &origin.bare);
        let node = format!(
            "{{\"full\": \"@acme/widget\", \"version\": \"1.2.0\", \"path\": \"@acme/widget@1.2.0/chunks\", \"integrity\": \"{digest}\", \"engineRange\": \"\", \"deps\": [], \"yanked\": true, \"artifactDigest\": \"{index_digest}\"}}"
        );
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        let installed =
            std::fs::read(fetched[0].dir.join(".rnx-cache").join("artifacts").join("runtime").join("libnative.so")).unwrap();
        assert_eq!(installed, blob);
    }

    #[test]
    fn artifact_marker_records_pin_digest() {
        use crate::oci::testkit::start_origin;
        let (_guard, _dir) = isolate_cache("artifact-marker");
        let blob = b"fake native bytes for marker test".to_vec();
        let blob_hash = Sha256::hexdigest(&blob);
        let manifest_doc = format!(
            "{{\"schemaVersion\": 2, \"config\": {{\"digest\": \"sha256:{}\"}}, \"layers\": [{{\"digest\": \"sha256:{blob_hash}\"}}]}}",
            "cc".repeat(32)
        );
        let manifest_hash = Sha256::hexdigest(manifest_doc.as_bytes());
        let index = artifact_index_for(&manifest_hash, &runtime_annotations());
        let index_digest = format!("sha256:{}", Sha256::hexdigest(&index));
        let mut files = BTreeMap::new();
        files.insert(format!("sha256:{manifest_hash}"), manifest_doc.into_bytes());
        files.insert(format!("sha256:{blob_hash}"), blob.clone());
        let origin = start_origin(index, files);
        let (manifest, pairs, digest) = artifact_fixture_tree("@acme/widget", "1.2.0", &origin.bare);
        let node = node_artifact_json("@acme/widget", "1.2.0", &digest, &index_digest);
        let server = start_tree(&[node], &manifest, pairs);
        let default = registry_cfg(&server.base);
        let fetched = ensure_registry_requirements(
            &requirements_for("@acme/widget", "^1.0.0"),
            Some(&default),
            &BTreeMap::new(),
            &[],
        )
        .unwrap();
        let marker =
            std::fs::read_to_string(fetched[0].dir.join(".rnx-artifact")).unwrap();
        let mut lines = marker.lines();
        assert_eq!(lines.next().unwrap_or_default(), index_digest.as_str());
        assert_eq!(lines.next(), None);
    }
}
