use crate::checksum::compute_package_checksum;
use crate::project::{ProjectConfig};
use crate::project::eval::{ConfigTarget, ConfigValue, eval_module};
use crate::parser::Parser;
use diagnostics::{Code, Diagnostic};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const LOCK_VERSION: u32 = 2;
pub const LOCK_FILE: &str = "Project.deplock";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LockedPackage {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: String,
    pub artifact_digest: Option<String>,
    pub license: Option<String>,
    pub link: Option<String>,
    pub dependencies: Vec<String>,
    pub tier: String,
    pub capabilities: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProjectDepLock {
    pub version: u32,
    pub packages: Vec<LockedPackage>,
}

struct PkgNode {
    name: String,
    version: String,
    source: String,
    checksum: String,
    artifact_digest: Option<String>,
    license: String,
    link: String,
    deps: Vec<(String, String)>,
    dir: PathBuf,
}

fn lock_have(lock: Option<&ProjectDepLock>) -> Vec<crate::fetch::HaveEntry> {
    let Some(lock) = lock else {
        return Vec::new();
    };
    let mut wanted: BTreeSet<(String, String)> = BTreeSet::new();
    for p in &lock.packages {
        if let Some(rest) = p.source.strip_prefix("registry:") {
            if let Some((full, version)) = rest.rsplit_once('@') {
                if !full.is_empty() && !version.is_empty() {
                    wanted.insert((full.to_string(), version.to_string()));
                }
            }
        }
    }
    crate::fetch::scan_cache_have()
        .into_iter()
        .filter(|h| wanted.contains(&(h.full.clone(), h.version.clone())))
        .collect()
}

fn existing_have(root: &Path) -> Vec<crate::fetch::HaveEntry> {
    lock_have(ProjectDepLock::load(root).ok().flatten().as_ref())
}

fn rel_parts(path: &Path) -> Option<Vec<String>> {
    let mut parts = Vec::new();
    for c in path.components() {
        match c {
            std::path::Component::CurDir
            | std::path::Component::RootDir
            | std::path::Component::Prefix(_) => {}
            std::path::Component::Normal(s) => parts.push(s.to_string_lossy().into_owned()),
            _ => return None,
        }
    }
    Some(parts)
}

fn rel_path(from_dir: &Path, to_path: &Path) -> Option<String> {
    let from = rel_parts(from_dir)?;
    let mut to = rel_parts(to_path)?;
    let mut common = 0;
    while common < from.len() && common < to.len() && from[common] == to[common] {
        common += 1;
    }
    if common == 0 {
        return None;
    }
    let mut out: Vec<String> = Vec::new();
    for _ in common..from.len() {
        out.push("..".to_string());
    }
    out.append(&mut to.split_off(common));
    Some(out.join("/"))
}

fn source_of(root: &Path, pkg_root: &Path, is_root: bool) -> String {
    if is_root {
        return "root".to_string();
    }
    match rel_path(root, pkg_root) {
        Some(rel) => format!("path:{rel}"),
        None => format!("path:{}", pkg_root.display()),
    }
}

fn requested_link(spec: &crate::project::DependencySpec) -> crate::license::LinkPreference {
    match spec {
        crate::project::DependencySpec::Semver { link, .. }
        | crate::project::DependencySpec::Path { link, .. }
        | crate::project::DependencySpec::Git { link, .. }
        | crate::project::DependencySpec::Url { link, .. }
        | crate::project::DependencySpec::Native { link, .. } => *link,
    }
}

fn collect_into(
    nodes: &mut BTreeMap<String, PkgNode>,
    anchor: &Path,
    root: &Path,
    config: &ProjectConfig,
    is_root: bool,
    have: &[crate::fetch::HaveEntry],
    pins: &BTreeMap<String, crate::project::NativePin>,
    prefer: crate::native::NativePrefer,
    used_pins: &mut BTreeSet<String>,
    overrides: &BTreeMap<String, String>,
    locked: &BTreeMap<String, String>,
) -> Result<(), Diagnostic> {
    let mut stack: Vec<(PathBuf, ProjectConfig, bool, Option<String>, String)> =
        vec![(root.to_path_buf(), config.clone(), is_root, None, "static".to_string())];
    while let Some((pkg_root, cfg, first, source_hint, incoming_link)) = stack.pop() {

        if nodes.contains_key(&cfg.name) {
            let seen = &nodes[&cfg.name];
            if seen.version != cfg.version {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "version conflict for package `{}`: {} and {}",
                        cfg.name, seen.version, cfg.version
                    ),
                ));
            }
            continue;
        }
        let mut deps: Vec<(String, String)> = Vec::new();
        for (dep_name, spec) in &cfg.dependencies {
            if let crate::project::DependencySpec::Native { lib, system, version, from, link, .. } = spec {
                if nodes.contains_key(dep_name) {
                    deps.push((dep_name.clone(), String::new()));
                    continue;
                }
                let pin = pins.get(dep_name);
                if pin.is_some() {
                    used_pins.insert(dep_name.clone());
                }
                let eff = crate::native::effective_choice(
                    *system,
                    version.as_deref(),
                    from.as_deref(),
                    pin,
                    prefer,
                );
                if !eff.system
                    && let Some(range) = eff.version.as_deref()
                {
                    crate::native::check_owner_version(dep_name, &cfg.version, range)?;
                }
                let installed = if eff.system {
                    Some(crate::native::verify_system_lib(lib, eff.version.as_deref())?)
                } else {
                    None
                };
                let owner_digest = if eff.from.is_some() {
                    crate::fetch::cached_artifact_digest(&pkg_root)
                } else {
                    None
                };
                let source = if eff.system {
                    format!("native:{lib}+system")
                } else if let Some(title) = eff.from.as_deref() {
                    format!("native:{lib}#{title}")
                } else {
                    format!("native:{lib}")
                };
                let has_shared = eff.system || eff.from.is_some();
                let effective = link.effective(has_shared).to_string();
                nodes.insert(
                    dep_name.clone(),
                    PkgNode {
                        source,
                        name: dep_name.clone(),
                        version: installed.unwrap_or_default(),
                        checksum: String::new(),
                        artifact_digest: owner_digest,
                        license: "unknown".to_string(),
                        link: effective,
                        deps: Vec::new(),
                        dir: pkg_root.clone(),
                    },
                );
                deps.push((dep_name.clone(), String::new()));
                continue;
            }
            let want_link = requested_link(spec);
            if want_link == crate::license::LinkPreference::Dynamic {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("dependency `{dep_name}`: `link: \"dynamic\"` needs a native dependency (only native entries provide shared objects)"),
                ));
            }
            let effective = want_link.effective(false).to_string();
            let (dep_root, dep_cfg, hint) = match spec {
                crate::project::DependencySpec::Path { path, .. } => {
                    let dep_root = std::fs::canonicalize(pkg_root.join(path)).map_err(|_| {
                        Diagnostic::new(Code::E108, format!("cannot resolve package `{dep_name}`"))
                    })?;
                    let dep_cfg = ProjectConfig::load_from_dir(&dep_root)?.ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("package `{dep_name}` has no Project.config"))
                    })?;
                    (dep_root, dep_cfg, None)
                }
                crate::project::DependencySpec::Git { git, rev, .. } => {
                    let anchor = crate::fetch::cache_anchor(&pkg_root);
                    let dep_root = crate::fetch::resolve_git_dep(&anchor, dep_name, git, rev)?;
                    let dep_cfg = ProjectConfig::load_from_dir(&dep_root)?.ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("package `{dep_name}` has no Project.config"))
                    })?;
                    (dep_root, dep_cfg, Some(format!("git:{git}#{rev}")))
                }
                crate::project::DependencySpec::Semver { version, .. } => {
                    let req = overrides.get(dep_name).map(String::as_str).unwrap_or(version.as_str());
                    let (dep_root, dep_cfg, resolved) =
                        crate::fetch::resolve_registry_package(
                            dep_name,
                            req,
                            cfg.registry.as_ref(),
                            &cfg.registries,
                            locked.get(dep_name).cloned(),
                            have,
                            None,
                        )?;
                    (dep_root, dep_cfg, Some(format!("registry:{dep_name}@{resolved}")))
                }
                crate::project::DependencySpec::Url { url, .. } => {
                    return Err(Diagnostic::new(
                        Code::E108,
                        format!("cannot resolve package `{dep_name}`: tarball `{url}` fetch is not implemented yet"),
                    ));
                }
                crate::project::DependencySpec::Native { .. } => {
                    unreachable!("native dependencies are recorded above")
                }
            };
            deps.push((dep_cfg.name.clone(), dep_cfg.version.clone()));
            stack.push((dep_root, dep_cfg, false, hint, effective));
        }
        let source = source_hint.unwrap_or_else(|| source_of(anchor, &pkg_root, first));
        // The lock pins the registry manifest digest for registry packages,
        // not the local tree hash: the digest is the version identity the
        // registry served and verified. Path, git and root packages keep the
        // local tree checksum.
        let checksum = if source.starts_with("registry:") {
            crate::fetch::cached_integrity(&pkg_root).ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("package `{}` has no registry integrity", cfg.name),
                )
            })?
        } else {
            compute_package_checksum(&pkg_root)?
        };
        let artifact_digest = if source.starts_with("registry:") {
            crate::fetch::cached_artifact_digest(&pkg_root)
        } else {
            None
        };
        nodes.insert(
            cfg.name.clone(),
            PkgNode {
                source,
                name: cfg.name.clone(),
                version: cfg.version.clone(),
                checksum,
                artifact_digest,
                license: crate::license::resolve_package_license(&pkg_root, cfg.license.as_deref()),
                link: incoming_link.clone(),
                deps,
                dir: pkg_root.clone(),
            },
        );
    }
    Ok(())
}

