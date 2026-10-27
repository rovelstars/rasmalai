use diagnostics::{Code, Diagnostic};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum NativePrefer {
    #[default]
    None,
    System,
    Bundled,
}

impl NativePrefer {
    pub fn from_env() -> NativePrefer {
        match std::env::var("RNX_PREFER_NATIVE").as_deref() {
            Ok("system") => NativePrefer::System,
            Ok("bundled") => NativePrefer::Bundled,
            _ => NativePrefer::None,
        }
    }

    pub fn resolve(system_flag: bool, bundled_flag: bool) -> NativePrefer {
        if system_flag {
            return NativePrefer::System;
        }
        if bundled_flag {
            return NativePrefer::Bundled;
        }
        NativePrefer::from_env()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EffectiveNative {
    pub system: bool,
    pub version: Option<String>,
    pub from: Option<String>,
}

pub fn effective_choice(
    declared_system: bool,
    declared_version: Option<&str>,
    declared_from: Option<&str>,
    pin: Option<&crate::project::NativePin>,
    prefer: NativePrefer,
) -> EffectiveNative {
    let pin_system = pin.map(|p| p.system);
    let pin_version = pin.and_then(|p| p.version.as_deref());
    let pin_from = pin.and_then(|p| p.from.as_deref());
    let version = pin_version.or(declared_version);
    let from = pin_from.or(declared_from);
    match (pin_system, prefer) {
        (Some(system), _) => EffectiveNative { system, version: version.map(str::to_string), from: if system { None } else { from.map(str::to_string) } },
        (None, NativePrefer::System) => EffectiveNative { system: true, version: version.map(str::to_string), from: None },
        (None, NativePrefer::Bundled) => EffectiveNative { system: false, version: version.map(str::to_string), from: from.map(str::to_string) },
        (None, NativePrefer::None) => EffectiveNative {
            system: declared_system,
            version: version.map(str::to_string),
            from: from.map(str::to_string),
        },
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NativeLink {
    pub lib: String,
    pub archive: Option<std::path::PathBuf>,
}

struct NativeDecl {
    dep: String,
    lib: String,
    system: bool,
    title: Option<String>,
    owner: std::path::PathBuf,
    owner_pkg: String,
}

fn lock_registry_versions(root: &std::path::Path) -> std::collections::BTreeMap<String, String> {
    let mut out = std::collections::BTreeMap::new();
    let lock = crate::deplock::ProjectDepLock::load(root).ok().flatten();
    for pkg in lock.iter().flat_map(|l| l.packages.iter()) {
        if let Some(rest) = pkg.source.strip_prefix("registry:")
            && let Some((full, version)) = rest.rsplit_once('@')
        {
            out.entry(full.to_string()).or_insert_with(|| version.to_string());
        }
    }
    out
}

fn exact_requirement_version(req: &str) -> Option<String> {
    let req = req.trim();
    if req.is_empty() || req == "*" || req == "latest" {
        return None;
    }
    let mut parts = req.split('.');
    let ok = parts.by_ref().take(3).all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()));
    if ok && parts.next().is_none() {
        return Some(req.to_string());
    }
    None
}

fn offline_dep_dir(
    pkg_dir: &std::path::Path,
    dep_name: &str,
    spec: &crate::project::DependencySpec,
    locked: &std::collections::BTreeMap<String, String>,
    entry_cfg: &crate::project::ProjectConfig,
) -> Option<std::path::PathBuf> {
    let offline = match spec {
        crate::project::DependencySpec::Path { path, .. } => std::fs::canonicalize(pkg_dir.join(path)).ok(),
        crate::project::DependencySpec::Git { rev, .. } => {
            let anchor = crate::fetch::cache_anchor(pkg_dir);
            let vendor = anchor.join(crate::fetch::VENDOR_DIR).join(dep_name);
            if vendor.join(crate::project::MANIFEST_FILE).is_file() {
                std::fs::canonicalize(&vendor).ok()
            } else {
                let dir = anchor
                    .join(".rnx-cache")
                    .join("cache")
                    .join("git")
                    .join(format!("{dep_name}-{}", crate::fetch::short_rev(rev)));
                std::fs::canonicalize(&dir).ok()
            }
        }
        crate::project::DependencySpec::Semver { version, .. } => {
            let pinned = locked.get(dep_name).cloned().or_else(|| exact_requirement_version(version));
            let pinned = pinned?;
            let base = crate::fetch::registry_base_for(
                dep_name,
                entry_cfg.registry.as_ref(),
                &entry_cfg.registries,
            );
            let dir = crate::fetch::find_cached_package_dir(&base, dep_name, &pinned)?;
            std::fs::canonicalize(&dir).ok()
        }
        crate::project::DependencySpec::Url { .. } | crate::project::DependencySpec::Native { .. } => None,
    };
    if offline.is_some() {
        return offline;
    }
    match spec {
        crate::project::DependencySpec::Git { git, rev, .. } => {
            crate::fetch::resolve_git_dep(&crate::fetch::cache_anchor(pkg_dir), dep_name, git, rev).ok()
        }
        crate::project::DependencySpec::Semver { version, .. } => crate::fetch::resolve_registry_package(
            dep_name,
            version,
            entry_cfg.registry.as_ref(),
            &entry_cfg.registries,
            locked.get(dep_name).cloned(),
            &[],
            Some(&crate::fetch::cache_anchor(pkg_dir)),
        )
        .ok()
        .map(|(dir, _, _)| dir),
        _ => None,
    }
}

fn find_bundled_blob(owner: &std::path::Path, title: &str) -> Option<std::path::PathBuf> {
    let root = crate::oci::project_artifact_cache(owner);
    let mut stack = vec![root];
    let mut hits: Vec<std::path::PathBuf> = Vec::new();
    while let Some(dir) = stack.pop() {
        let mut entries: Vec<std::path::PathBuf> = Vec::new();
        let Ok(read) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in read.flatten() {
            entries.push(entry.path());
        }
        entries.sort();
        for path in entries {
            if path.is_dir() {
                if path.file_name().map(|n| n == "blobs").unwrap_or(false) {
                    continue;
                }
                stack.push(path);
            } else if path.is_file() && path.file_name().map(|n| n == title).unwrap_or(false) {
                hits.push(path);
            }
        }
    }
    hits.sort();
    hits.into_iter().next()
}

pub fn resolve_native_links(
    scope_root: &std::path::Path,
    prefer: NativePrefer,
    wanted: &[String],
) -> Result<Vec<NativeLink>, Diagnostic> {
    let manifest = crate::project::load_manifest(scope_root)?;
    let Some(manifest) = manifest else {
        return Ok(wanted
            .iter()
            .map(|lib| NativeLink { lib: lib.clone(), archive: None })
            .collect());
    };
    let mut roots: Vec<(std::path::PathBuf, crate::project::ProjectConfig)> = Vec::new();
    let pins: std::collections::BTreeMap<String, crate::project::NativePin> = match &manifest.workspace {
        Some(ws) => {
            let members = crate::project::resolve_workspace_members(scope_root, ws)?;
            let mut names: Vec<&String> = members.keys().collect();
            names.sort();
            for name in names {
                if let Some((dir, cfg)) = members.get(name) {
                    roots.push((dir.clone(), cfg.clone()));
                }
            }
            crate::project::ProjectConfig::load_from_dir(scope_root)
                .ok()
                .flatten()
                .map(|c| c.native_pins)
                .unwrap_or_default()
        }
        None => {
            let cfg = manifest.project.ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("{}: missing [project] section", scope_root.display()),
                )
            })?;
            roots.push((scope_root.to_path_buf(), cfg.clone()));
            cfg.native_pins.clone()
        }
    };
    let locked = lock_registry_versions(scope_root);
    let mut decls: std::collections::BTreeMap<String, NativeDecl> = std::collections::BTreeMap::new();
    let mut seen: std::collections::BTreeSet<std::path::PathBuf> = std::collections::BTreeSet::new();
    let mut stack = roots;
    stack.reverse();
    while let Some((pkg_dir, cfg)) = stack.pop() {
        let Ok(canonical) = std::fs::canonicalize(&pkg_dir) else {
            continue;
        };
        if !seen.insert(canonical.clone()) {
            continue;
        }
        let mut deps: Vec<(&String, &crate::project::DependencySpec)> = cfg.dependencies.iter().collect();
        deps.sort_by(|a, b| a.0.cmp(b.0));
        let mut pending: Vec<(std::path::PathBuf, crate::project::ProjectConfig)> = Vec::new();
        for (dep_name, spec) in deps {
            if let crate::project::DependencySpec::Native { lib, system, version, from, .. } = spec {
                if decls.contains_key(lib) {
                    continue;
                }
                let eff = effective_choice(
                    *system,
                    version.as_deref(),
                    from.as_deref(),
                    pins.get(dep_name),
                    prefer,
                );
                decls.insert(
                    lib.clone(),
                    NativeDecl {
                        dep: dep_name.clone(),
                        lib: lib.clone(),
                        system: eff.system,
                        title: eff.from.clone(),
                        owner: canonical.clone(),
                        owner_pkg: cfg.name.clone(),
                    },
                );
                continue;
            }
            let dir = offline_dep_dir(&canonical, dep_name, spec, &locked, &cfg);
            match dir {
                Some(dir) => match crate::project::ProjectConfig::load_from_dir(&dir)? {
                    Some(dep_cfg) => pending.push((dir, dep_cfg)),
                    None => continue,
                },
                None => continue,
            }
        }
        pending.reverse();
        stack.extend(pending);
    }
    let mut out = Vec::with_capacity(wanted.len());
    for lib in wanted {
        let Some(decl) = decls.get(lib) else {
            out.push(NativeLink { lib: lib.clone(), archive: None });
            continue;
        };
        if decl.system {
            out.push(NativeLink { lib: lib.clone(), archive: None });
            continue;
        }
        let Some(title) = decl.title.as_deref() else {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "native dependency `{}` (`{}`) selects the bundled build but names no blob (declare `from: \"lib{}.a\"` on the dependency in `{}/Project.config` or rebuild with `--prefer-system` for the host library)",
                    decl.dep, decl.lib, decl.lib, decl.owner_pkg,
                ),
            ));
        };
        match find_bundled_blob(&decl.owner, title) {
            Some(path) => out.push(NativeLink { lib: lib.clone(), archive: Some(path) }),
            None => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "bundled blob `{title}` for native `{}` (package `{}`) not found under `{}` (fetch the owning package's `artifacts` image so the blob is staged, then rebuild; or rebuild with `--prefer-system` for the host library)",
                        decl.lib,
                        decl.dep,
                        crate::oci::project_artifact_cache(&decl.owner).display(),
                    ),
                ));
            }
        }
    }
    Ok(out)
}