fn collect_graph(
    root: &Path,
    config: &ProjectConfig,
    have: &[crate::fetch::HaveEntry],
    prefer: crate::native::NativePrefer,
    overrides: &BTreeMap<String, String>,
    locked: &BTreeMap<String, String>,
) -> Result<Vec<PkgNode>, Diagnostic> {
    let mut nodes: BTreeMap<String, PkgNode> = BTreeMap::new();
    let mut used_pins: BTreeSet<String> = BTreeSet::new();
    collect_into(&mut nodes, root, root, config, true, have, &config.native_pins, prefer, &mut used_pins, overrides, locked)?;
    for name in config.native_pins.keys() {
        if !used_pins.contains(name) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("native pin `{name}` matches no native dependency in scope"),
            ));
        }
    }
    Ok(nodes.into_values().collect())
}

fn collect_workspace_graph(
    ws_root: &Path,
    members: &BTreeMap<String, (PathBuf, ProjectConfig)>,
    have: &[crate::fetch::HaveEntry],
    prefer: crate::native::NativePrefer,
    overrides: &BTreeMap<String, String>,
    locked: &BTreeMap<String, String>,
) -> Result<Vec<PkgNode>, Diagnostic> {
    let ws_pins = ProjectConfig::load_from_dir(ws_root)
        .ok()
        .flatten()
        .map(|c| c.native_pins)
        .unwrap_or_default();
    let mut nodes: BTreeMap<String, PkgNode> = BTreeMap::new();
    let mut used_pins: BTreeSet<String> = BTreeSet::new();
    for (root, cfg) in members.values() {
        collect_into(&mut nodes, ws_root, root, cfg, false, have, &ws_pins, prefer, &mut used_pins, overrides, locked)?;
    }
    for name in ws_pins.keys() {
        if !used_pins.contains(name) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("native pin `{name}` matches no native dependency in scope"),
            ));
        }
    }
    Ok(nodes.into_values().collect())
}

fn nodes_to_lock(nodes: Vec<PkgNode>) -> Result<ProjectDepLock, Diagnostic> {
    let mut packages: Vec<LockedPackage> = Vec::new();
    for n in nodes {
        if n.source.starts_with("native:") {
            packages.push(LockedPackage {
                name: n.name,
                version: n.version,
                source: n.source,
                checksum: n.checksum,
                artifact_digest: None,
                license: Some(n.license),
                link: Some(n.link),
                dependencies: Vec::new(),
                tier: "system".to_string(),
                capabilities: Vec::new(),
            });
            continue;
        }
        let a = crate::security::analyze_dep_dir(&n.dir)?;
        packages.push(LockedPackage {
            name: n.name,
            version: n.version,
            source: n.source,
            checksum: n.checksum,
            artifact_digest: n.artifact_digest,
            license: Some(n.license),
            link: Some(n.link),
            dependencies: n.deps.iter().map(|(n, v)| {
                if v.is_empty() {
                    n.clone()
                } else {
                    format!("{n} {v}")
                }
            }).collect(),
            tier: a.tier.to_string(),
            capabilities: a.capabilities.clone(),
        });
    }
    packages.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(ProjectDepLock {
        version: LOCK_VERSION,
        packages,
    })
}

fn check_nodes(nodes: Vec<PkgNode>, lock: &ProjectDepLock) -> Result<(), Diagnostic> {
    if lock.version != 1 && lock.version != LOCK_VERSION {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "unsupported Project.deplock version {}, run 'rnx lock'",
                lock.version
            ),
        ));
    }
    let by_name: BTreeMap<&str, &LockedPackage> =
        lock.packages.iter().map(|p| (p.name.as_str(), p)).collect();
    for node in nodes {
        let locked = by_name.get(node.name.as_str()).ok_or_else(|| {
            Diagnostic::new(
                Code::E108,
                format!(
                    "package `{}` not found in Project.deplock, run 'rnx lock'",
                    node.name
                ),
            )
        })?;
        if node.source.starts_with("native:") {
            if locked.source != node.source {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "source mismatch for package `{}`: Project.deplock has {}, project has {}",
                        node.name, locked.source, node.source
                    ),
                ));
            }
            if locked.version != node.version {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "version mismatch for package `{}`: Project.deplock has {}, project has {}",
                        node.name, locked.version, node.version
                    ),
                ));
            }
            if locked.artifact_digest != node.artifact_digest {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("artifact mismatch for package `{}`", node.name),
                ));
            }
            if let Some(want) = locked.link.as_deref()
                && want != node.link.as_str()
            {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!(
                        "link mismatch for package `{}`: Project.deplock has `{want}`, project wants `{}` (run 'rnx lock')",
                        node.name, node.link,
                    ),
                ));
            }
            continue;
        }
        if locked.version != node.version {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "version mismatch for package `{}`: Project.deplock has {}, project has {}",
                    node.name, locked.version, node.version
                ),
            ));
        }
        if locked.source != node.source {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "source mismatch for package `{}`: Project.deplock has {}, project has {}",
                    node.name, locked.source, node.source
                ),
            ));
        }
        if locked.checksum != node.checksum {
            return Err(Diagnostic::new(
                Code::E108,
                format!("checksum mismatch for package `{}`", node.name),
            ));
        }
        if locked.artifact_digest != node.artifact_digest {
            return Err(Diagnostic::new(
                Code::E108,
                format!("artifact mismatch for package `{}`", node.name),
            ));
        }
        if let Some(want) = locked.link.as_deref()
            && want != node.link.as_str()
        {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "link mismatch for package `{}`: Project.deplock has `{want}`, project wants `{}` (run 'rnx lock')",
                    node.name, node.link,
                ),
            ));
        }
        let scan = crate::security::analyze_dep_dir(&node.dir)?;
        let mut deduced = BTreeSet::new();
        for s in &scan.capabilities {
            match crate::security::parse_cap(s) {
                Ok(c) => {
                    deduced.insert(c);
                }
                Err(d) => return Err(d),
            }
        }
        let loc = |_: &crate::capabilities::Capability| node.dir.display().to_string();
        if let Some(d) = crate::security::entry_scan_diags(&node.name, &deduced, locked, &loc)
            .into_iter()
            .next()
        {
            return Err(d);
        }
    }
    Ok(())
}

impl ProjectDepLock {
    pub fn resolve(root: &Path, config: &ProjectConfig) -> Result<ProjectDepLock, Diagnostic> {
        Self::resolve_with(root, config, crate::native::NativePrefer::from_env())
    }

    pub fn resolve_with(
        root: &Path,
        config: &ProjectConfig,
        prefer: crate::native::NativePrefer,
    ) -> Result<ProjectDepLock, Diagnostic> {
        Self::resolve_with_overrides(root, config, prefer, &BTreeMap::new())
    }

    pub fn resolve_with_overrides(
        root: &Path,
        config: &ProjectConfig,
        prefer: crate::native::NativePrefer,
        overrides: &BTreeMap<String, String>,
    ) -> Result<ProjectDepLock, Diagnostic> {
        let have = existing_have(root);
        nodes_to_lock(collect_graph(root, config, &have, prefer, overrides, &BTreeMap::new())?)
    }

    pub fn resolve_workspace(
        ws_root: &Path,
        members: &BTreeMap<String, (PathBuf, ProjectConfig)>,
    ) -> Result<ProjectDepLock, Diagnostic> {
        Self::resolve_workspace_with(ws_root, members, crate::native::NativePrefer::from_env())
    }

    pub fn resolve_workspace_with(
        ws_root: &Path,
        members: &BTreeMap<String, (PathBuf, ProjectConfig)>,
        prefer: crate::native::NativePrefer,
    ) -> Result<ProjectDepLock, Diagnostic> {
        Self::resolve_workspace_with_overrides(ws_root, members, prefer, &BTreeMap::new())
    }

    pub fn resolve_workspace_with_overrides(
        ws_root: &Path,
        members: &BTreeMap<String, (PathBuf, ProjectConfig)>,
        prefer: crate::native::NativePrefer,
        overrides: &BTreeMap<String, String>,
    ) -> Result<ProjectDepLock, Diagnostic> {
        let have = existing_have(ws_root);
        nodes_to_lock(collect_workspace_graph(ws_root, members, &have, prefer, overrides, &BTreeMap::new())?)
    }

    pub fn load(root: &Path) -> Result<Option<ProjectDepLock>, Diagnostic> {
        let path = root.join(LOCK_FILE);
        let text = match std::fs::read_to_string(&path) {
            Ok(t) => t,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
            Err(e) => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("cannot read {}: {e}", path.display()),
                ));
            }
        };
        match ProjectDepLock::parse(&text) {
            Ok(l) => Ok(Some(l)),
            Err(msg) => Err(Diagnostic::new(
                Code::E108,
                format!("{}: {msg}", path.display()),
            )),
        }
    }

    pub fn write(&self, root: &Path) -> Result<(), Diagnostic> {
        let path = root.join(LOCK_FILE);
        std::fs::write(&path, self.to_rnx()).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write {}: {e}", path.display()))
        })
    }

    pub fn to_rnx(&self) -> String {
        let mut out = String::from("// Project.deplock - GENERATED AUTOMATICALLY BY RNX LOCK - DO NOT EDIT\nexport default {\n    version: ");
        out.push_str(&self.version.to_string());
        out.push_str(",\n    packages: [");
        if self.packages.is_empty() {
            out.push_str("]\n}\n");
            return out;
        }
        out.push('\n');
        for (i, p) in self.packages.iter().enumerate() {
            out.push_str("        {\n");
            out.push_str(&format!("            name: {},\n", rnx_string(&p.name)));
            out.push_str(&format!("            version: {},\n", rnx_string(&p.version)));
            out.push_str(&format!("            source: {},\n", rnx_string(&p.source)));
            out.push_str(&format!("            checksum: {},\n", rnx_string(&p.checksum)));
            if let Some(digest) = &p.artifact_digest {
                out.push_str(&format!("            artifactDigest: {},\n", rnx_string(digest)));
            }
            if let Some(license) = &p.license {
                out.push_str(&format!("            license: {},\n", rnx_string(license)));
            }
            if let Some(link) = &p.link {
                out.push_str(&format!("            link: {},\n", rnx_string(link)));
            }
            out.push_str("            dependencies: [");
            for (j, d) in p.dependencies.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                out.push_str(&rnx_string(d));
            }
            out.push_str("],\n");
            out.push_str(&format!("            tier: {},\n", rnx_string(&p.tier)));
            out.push_str("            capabilities: [");
            for (j, c) in p.capabilities.iter().enumerate() {
                if j > 0 {
                    out.push_str(", ");
                }
                out.push_str(&rnx_string(c));
            }
            out.push_str("]\n");
            out.push_str("        }");
            if i + 1 < self.packages.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("    ]\n}\n");
        out
    }

    pub fn parse(text: &str) -> Result<ProjectDepLock, String> {
        let module = Parser::parse_module(text).map_err(|e| e.message.clone())?;
        let value =
            eval_module(&module, ConfigTarget::host()).map_err(|e| e.message.clone())?;
        let top = match &value {
            ConfigValue::Object(_) => &value,
            _ => return Err("lockfile `export default` must be an object".to_string()),
        };
        let version = match top.get("version") {
            Some(ConfigValue::Int(v)) if *v >= 0 => *v as u32,
            Some(_) => return Err("field `version` must be a non-negative integer".to_string()),
            None => return Err("missing required field `version`".to_string()),
        };
        let mut packages: Vec<LockedPackage> = Vec::new();
        match top.get("packages") {
            Some(ConfigValue::Array(items)) => {
                for item in items {
                    packages.push(parse_locked(item)?);
                }
            }
            Some(_) => return Err("field `packages` must be an array".to_string()),
            None => return Err("missing required field `packages`".to_string()),
        }
        packages.sort_by(|a, b| a.name.cmp(&b.name));
        Ok(ProjectDepLock { version, packages })
    }
}

fn parse_locked(val: &ConfigValue) -> Result<LockedPackage, String> {
    let obj = match val {
        ConfigValue::Object(_) => val,
        _ => return Err("locked package must be an object".to_string()),
    };
    let get_str = |key: &str| -> Result<String, String> {
        match obj.get(key) {
            Some(ConfigValue::String(s)) => Ok(s.clone()),
            Some(_) => Err(format!("locked package field `{key}` must be a string")),
            None => Err(format!("locked package missing field `{key}`")),
        }
    };
    let str_list = |key: &str, required: bool| -> Result<Vec<String>, String> {
        match obj.get(key) {
            Some(ConfigValue::Array(items)) => {
                let mut list = Vec::new();
                for item in items {
                    match item {
                        ConfigValue::String(s) => list.push(s.clone()),
                        _ => return Err(format!("locked package `{key}` must be strings")),
                    }
                }
                Ok(list)
            }
            Some(_) => Err(format!("locked package field `{key}` must be an array")),
            None if required => Err(format!("locked package missing field `{key}`")),
            None => Ok(Vec::new()),
        }
    };
    let dependencies = str_list("dependencies", true)?;
    let capabilities = str_list("capabilities", false)?;
    let tier = match obj.get("tier") {
        Some(ConfigValue::String(s)) => s.clone(),
        Some(_) => return Err("locked package field `tier` must be a string".to_string()),
        None => "pure".to_string(),
    };
    let artifact_digest = match obj.get("artifactDigest") {
        Some(ConfigValue::String(s)) => Some(s.clone()),
        Some(_) => return Err("locked package field `artifactDigest` must be a string".to_string()),
        None => None,
    };
    let license = match obj.get("license") {
        Some(ConfigValue::String(s)) => Some(s.clone()),
        Some(_) => return Err("locked package field `license` must be a string".to_string()),
        None => None,
    };
    let link = match obj.get("link") {
        Some(ConfigValue::String(s)) if s == "static" || s == "dynamic" => Some(s.clone()),
        Some(ConfigValue::String(s)) => {
            return Err(format!("locked package field `link` `{s}` must be `static` or `dynamic`"))
        }
        Some(_) => return Err("locked package field `link` must be a string".to_string()),
        None => None,
    };
    Ok(LockedPackage {
        name: get_str("name")?,
        version: get_str("version")?,
        source: get_str("source")?,
        checksum: get_str("checksum")?,
        artifact_digest,
        license,
        link,
        dependencies,
        tier,
        capabilities,
    })
}

pub fn rnx_string(s: &str) -> String {
    let mut out = String::with_capacity(s.len() + 2);
    out.push('"');
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => {
                out.push_str(&format!("\\u{{{:x}}}", c as u32));
            }
            _ => out.push(c),
        }
    }
    out.push('"');
    out
}
pub fn verify_lock(root: &Path, config: &ProjectConfig, lock: &ProjectDepLock) -> Result<(), Diagnostic> {
    verify_lock_with(root, config, lock, crate::native::NativePrefer::from_env())
}