fn valid_probe_name(lib: &str) -> bool {
    !lib.is_empty()
        && !lib.starts_with('-')
        && lib.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'+' || b == b'-')
}

fn pkg_config_version(lib: &str) -> Option<String> {
    let out = std::process::Command::new("pkg-config")
        .arg("--modversion")
        .arg(lib)
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let version = String::from_utf8_lossy(&out.stdout).trim().to_string();
    if version.is_empty() { None } else { Some(version) }
}

fn ldconfig_soname_version(lib: &str) -> Option<Option<String>> {
    let out = std::process::Command::new("ldconfig").arg("-p").output().ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout);
    let want = format!("lib{lib}.so");
    let mut found = false;
    let mut version: Option<String> = None;
    for line in text.lines() {
        let entry = line.trim();
        let Some(name) = entry.split_whitespace().next() else {
            continue;
        };
        if name == want || name.starts_with(&format!("{want}.")) {
            found = true;
            if let Some(suffix) = name.strip_prefix(&format!("{want}.")) {
                let candidate = suffix.split_whitespace().next().unwrap_or_default();
                if !candidate.is_empty()
                    && candidate.bytes().all(|b| b.is_ascii_digit() || b == b'.')
                    && version.is_none()
                {
                    version = Some(candidate.to_string());
                }
            }
        }
    }
    if found { Some(version) } else { None }
}

pub fn verify_system_lib(lib: &str, constraint: Option<&str>) -> Result<String, Diagnostic> {
    if !valid_probe_name(lib) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("system library `{lib}` is not a valid library name (expected `[A-Za-z0-9._+-]+`, never starting with `-`)"),
        ));
    }
    if let Some(installed) = pkg_config_version(lib) {
        if let Some(range) = constraint {
            if !crate::fetch::engine_satisfied(range, &installed) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("system library `{lib}` reports version `{installed}` which does not satisfy `{range}` (via `pkg-config --modversion {lib}`)"),
                ));
            }
        }
        return Ok(installed);
    }
    match ldconfig_soname_version(lib) {
        Some(maybe_version) => {
            if let Some(range) = constraint {
                match maybe_version {
                    Some(installed) if crate::fetch::engine_satisfied(range, &installed) => Ok(installed),
                    Some(installed) => Err(Diagnostic::new(
                        Code::E108,
                        format!("system library `{lib}` reports version `{installed}` which does not satisfy `{range}` (via `ldconfig -p`)"),
                    )),
                    None => Err(Diagnostic::new(
                        Code::E108,
                        format!("system library `{lib}` is present but its version cannot be verified without `pkg-config`, so constraint `{range}` fails closed"),
                    )),
                }
            } else {
                Ok(maybe_version.unwrap_or_default())
            }
        }
        None => Err(Diagnostic::new(
            Code::E108,
            format!("system library `{lib}` not found (`pkg-config --modversion {lib}` failed and `ldconfig -p` lists no `lib{lib}.so`)"),
        )),
    }
}