pub fn verify_lock_with(
    root: &Path,
    config: &ProjectConfig,
    lock: &ProjectDepLock,
    prefer: crate::native::NativePrefer,
) -> Result<(), Diagnostic> {
    let have = lock_have(Some(lock));
    check_nodes(collect_graph(root, config, &have, prefer, &BTreeMap::new(), &registry_pins(lock))?, lock)
}

pub fn verify_workspace_lock(
    ws_root: &Path,
    members: &BTreeMap<String, (PathBuf, ProjectConfig)>,
    lock: &ProjectDepLock,
) -> Result<(), Diagnostic> {
    verify_workspace_lock_with(ws_root, members, lock, crate::native::NativePrefer::from_env())
}

pub fn verify_workspace_lock_with(
    ws_root: &Path,
    members: &BTreeMap<String, (PathBuf, ProjectConfig)>,
    lock: &ProjectDepLock,
    prefer: crate::native::NativePrefer,
) -> Result<(), Diagnostic> {
    let have = lock_have(Some(lock));
    check_nodes(collect_workspace_graph(ws_root, members, &have, prefer, &BTreeMap::new(), &registry_pins(lock))?, lock)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateSelection {
    pub name: String,
    pub selector: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct UpdateChange {
    pub name: String,
    pub old: String,
    pub new: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct UpdateSummary {
    pub updated: Vec<UpdateChange>,
    pub held: Vec<UpdateChange>,
}

pub fn parse_update_selector(arg: &str) -> Result<UpdateSelection, Diagnostic> {
    let bad = || {
        Diagnostic::new(
            Code::E108,
            format!("bad update target `{arg}`: expected `name` or `name@range`"),
        )
    };
    let arg = arg.trim();
    if arg.is_empty() {
        return Err(bad());
    }
    match arg.rfind('@').filter(|&i| i > 0) {
        None => {
            if arg == "@" {
                return Err(bad());
            }
            Ok(UpdateSelection { name: arg.to_string(), selector: None })
        }
        Some(i) => {
            let name = arg[..i].trim();
            let selector = arg[i + 1..].trim();
            if name.is_empty() || selector.is_empty() {
                return Err(bad());
            }
            Ok(UpdateSelection { name: name.to_string(), selector: Some(selector.to_string()) })
        }
    }
}

fn major_of(version: &str) -> Option<u64> {
    version.split('.').next()?.trim().parse::<u64>().ok()
}

fn locked_registry<'a>(lock: &'a ProjectDepLock, name: &str) -> Option<&'a LockedPackage> {
    lock.packages
        .iter()
        .find(|p| p.name == name && p.source.starts_with("registry:"))
}

fn registry_pins(lock: &ProjectDepLock) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for p in &lock.packages {
        if p.source.starts_with("registry:") {
            out.insert(p.name.clone(), p.version.clone());
        }
    }
    out
}

fn manifest_ranges(configs: &[&ProjectConfig]) -> BTreeMap<String, String> {
    let mut out = BTreeMap::new();
    for config in configs {
        for (name, spec) in &config.dependencies {
            if let crate::project::DependencySpec::Semver { version, .. } = spec {
                out.entry(name.clone()).or_insert_with(|| version.clone());
            }
        }
    }
    out
}

fn update_overrides(
    ranges: &BTreeMap<String, String>,
    old: &ProjectDepLock,
    selections: &[UpdateSelection],
    update_all: bool,
) -> Result<(BTreeMap<String, String>, BTreeSet<String>), Diagnostic> {
    let mut wanted: BTreeMap<String, Option<String>> = BTreeMap::new();
    for s in selections {
        if wanted.contains_key(&s.name) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("duplicate update target `{}`", s.name),
            ));
        }
        wanted.insert(s.name.clone(), s.selector.clone());
    }
    for name in wanted.keys() {
        if !ranges.contains_key(name) {
            return Err(Diagnostic::new(Code::E108, format!("package `{name}` is not a registry dependency (only registry ranges update)")));
        }
    }
    let mut overrides = BTreeMap::new();
    let mut explicit = BTreeSet::new();
    for name in ranges.keys() {
        match wanted.get(name) {
            Some(Some(selector)) => {
                if !crate::fetch::is_valid_range(selector) {
                    return Err(Diagnostic::new(Code::E108, format!("selector `{selector}` for package `{name}` is not a valid range (`*`, `latest`, `X.Y.Z`, `^`, `~`, `>=`); dist-tags are not resolved by this client")));
                }
                explicit.insert(name.clone());
                overrides.insert(name.clone(), selector.clone());
            }
            Some(None) => {}
            None => {
                if !update_all
                    && let Some(pinned) = locked_registry(old, name)
                {
                    overrides.insert(name.clone(), pinned.version.clone());
                }
            }
        }
    }
    Ok((overrides, explicit))
}

fn update_targets(
    ranges: &BTreeMap<String, String>,
    old: &ProjectDepLock,
    selections: &[UpdateSelection],
    update_all: bool,
) -> BTreeSet<String> {
    if update_all {
        return ranges.keys().cloned().collect();
    }
    let mut out: BTreeSet<String> = selections.iter().map(|s| s.name.clone()).collect();
    for name in ranges.keys() {
        if locked_registry(old, name).is_none() {
            out.insert(name.clone());
        }
    }
    out
}

fn summarize_update(old: &ProjectDepLock, new: &ProjectDepLock) -> Vec<UpdateChange> {
    let mut updated = Vec::new();
    for p in &new.packages {
        if !p.source.starts_with("registry:") {
            continue;
        }
        match locked_registry(old, &p.name) {
            Some(o)
                if o.version == p.version
                    && o.checksum == p.checksum
                    && o.artifact_digest == p.artifact_digest => {}
            Some(o) => updated.push(UpdateChange {
                name: p.name.clone(),
                old: o.version.clone(),
                new: p.version.clone(),
            }),
            None => updated.push(UpdateChange {
                name: p.name.clone(),
                old: "none".to_string(),
                new: p.version.clone(),
            }),
        }
    }
    updated.sort_by(|a, b| a.name.cmp(&b.name));
    updated
}

fn hold_major_moves(
    old: &ProjectDepLock,
    candidate: &ProjectDepLock,
    targets: &BTreeSet<String>,
    explicit: &BTreeSet<String>,
    allow_major: bool,
    overrides: &mut BTreeMap<String, String>,
) -> Vec<UpdateChange> {
    if allow_major {
        return Vec::new();
    }
    let mut held = Vec::new();
    for name in targets {
        if explicit.contains(name) {
            continue;
        }
        let old_version = locked_registry(old, name).map(|p| p.version.as_str()).unwrap_or_default();
        let new_version = candidate
            .packages
            .iter()
            .find(|p| p.name == *name)
            .map(|p| p.version.as_str())
            .unwrap_or_default();
        if old_version.is_empty() || new_version.is_empty() || old_version == new_version {
            continue;
        }
        let crossed = match (major_of(old_version), major_of(new_version)) {
            (Some(a), Some(b)) => a != b,
            _ => false,
        };
        if crossed {
            overrides.insert(name.clone(), old_version.to_string());
            held.push(UpdateChange {
                name: name.clone(),
                old: old_version.to_string(),
                new: new_version.to_string(),
            });
        }
    }
    held.sort_by(|a, b| a.name.cmp(&b.name));
    held
}

impl ProjectDepLock {
    pub fn update_lock(
        root: &Path,
        config: &ProjectConfig,
        prefer: crate::native::NativePrefer,
        selections: &[UpdateSelection],
        allow_major: bool,
    ) -> Result<(ProjectDepLock, UpdateSummary), Diagnostic> {
        let old = ProjectDepLock::load(root)?.ok_or_else(|| {
            Diagnostic::new(Code::E108, "Project.deplock not found, run 'rnx lock'")
        })?;
        let ranges = manifest_ranges(std::slice::from_ref(&config));
        let update_all = selections.is_empty();
        let (mut overrides, explicit) = update_overrides(&ranges, &old, selections, update_all)?;
        let targets = update_targets(&ranges, &old, selections, update_all);
        let have = existing_have(root);
        let mut candidate = nodes_to_lock(collect_graph(root, config, &have, prefer, &overrides, &BTreeMap::new())?)?;
        let held = hold_major_moves(&old, &candidate, &targets, &explicit, allow_major, &mut overrides);
        if !held.is_empty() {
            candidate = nodes_to_lock(collect_graph(root, config, &have, prefer, &overrides, &BTreeMap::new())?)?;
        }
        let updated = summarize_update(&old, &candidate);
        Ok((candidate, UpdateSummary { updated, held }))
    }

    pub fn update_workspace_lock(
        ws_root: &Path,
        members: &BTreeMap<String, (PathBuf, ProjectConfig)>,
        prefer: crate::native::NativePrefer,
        selections: &[UpdateSelection],
        allow_major: bool,
    ) -> Result<(ProjectDepLock, UpdateSummary), Diagnostic> {
        let old = ProjectDepLock::load(ws_root)?.ok_or_else(|| {
            Diagnostic::new(Code::E108, "Project.deplock not found, run 'rnx lock'")
        })?;
        let configs: Vec<&ProjectConfig> = members.values().map(|(_, c)| c).collect();
        let ranges = manifest_ranges(&configs);
        let update_all = selections.is_empty();
        let (mut overrides, explicit) = update_overrides(&ranges, &old, selections, update_all)?;
        let targets = update_targets(&ranges, &old, selections, update_all);
        let have = existing_have(ws_root);
        let mut candidate =
            nodes_to_lock(collect_workspace_graph(ws_root, members, &have, prefer, &overrides, &BTreeMap::new())?)?;
        let held = hold_major_moves(&old, &candidate, &targets, &explicit, allow_major, &mut overrides);
        if !held.is_empty() {
            candidate =
                nodes_to_lock(collect_workspace_graph(ws_root, members, &have, prefer, &overrides, &BTreeMap::new())?)?;
        }
        let updated = summarize_update(&old, &candidate);
        Ok((candidate, UpdateSummary { updated, held }))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tree(tag: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("lock-{tag}"));
        std::fs::create_dir_all(dir.join("dep").join("src")).unwrap();
        std::fs::create_dir_all(dir.join("app").join("src")).unwrap();
        std::fs::write(
            dir.join("dep").join("Project.config"),
            "export default {\n    project: {\n        name: \"dep\",\n        version: \"0.2.0\"\n    },\n    entries: { main: \"src/lib.rnx\" }\n}",
        )
        .unwrap();
        std::fs::write(dir.join("dep").join("src").join("lib.rnx"), "fn f(): Int { return 1; }\n").unwrap();
        std::fs::write(
            dir.join("app").join("Project.config"),
            "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    },\n    entries: { main: \"src/main.rnx\" },\n    dependencies: {\n        dep: { path: \"../dep\" }\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("app").join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        dir
    }

    #[test]
    fn lock_roundtrip_is_sorted_and_deterministic() {
        let dir = tree("round");
        let root = dir.join("app");
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = ProjectDepLock::resolve(&root, &cfg).unwrap();
        assert_eq!(lock.packages.len(), 2);
        assert_eq!(lock.packages[0].name, "app");
        assert_eq!(lock.packages[0].source, "root");
        assert_eq!(lock.packages[0].dependencies, vec!["dep 0.2.0".to_string()]);
        assert_eq!(lock.packages[1].name, "dep");
        assert!(lock.packages[1].source.starts_with("path:"));
        assert_eq!(lock.packages[1].checksum.len(), 64);
        let text = lock.to_rnx();
        let again = ProjectDepLock::resolve(&root, &cfg).unwrap().to_rnx();
        assert_eq!(text, again);
        let parsed = ProjectDepLock::parse(&text).unwrap();
        assert_eq!(parsed, lock);
        verify_lock(&root, &cfg, &parsed).unwrap();
    }

    #[test]
    fn semver_dep_locks_from_registry() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("deplock-reg");
        let (manifest, pairs, digest, _pc, _main) =
            crate::fetch::testkit::fixture_tree("@acme/widget", "1.2.0");
        let node = crate::fetch::testkit::node_json("@acme/widget", "1.2.0", &digest, false);
        let server = crate::fetch::testkit::MockRegistry::start(
            crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest),
                chunk_override: Some(pairs),
            },
        );
        let dir = crate::scratch::dir("lock");
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n        \"@acme/widget\": {{ version: \"^1.0.0\" }}\n    }},\n    registry: {{ url: \"{}\" }}\n}}\n",
                server.base
            ),
        )
        .unwrap();
        std::fs::write(root.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        let cfg = crate::project::ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = ProjectDepLock::resolve(&root, &cfg).unwrap();
        assert_eq!(lock.packages.len(), 2);
        let widget = lock.packages.iter().find(|p| p.name == "@acme/widget").unwrap();
        assert_eq!(widget.version, "1.2.0");
        assert_eq!(widget.source, "registry:@acme/widget@1.2.0");
        assert_eq!(widget.checksum, digest);
        verify_lock(&root, &cfg, &lock).unwrap();
    }

    #[test]
    fn verify_rejects_tamper_and_missing_entries() {
        let dir = tree("tamper");
        let root = dir.join("app");
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let mut lock = ProjectDepLock::resolve(&root, &cfg).unwrap();
        std::fs::write(dir.join("dep").join("src").join("lib.rnx"), "fn f(): Int { return 2; }\n").unwrap();
        let e = verify_lock(&root, &cfg, &lock).unwrap_err();
        assert!(e.message.contains("checksum mismatch"), "{}", e.message);
        lock.packages.retain(|p| p.name != "dep");
        let e = verify_lock(&root, &cfg, &lock).unwrap_err();
        assert!(e.message.contains("not found in Project.deplock"), "{}", e.message);
    }

    fn native_tree(tag: &str, deps: &str, extra: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
        let dir = crate::scratch::dir(&format!("lock-native-{tag}"));
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n{deps}    }}{extra}}}\n"
            ),
        )
        .unwrap();
        std::fs::write(root.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        (dir, root)
    }

    fn stub_pkg_config(tag: &str, version: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("lock-pkgcfg-{tag}"));
        let path = dir.join("pkg-config");
        std::fs::write(
            &path,
            format!("#!/bin/sh\nif [ \"$1\" = \"--modversion\" ]; then echo \"{version}\"; exit 0; fi\nexit 1\n"),
        )
        .unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mut perms = std::fs::metadata(&path).unwrap().permissions();
            perms.set_mode(0o755);
            std::fs::set_permissions(&path, perms).unwrap();
        }
        dir
    }

    struct PathPrepend {
        prev: Option<String>,
    }

    impl Drop for PathPrepend {
        fn drop(&mut self) {
            unsafe {
                match &self.prev {
                    Some(p) => std::env::set_var("PATH", p),
                    None => std::env::remove_var("PATH"),
                }
            }
        }
    }

    fn prepend_path(dir: &std::path::Path) -> PathPrepend {
        let prev = std::env::var("PATH").ok();
        let next = match &prev {
            Some(p) if !p.is_empty() => format!("{}:{p}", dir.display()),
            _ => dir.display().to_string(),
        };
        unsafe {
            std::env::set_var("PATH", next);
        }
        PathPrepend { prev }
    }

    #[test]
    fn native_system_choice_records_installed_version() {
        let _guard = crate::native::test_path_guard();
        let stub = stub_pkg_config("sys", "1.2.13");
        let _path = prepend_path(&stub);
        let (_dir, root) = native_tree("sys", "        zlib: { native: \"z\", system: true, version: \">=1.0.0\" }\n", "");
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock =
            ProjectDepLock::resolve_with(&root, &cfg, crate::native::NativePrefer::None).unwrap();
        let zlib = lock.packages.iter().find(|p| p.name == "zlib").unwrap();
        assert_eq!(zlib.source, "native:z+system");
        assert_eq!(zlib.version, "1.2.13");
        verify_lock_with(&root, &cfg, &lock, crate::native::NativePrefer::None).unwrap();
    }

    #[test]
    fn native_system_mismatch_fails_closed_naming_versions() {
        let _guard = crate::native::test_path_guard();
        let stub = stub_pkg_config("mismatch", "1.2.13");
        let _path = prepend_path(&stub);
        let (_dir, root) = native_tree("mismatch", "        zlib: { native: \"z\", system: true, version: \">=9.0.0\" }\n", "");
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let err = ProjectDepLock::resolve_with(&root, &cfg, crate::native::NativePrefer::None).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("1.2.13"), "{}", err.message);
        assert!(err.message.contains(">=9.0"), "{}", err.message);
    }

    #[test]
    fn native_prefer_bundled_overrides_system_declaration() {
        let _guard = crate::native::test_path_guard();
        let stub = stub_pkg_config("prefer", "1.2.13");
        let _path = prepend_path(&stub);
        let (_dir, root) = native_tree("prefer", "        zlib: { native: \"z\", system: true }\n", "");
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock =
            ProjectDepLock::resolve_with(&root, &cfg, crate::native::NativePrefer::Bundled).unwrap();
        let zlib = lock.packages.iter().find(|p| p.name == "zlib").unwrap();
        assert_eq!(zlib.source, "native:z");
        let err =
            verify_lock_with(&root, &cfg, &lock, crate::native::NativePrefer::None).unwrap_err();
        assert!(err.message.contains("source mismatch"), "{}", err.message);
    }

    #[test]
    fn native_pin_repins_transitive_source_and_rejects_strays() {
        let _guard = crate::native::test_path_guard();
        let stub = stub_pkg_config("pin", "3.0.0");
        let _path = prepend_path(&stub);
        let (_dir, root) = native_tree(
            "pin",
            "        zlib: { native: \"z\", system: true }\n",
            ",\n    native: { zlib: { system: false } }\n",
        );
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock =
            ProjectDepLock::resolve_with(&root, &cfg, crate::native::NativePrefer::None).unwrap();
        let zlib = lock.packages.iter().find(|p| p.name == "zlib").unwrap();
        assert_eq!(zlib.source, "native:z");
        let (_dir2, root2) = native_tree(
            "pin-stray",
            "        zlib: { native: \"z\", system: true }\n",
            ",\n    native: { nosuch: { system: true } }\n",
        );
        let cfg2 = ProjectConfig::load_from_dir(&root2).unwrap().unwrap();
        let err = ProjectDepLock::resolve_with(&root2, &cfg2, crate::native::NativePrefer::None).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("nosuch"), "{}", err.message);
    }

    #[test]
    fn native_from_records_title_choice() {
        let _guard = crate::native::test_path_guard();
        let (_dir, root) = native_tree(
            "from",
            "        codec: { native: \"codec\", from: \"libcodec.so\" }\n",
            ",\n    artifacts: \"ghcr.io/org/demo\"\n",
        );
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock =
            ProjectDepLock::resolve_with(&root, &cfg, crate::native::NativePrefer::None).unwrap();
        let codec = lock.packages.iter().find(|p| p.name == "codec").unwrap();
        assert_eq!(codec.source, "native:codec#libcodec.so");
        verify_lock_with(&root, &cfg, &lock, crate::native::NativePrefer::None).unwrap();
    }

    fn update_app(
        tag: &str,
        deps: &str,
        extra: &str,
        registry: &str,
    ) -> (crate::scratch::Scratch, std::path::PathBuf) {
        let dir = crate::scratch::dir(&format!("update-{tag}"));
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n{deps}    }}{extra},\n    registry: {{ url: \"{registry}\" }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(
            root.join("src").join("main.rnx"),
            "fn Main(): Int { return 0; }\n",
        )
        .unwrap();
        (dir, root)
    }

    fn update_point_registry(root: &std::path::Path, deps: &str, extra: &str, registry: &str) {
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n{deps}    }}{extra},\n    registry: {{ url: \"{registry}\" }}\n}}\n"
            ),
        )
        .unwrap();
    }

    fn update_mock(
        version: &str,
        full: &str,
        engine: &str,
    ) -> (String, Vec<(String, Vec<u8>)>, String, String) {
        let n: u32 = version.bytes().map(u32::from).sum();
        let main = format!("export fn hello(): Int {{ return {n}; }}\n").into_bytes();
        let (manifest, pairs, digest, _, _) =
            crate::fetch::testkit::fixture_tree_with_main(full, version, &main);
        let node = if engine.is_empty() {
            crate::fetch::testkit::node_json(full, version, &digest, false)
        } else {
            crate::fetch::testkit::node_engine_json(full, version, &digest, false, engine)
        };
        (manifest, pairs, digest, node)
    }

    #[test]
    fn update_selector_parsing() {
        let s = parse_update_selector("widget").unwrap();
        assert_eq!(s.name, "widget");
        assert_eq!(s.selector, None);
        let s = parse_update_selector("@acme/widget").unwrap();
        assert_eq!(s.name, "@acme/widget");
        assert_eq!(s.selector, None);
        let s = parse_update_selector("widget@^1.0.0").unwrap();
        assert_eq!(s.name, "widget");
        assert_eq!(s.selector.as_deref(), Some("^1.0.0"));
        let s = parse_update_selector("@acme/widget@1.2.3").unwrap();
        assert_eq!(s.name, "@acme/widget");
        assert_eq!(s.selector.as_deref(), Some("1.2.3"));
        let s = parse_update_selector("widget@latest").unwrap();
        assert_eq!(s.selector.as_deref(), Some("latest"));
        for bad in ["", "@", "widget@", "@widget@"] {
            assert!(parse_update_selector(bad).is_err(), "{bad}");
        }
    }

    #[test]
    fn update_rewrites_lock_with_new_digest() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-rewrite");
        let (manifest_a, pairs_a, digest_a, node_a) = update_mock("1.2.0", "@acme/widget", "");
        let (manifest_b, pairs_b, digest_b, node_b) = update_mock("1.3.0", "@acme/widget", "");
        assert_ne!(digest_a, digest_b);
        let server_a =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_a]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_a),
                chunk_override: Some(pairs_a),
            });
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" }\n";
        let (_dir, root) = update_app("rewrite", deps, "", &server_a.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let old = ProjectDepLock::resolve(&root, &cfg).unwrap();
        old.write(&root).unwrap();
        let server_b =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_b]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_b),
                chunk_override: Some(pairs_b),
            });
        update_point_registry(&root, deps, "", &server_b.base);
        let cfg_b = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let (new_lock, summary) = ProjectDepLock::update_lock(
            &root,
            &cfg_b,
            crate::native::NativePrefer::None,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(summary.updated.len(), 1);
        assert_eq!(summary.updated[0].name, "@acme/widget");
        assert_eq!(summary.updated[0].old, "1.2.0");
        assert_eq!(summary.updated[0].new, "1.3.0");
        assert!(summary.held.is_empty());
        let widget = new_lock
            .packages
            .iter()
            .find(|p| p.name == "@acme/widget")
            .unwrap();
        assert_eq!(widget.version, "1.3.0");
        assert_eq!(widget.source, "registry:@acme/widget@1.3.0");
        assert_eq!(widget.checksum, digest_b);
        verify_lock(&root, &cfg_b, &new_lock).unwrap();
    }

    #[test]
    fn update_noop_when_current() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-noop");
        let (manifest, pairs, digest, node) = update_mock("1.2.0", "@acme/widget", "");
        let server =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest),
                chunk_override: Some(pairs),
            });
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" }\n";
        let (_dir, root) = update_app("noop", deps, "", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let old = ProjectDepLock::resolve(&root, &cfg).unwrap();
        old.write(&root).unwrap();
        assert_eq!(
            old.packages
                .iter()
                .find(|p| p.name == "@acme/widget")
                .unwrap()
                .checksum,
            digest
        );
        let (new_lock, summary) =
            ProjectDepLock::update_lock(&root, &cfg, crate::native::NativePrefer::None, &[], false)
                .unwrap();
        assert!(summary.updated.is_empty());
        assert!(summary.held.is_empty());
        assert_eq!(new_lock.to_rnx(), old.to_rnx());
    }

    fn dir_snapshot(root: &std::path::Path) -> Vec<(String, Vec<u8>)> {
        let mut out = Vec::new();
        let mut stack = vec![root.to_path_buf()];
        while let Some(dir) = stack.pop() {
            let mut entries: Vec<std::path::PathBuf> =
                std::fs::read_dir(&dir).unwrap().map(|e| e.unwrap().path()).collect();
            entries.sort();
            for path in entries {
                if path.is_dir() {
                    stack.push(path);
                } else {
                    let rel = path.strip_prefix(root).unwrap().to_string_lossy().into_owned();
                    out.push((rel, std::fs::read(&path).unwrap()));
                }
            }
        }
        out.sort();
        out
    }

    fn offline_registry_app(tag: &str) -> (crate::scratch::Scratch, std::path::PathBuf) {
        let dead = "http://127.0.0.1:9";
        let dir = crate::scratch::dir(&format!("verify-offline-{tag}"));
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"app\",\n        version: \"0.1.0\"\n    }},\n    dependencies: {{\n        \"@acme/widget\": {{ version: \"^1.0.0\" }}\n    }},\n    registry: {{ url: \"{dead}\" }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(root.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        (dir, root)
    }

    fn plant_widget_cache(integrity: &str) {
        let dead = "http://127.0.0.1:9";
        let cached = crate::fetch::cached_package_dir(dead, "@acme/widget", "1.2.0");
        std::fs::create_dir_all(cached.join("src")).unwrap();
        std::fs::write(
            cached.join(crate::project::MANIFEST_FILE),
            "export default {\n    project: {\n        name: \"@acme/widget\",\n        version: \"1.2.0\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(
            cached.join("src").join("main.rnx"),
            "export fn hello(): Int { return 1; }\n",
        )
        .unwrap();
        std::fs::write(cached.join(".rnx-integrity"), format!("@acme/widget\n1.2.0\n{integrity}\n"))
            .unwrap();
    }

    fn offline_lock(root: &std::path::Path, integrity: &str) -> ProjectDepLock {
        let cached = crate::fetch::cached_package_dir("http://127.0.0.1:9", "@acme/widget", "1.2.0");
        let analysis = crate::security::analyze_dep_dir(&cached).unwrap();
        let widget = LockedPackage {
            name: "@acme/widget".to_string(),
            version: "1.2.0".to_string(),
            source: "registry:@acme/widget@1.2.0".to_string(),
            checksum: integrity.to_string(),
            artifact_digest: None,
            license: Some("unknown".to_string()),
            link: Some("static".to_string()),
            dependencies: Vec::new(),
            tier: analysis.tier.to_string(),
            capabilities: analysis.capabilities.clone(),
        };
        let app = LockedPackage {
            name: "app".to_string(),
            version: "0.1.0".to_string(),
            source: "root".to_string(),
            checksum: crate::checksum::compute_package_checksum(root).unwrap(),
            artifact_digest: None,
            license: Some("unknown".to_string()),
            link: Some("static".to_string()),
            dependencies: vec!["@acme/widget 1.2.0".to_string()],
            tier: "pure".to_string(),
            capabilities: Vec::new(),
        };
        ProjectDepLock { version: LOCK_VERSION, packages: vec![app, widget] }
    }

    #[test]
    fn verify_uses_locked_pins_offline() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("verify-pins");
        let (_dir, root) = offline_registry_app("pins");
        let integrity = "ab".repeat(32);
        plant_widget_cache(&integrity);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = offline_lock(&root, &integrity);
        verify_lock(&root, &cfg, &lock).unwrap();
    }

    #[test]
    fn verify_never_writes_the_deplock() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("verify-write");
        let (_dir, root) = offline_registry_app("write");
        let integrity = "ab".repeat(32);
        plant_widget_cache(&integrity);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let lock = offline_lock(&root, &integrity);
        lock.write(&root).unwrap();
        let before = dir_snapshot(&root);
        verify_lock(&root, &cfg, &lock).unwrap();
        assert_eq!(dir_snapshot(&root), before, "verify must never write the deplock");
    }

    #[test]
    fn update_named_pins_other_direct_deps() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-named");
        let (pkg_wa, digest_wa) = crate::fetch::testkit::routed_fixture("@acme/widget", "1.2.0", 0);
        let (pkg_ga, digest_ga) = crate::fetch::testkit::routed_fixture("@acme/gadget", "2.0.0", 0);
        let node_wa = crate::fetch::testkit::node_json("@acme/widget", "1.2.0", &digest_wa, false);
        let node_ga = crate::fetch::testkit::node_json("@acme/gadget", "2.0.0", &digest_ga, false);
        let server_a = crate::fetch::testkit::MockRegistry::start_routed(
            crate::fetch::testkit::RoutedConfig {
                version_spec: 0,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_wa, node_ga]),
                packages: vec![pkg_wa, pkg_ga],
            },
        );
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" },\n        \"@acme/gadget\": { version: \"^2.0.0\" }\n";
        let (_dir, root) = update_app("named", deps, "", &server_a.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let old = ProjectDepLock::resolve(&root, &cfg).unwrap();
        old.write(&root).unwrap();
        let (pkg_wb, digest_wb) = crate::fetch::testkit::routed_fixture("@acme/widget", "1.3.0", 0);
        let (pkg_gb, digest_gb) = crate::fetch::testkit::routed_fixture("@acme/gadget", "2.0.0", 0);
        let node_wb = crate::fetch::testkit::node_json("@acme/widget", "1.3.0", &digest_wb, false);
        let node_gb = crate::fetch::testkit::node_json("@acme/gadget", "2.0.0", &digest_gb, false);
        let server_b = crate::fetch::testkit::MockRegistry::start_routed(
            crate::fetch::testkit::RoutedConfig {
                version_spec: 0,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_wb, node_gb]),
                packages: vec![pkg_wb, pkg_gb],
            },
        );
        update_point_registry(&root, deps, "", &server_b.base);
        let cfg_b = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let only_widget = vec![parse_update_selector("@acme/widget").unwrap()];
        let (new_lock, summary) = ProjectDepLock::update_lock(
            &root,
            &cfg_b,
            crate::native::NativePrefer::None,
            &only_widget,
            false,
        )
        .unwrap();
        assert_eq!(summary.updated.len(), 1);
        assert_eq!(summary.updated[0].name, "@acme/widget");
        assert_eq!(summary.updated[0].old, "1.2.0");
        assert_eq!(summary.updated[0].new, "1.3.0");
        let gadget = new_lock
            .packages
            .iter()
            .find(|p| p.name == "@acme/gadget")
            .unwrap();
        assert_eq!(gadget.version, "2.0.0");
        let widget = new_lock
            .packages
            .iter()
            .find(|p| p.name == "@acme/widget")
            .unwrap();
        assert_eq!(widget.checksum, digest_wb);
        let bodies = server_b.resolve_bodies();
        assert!(!bodies.is_empty());
        assert!(
            bodies
                .iter()
                .any(|b| crate::fetch::testkit::requirement(b, "@acme/widget") == "^1.0.0"),
            "{bodies:?}"
        );
        assert!(
            bodies
                .iter()
                .any(|b| crate::fetch::testkit::requirement(b, "@acme/gadget") == "2.0.0"),
            "{bodies:?}"
        );
    }

    #[test]
    fn update_latest_selector_pins_digest_not_tag() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-latest");
        let (manifest_a, pairs_a, _, node_a) = update_mock("1.2.0", "@acme/widget", "");
        let server_a =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_a]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_a),
                chunk_override: Some(pairs_a),
            });
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" }\n";
        let (_dir, root) = update_app("latest", deps, "", &server_a.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        ProjectDepLock::resolve(&root, &cfg)
            .unwrap()
            .write(&root)
            .unwrap();
        let (manifest_b, pairs_b, digest_b, node_b) = update_mock("1.9.0", "@acme/widget", "");
        let server_b =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_b]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_b),
                chunk_override: Some(pairs_b),
            });
        update_point_registry(&root, deps, "", &server_b.base);
        let cfg_b = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let sel = vec![parse_update_selector("@acme/widget@latest").unwrap()];
        let (new_lock, summary) = ProjectDepLock::update_lock(
            &root,
            &cfg_b,
            crate::native::NativePrefer::None,
            &sel,
            false,
        )
        .unwrap();
        assert_eq!(summary.updated.len(), 1);
        assert_eq!(summary.updated[0].new, "1.9.0");
        let widget = new_lock
            .packages
            .iter()
            .find(|p| p.name == "@acme/widget")
            .unwrap();
        assert_eq!(widget.source, "registry:@acme/widget@1.9.0");
        assert_eq!(widget.checksum, digest_b);
        assert!(!widget.source.contains("latest"));
        let bodies = server_b.resolve_bodies();
        let last = bodies.last().unwrap();
        assert_eq!(
            crate::fetch::testkit::requirement(last, "@acme/widget"),
            "latest"
        );
    }

    #[test]
    fn update_holds_major_without_flag() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-major");
        let (manifest_a, pairs_a, _, node_a) = update_mock("1.2.0", "@acme/widget", "");
        let server_a =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_a]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_a),
                chunk_override: Some(pairs_a),
            });
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" }\n";
        let (_dir, root) = update_app("major", deps, "", &server_a.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        ProjectDepLock::resolve(&root, &cfg)
            .unwrap()
            .write(&root)
            .unwrap();
        let (manifest_old, pairs_old, digest_old, _) = update_mock("1.2.0", "@acme/widget", "");
        let (manifest_new, pairs_new, digest_new, _) = update_mock("2.0.0", "@acme/widget", "");
        let node_old =
            crate::fetch::testkit::node_json("@acme/widget", "1.2.0", &digest_old, false);
        let node_new =
            crate::fetch::testkit::node_json("@acme/widget", "2.0.0", &digest_new, false);
        let body_old = crate::fetch::testkit::resolve_json_static(&[node_old]);
        let body_new = crate::fetch::testkit::resolve_json_static(&[node_new]);
        let mut chunks = std::collections::BTreeMap::new();
        chunks.insert("@acme/widget@1.2.0".to_string(), (manifest_old, pairs_old));
        chunks.insert("@acme/widget@2.0.0".to_string(), (manifest_new, pairs_new));
        let server_b = crate::fetch::testkit::MockRegistry::start_dynamic(chunks, move |reqs| {
            if reqs.get("@acme/widget").map(String::as_str) == Some("1.2.0") {
                body_old.clone()
            } else {
                body_new.clone()
            }
        });
        update_point_registry(&root, deps, "", &server_b.base);
        let cfg_b = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let (held_lock, held_summary) = ProjectDepLock::update_lock(
            &root,
            &cfg_b,
            crate::native::NativePrefer::None,
            &[],
            false,
        )
        .unwrap();
        assert!(held_summary.updated.is_empty());
        assert_eq!(held_summary.held.len(), 1);
        assert_eq!(held_summary.held[0].old, "1.2.0");
        assert_eq!(held_summary.held[0].new, "2.0.0");
        assert_eq!(
            held_lock
                .packages
                .iter()
                .find(|p| p.name == "@acme/widget")
                .unwrap()
                .version,
            "1.2.0"
        );
        let (moved_lock, moved_summary) = ProjectDepLock::update_lock(
            &root,
            &cfg_b,
            crate::native::NativePrefer::None,
            &[],
            true,
        )
        .unwrap();
        assert_eq!(moved_summary.updated.len(), 1);
        assert_eq!(moved_summary.updated[0].new, "2.0.0");
        assert!(moved_summary.held.is_empty());
        assert_eq!(
            moved_lock
                .packages
                .iter()
                .find(|p| p.name == "@acme/widget")
                .unwrap()
                .version,
            "2.0.0"
        );
    }

    #[test]
    fn update_rejects_unknown_tag_and_unknown_package() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-reject");
        let (manifest, pairs, _, node) = update_mock("1.2.0", "@acme/widget", "");
        let server =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest),
                chunk_override: Some(pairs),
            });
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" }\n";
        let (_dir, root) = update_app("reject", deps, "", &server.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        ProjectDepLock::resolve(&root, &cfg)
            .unwrap()
            .write(&root)
            .unwrap();
        let tag = vec![parse_update_selector("@acme/widget@beta").unwrap()];
        let err = ProjectDepLock::update_lock(
            &root,
            &cfg,
            crate::native::NativePrefer::None,
            &tag,
            false,
        )
        .unwrap_err();
        assert!(err.message.contains("not a valid range"), "{}", err.message);
        assert!(err.message.contains("dist-tags"), "{}", err.message);
        let unknown = vec![parse_update_selector("nosuch").unwrap()];
        let err = ProjectDepLock::update_lock(
            &root,
            &cfg,
            crate::native::NativePrefer::None,
            &unknown,
            false,
        )
        .unwrap_err();
        assert!(
            err.message.contains("not a registry dependency"),
            "{}",
            err.message
        );
    }

    #[test]
    fn update_fails_closed_on_engine_mismatch() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-engine");
        let (manifest_a, pairs_a, _, node_a) = update_mock("1.2.0", "@acme/widget", "");
        let server_a =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_a]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_a),
                chunk_override: Some(pairs_a),
            });
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" }\n";
        let (_dir, root) = update_app("engine", deps, "", &server_a.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        ProjectDepLock::resolve(&root, &cfg)
            .unwrap()
            .write(&root)
            .unwrap();
        let (manifest_b, pairs_b, _, node_b) = update_mock("1.3.0", "@acme/widget", ">=99.0.0");
        let server_b =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_b]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_b),
                chunk_override: Some(pairs_b),
            });
        update_point_registry(&root, deps, "", &server_b.base);
        let cfg_b = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let err = ProjectDepLock::update_lock(
            &root,
            &cfg_b,
            crate::native::NativePrefer::None,
            &[],
            false,
        )
        .unwrap_err();
        assert!(err.message.contains("needs engine"), "{}", err.message);
    }

    #[test]
    fn update_keeps_consumer_native_override() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("update-native");
        let (manifest_a, pairs_a, _, node_a) = update_mock("1.2.0", "@acme/widget", "");
        let server_a =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_a]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_a),
                chunk_override: Some(pairs_a),
            });
        let deps = "        \"@acme/widget\": { version: \"^1.0.0\" },\n        zlib: { native: \"z\", system: true }\n";
        let extra = ",\n    native: { zlib: { system: false } }";
        let (_dir, root) = update_app("native", deps, extra, &server_a.base);
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let old =
            ProjectDepLock::resolve_with(&root, &cfg, crate::native::NativePrefer::None).unwrap();
        assert_eq!(
            old.packages
                .iter()
                .find(|p| p.name == "zlib")
                .unwrap()
                .source,
            "native:z"
        );
        old.write(&root).unwrap();
        let (manifest_b, pairs_b, _, node_b) = update_mock("1.3.0", "@acme/widget", "");
        let server_b =
            crate::fetch::testkit::MockRegistry::start(crate::fetch::testkit::MockConfig {
                version_spec: 0,
                capabilities: vec!["tombstones"],
                resolve_status: 200,
                resolve_body: crate::fetch::testkit::resolve_json_static(&[node_b]),
                manifest_status: 200,
                fallback_tarball: Vec::new(),
                manifest_error: String::new(),
                manifest_override: Some(manifest_b),
                chunk_override: Some(pairs_b),
            });
        update_point_registry(&root, deps, extra, &server_b.base);
        let cfg_b = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let (new_lock, summary) = ProjectDepLock::update_lock(
            &root,
            &cfg_b,
            crate::native::NativePrefer::None,
            &[],
            false,
        )
        .unwrap();
        assert_eq!(summary.updated.len(), 1);
        assert_eq!(
            new_lock
                .packages
                .iter()
                .find(|p| p.name == "zlib")
                .unwrap()
                .source,
            "native:z"
        );
        verify_lock_with(&root, &cfg_b, &new_lock, crate::native::NativePrefer::None).unwrap();
    }

    #[test]
    fn update_needs_lockfile() {
        let dir = crate::scratch::dir("update-nolock");
        let root = dir.join("app");
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::write(
            root.join("Project.config"),
            "export default {\n    project: {\n        name: \"app\",\n        version: \"0.1.0\"\n    }\n}\n",
        )
        .unwrap();
        let cfg = ProjectConfig::load_from_dir(&root).unwrap().unwrap();
        let err =
            ProjectDepLock::update_lock(&root, &cfg, crate::native::NativePrefer::None, &[], false)
                .unwrap_err();
        assert!(err.message.contains("run 'rnx lock'"), "{}", err.message);
    }
}