pub fn check_owner_version(dep: &str, owner_version: &str, constraint: &str) -> Result<(), Diagnostic> {
    if crate::fetch::engine_satisfied(constraint, owner_version) {
        return Ok(());
    }
    Err(Diagnostic::new(
        Code::E108,
        format!("native dependency `{dep}` pins bundled version `{constraint}` but the declaring package is at `{owner_version}`"),
    ))
}

pub fn triple_target(triple: &str) -> Option<crate::project::eval::ConfigTarget> {
    let parts: Vec<&str> = triple.split('-').collect();
    if parts.len() < 3 || parts.len() > 4 {
        return None;
    }
    if parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    Some(crate::project::eval::ConfigTarget {
        os: parts[2].to_string(),
        arch: parts[0].to_string(),
        env: parts.get(3).unwrap_or(&"").to_string(),
    })
}

pub fn host_triple() -> String {
    let os = std::env::consts::OS;
    let arch = std::env::consts::ARCH;
    let env = std::env::var("RNX_TARGET_ENV").unwrap_or_else(|_| {
        if os == "windows" {
            "msvc".to_string()
        } else if os == "linux" {
            "gnu".to_string()
        } else {
            String::new()
        }
    });
    let vendor = "unknown";
    if env.is_empty() {
        format!("{arch}-{vendor}-{os}")
    } else {
        format!("{arch}-{vendor}-{os}-{env}")
    }
}

#[cfg(test)]
pub(crate) fn test_path_guard() -> std::sync::MutexGuard<'static, ()> {
    static GUARD: std::sync::OnceLock<std::sync::Mutex<()>> = std::sync::OnceLock::new();
    GUARD.get_or_init(|| std::sync::Mutex::new(())).lock().unwrap_or_else(|e| e.into_inner())
}

#[cfg(test)]
mod tests {
    use super::*;

    struct PathOverride {
        prev: Option<String>,
    }

    impl PathOverride {
        fn prepend(dir: &std::path::Path) -> PathOverride {
            let prev = std::env::var("PATH").ok();
            let next = match &prev {
                Some(p) if !p.is_empty() => format!("{}:{}", dir.display(), p),
                _ => dir.display().to_string(),
            };
            unsafe {
                std::env::set_var("PATH", next);
            }
            PathOverride { prev }
        }
    }

    impl Drop for PathOverride {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    fn stub_bin(tag: &str, name: &str, body: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("native-probe-{tag}"));
        let path = dir.join(name);
        std::fs::write(&path, body).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        dir
    }

    #[test]
    fn prefer_flags_win_over_env() {
        assert_eq!(NativePrefer::resolve(true, false), NativePrefer::System);
        assert_eq!(NativePrefer::resolve(false, true), NativePrefer::Bundled);
        assert_eq!(NativePrefer::resolve(false, false), NativePrefer::from_env());
        assert_eq!(NativePrefer::resolve(true, true), NativePrefer::System);
    }

    fn link_tree(tag: &str, deps: &str, extra: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
        let dir = crate::scratch::dir(&format!("native-link-{tag}"));
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }}{extra},\n    dependencies: {{\n{deps}    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(root.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        (dir, root)
    }

    fn link_path_dep(
        tag: &str,
        name: &str,
        deps: &str,
        extra: &str,
        blob: Option<(&str, &[u8])>,
    ) -> (crate::scratch::Scratch, std::path::PathBuf) {
        let dir = crate::scratch::dir(&format!("native-linkdep-{tag}"));
        let pkg = dir.join(name);
        std::fs::create_dir_all(pkg.join("src")).unwrap();
        std::fs::write(
            pkg.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"0.1.0\"\n    }}{extra},\n    dependencies: {{\n{deps}    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(pkg.join("src").join("lib.rnx"), "fn f(): Int { return 1; }\n").unwrap();
        if let Some((title, bytes)) = blob {
            let staged = pkg.join(".rnx-cache").join("artifacts").join("runtime").join(title);
            std::fs::create_dir_all(staged.parent().unwrap()).unwrap();
            std::fs::write(&staged, bytes).unwrap();
        }
        (dir, pkg)
    }

    fn resolve_wanted(root: &std::path::Path, prefer: NativePrefer, wanted: &[&str]) -> Result<Vec<NativeLink>, String> {
        let wanted: Vec<String> = wanted.iter().map(|s| s.to_string()).collect();
        resolve_native_links(root, prefer, &wanted).map_err(|d| d.message.clone())
    }

    #[test]
    fn link_plan_defaults_to_declared_choice() {
        let (_dep, author) = link_path_dep(
            "declared",
            "author",
            "        foo: { native: \"foo\", from: \"libfoo.a\", link: \"static\" }\n",
            ",\n    artifacts: \"example.com/org/author\"\n",
            Some(("libfoo.a", b"archive-bytes")),
        );
        let (_dir, root) = link_tree(
            "declared",
            &format!("        author: {{ path: \"{}\" }}\n", author.display()),
            "",
        );
        let plan = resolve_wanted(&root, NativePrefer::None, &["foo"]).unwrap();
        assert_eq!(plan.len(), 1);
        let archive = plan[0].archive.as_ref().unwrap();
        assert_eq!(std::fs::read(archive).unwrap(), b"archive-bytes");
        let plan = resolve_wanted(&root, NativePrefer::Bundled, &["foo"]).unwrap();
        assert!(plan[0].archive.is_some());
        let plan = resolve_wanted(&root, NativePrefer::System, &["foo"]).unwrap();
        assert_eq!(plan, vec![NativeLink { lib: "foo".to_string(), archive: None }]);
    }

    #[test]
    fn link_plan_undeclared_lib_stays_system() {
        let (_dir, root) = link_tree("undeclared", "", "");
        let plan = resolve_wanted(&root, NativePrefer::Bundled, &["z"]).unwrap();
        assert_eq!(plan, vec![NativeLink { lib: "z".to_string(), archive: None }]);
    }

    #[test]
    fn link_plan_missing_blob_fails_closed() {
        let (_dep, author) = link_path_dep(
            "missing",
            "author",
            "        foo: { native: \"foo\", from: \"libfoo.a\", link: \"static\" }\n",
            ",\n    artifacts: \"example.com/org/author\"\n",
            None,
        );
        let (_dir, root) = link_tree(
            "missing",
            &format!("        author: {{ path: \"{}\" }}\n", author.display()),
            "",
        );
        let err = resolve_wanted(&root, NativePrefer::None, &["foo"]).unwrap_err();
        assert!(err.contains("libfoo.a"), "{err}");
        assert!(err.contains("foo"), "{err}");
        assert!(err.contains("--prefer-system"), "{err}");
    }

    #[test]
    fn link_plan_bundled_without_title_fails_closed() {
        let (_dep, author) = link_path_dep(
            "notitle",
            "author",
            "        z: { native: \"z\", system: true }\n",
            "",
            None,
        );
        let (_dir, root) = link_tree(
            "notitle",
            &format!("        author: {{ path: \"{}\" }}\n", author.display()),
            "",
        );
        let plan = resolve_wanted(&root, NativePrefer::None, &["z"]).unwrap();
        assert_eq!(plan, vec![NativeLink { lib: "z".to_string(), archive: None }]);
        let err = resolve_wanted(&root, NativePrefer::Bundled, &["z"]).unwrap_err();
        assert!(err.contains("names no blob"), "{err}");
        assert!(err.contains("from:"), "{err}");
    }

    #[test]
    fn link_plan_without_manifest_stays_system() {
        let dir = crate::scratch::dir("native-link-nomanifest");
        let plan = resolve_wanted(&dir, NativePrefer::Bundled, &["z"]).unwrap();
        assert_eq!(plan, vec![NativeLink { lib: "z".to_string(), archive: None }]);
    }

    #[test]
    fn link_plan_ignores_unwanted_libs() {
        let (_dep, author) = link_path_dep(
            "unwanted",
            "author",
            "        foo: { native: \"foo\", from: \"libfoo.a\", link: \"static\" }\n",
            ",\n    artifacts: \"example.com/org/author\"\n",
            None,
        );
        let (_dir, root) = link_tree(
            "unwanted",
            &format!("        author: {{ path: \"{}\" }}\n", author.display()),
            "",
        );
        let plan = resolve_wanted(&root, NativePrefer::None, &["other"]).unwrap();
        assert_eq!(plan, vec![NativeLink { lib: "other".to_string(), archive: None }]);
    }

    #[test]
    fn effective_choice_prefers_pin_then_flag_then_declared() {
        let declared = (true, Some(">=1.0"), None);
        let pin = crate::project::NativePin { system: false, version: None, from: Some("libz.so".to_string()) };
        let got = effective_choice(declared.0, declared.1, declared.2, Some(&pin), NativePrefer::System);
        assert_eq!(got, EffectiveNative { system: false, version: Some(">=1.0".to_string()), from: Some("libz.so".to_string()) });
        let got = effective_choice(false, None, Some("libz.so"), None, NativePrefer::System);
        assert_eq!(got, EffectiveNative { system: true, version: None, from: None });
        let got = effective_choice(true, Some(">=2.0.0"), None, None, NativePrefer::Bundled);
        assert_eq!(got, EffectiveNative { system: false, version: Some(">=2.0.0".to_string()), from: None });
        let got = effective_choice(true, None, None, None, NativePrefer::None);
        assert_eq!(got, EffectiveNative { system: true, version: None, from: None });
    }

    #[test]
    fn probe_rejects_bad_names_without_probing() {
        let _guard = test_path_guard();
        for bad in ["-lfoo", "a/b", "has space", "semi;colon", ""] {
            let err = verify_system_lib(bad, None).unwrap_err();
            assert_eq!(err.code, diagnostics::Code::E108);
            assert!(err.message.contains("not a valid library name"), "{}", err.message);
        }
    }

    #[test]
    fn probe_reads_pkg_config_and_checks_constraints() {
        let _guard = test_path_guard();
        let dir = stub_bin(
            "ok",
            "pkg-config",
            "#!/bin/sh\nif [ \"$1\" = \"--modversion\" ]; then echo \"1.2.13\"; exit 0; fi\nexit 1\n",
        );
        let _path = PathOverride::prepend(&dir);
        assert_eq!(verify_system_lib("z", Some(">=1.2.0")).unwrap(), "1.2.13");
        assert_eq!(verify_system_lib("z", None).unwrap(), "1.2.13");
        let err = verify_system_lib("z", Some(">=2.0.0")).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("1.2.13"), "{}", err.message);
        assert!(err.message.contains(">=2.0.0"), "{}", err.message);
    }

    #[test]
    fn probe_fails_closed_when_library_is_absent() {
        let _guard = test_path_guard();
        let dir = stub_bin("missing", "pkg-config", "#!/bin/sh\nexit 1\n");
        std::fs::write(dir.join("ldconfig"), "#!/bin/sh\nexit 1\n").unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let path = dir.join("ldconfig");
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        let _path = PathOverride::prepend(&dir);
        let err = verify_system_lib("nosuchlib_xyz", None).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("not found"), "{}", err.message);
    }

    #[test]
    fn triple_target_splits_arch_vendor_os_env() {
        let t = triple_target("x86_64-unknown-linux-gnu").unwrap();
        assert_eq!((t.arch.as_str(), t.os.as_str(), t.env.as_str()), ("x86_64", "linux", "gnu"));
        let t = triple_target("aarch64-apple-darwin").unwrap();
        assert_eq!((t.arch.as_str(), t.os.as_str(), t.env.as_str()), ("aarch64", "darwin", ""));
        assert!(triple_target("nope").is_none());
        assert!(triple_target("a-b-c-d-e").is_none());
    }

    #[test]
    fn owner_version_check_uses_range_grammar() {
        check_owner_version("z", "1.2.0", "^1.0.0").unwrap();
        let err = check_owner_version("z", "2.0.0", "^1.0.0").unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("2.0.0"), "{}", err.message);
    }
}
