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
                    let (dep_root, dep_cfg, resolved) =
                        crate::fetch::resolve_registry_package(
                            dep_name,
                            version,
                            cfg.registry.as_ref(),
                            &cfg.registries,
                            None,
                            have,
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
) -> Result<Vec<PkgNode>, Diagnostic> {
    let mut nodes: BTreeMap<String, PkgNode> = BTreeMap::new();
    let mut used_pins: BTreeSet<String> = BTreeSet::new();
    collect_into(&mut nodes, root, root, config, true, have, &config.native_pins, prefer, &mut used_pins)?;
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
) -> Result<Vec<PkgNode>, Diagnostic> {
    let ws_pins = ProjectConfig::load_from_dir(ws_root)
        .ok()
        .flatten()
        .map(|c| c.native_pins)
        .unwrap_or_default();
    let mut nodes: BTreeMap<String, PkgNode> = BTreeMap::new();
    let mut used_pins: BTreeSet<String> = BTreeSet::new();
    for (root, cfg) in members.values() {
        collect_into(&mut nodes, ws_root, root, cfg, false, have, &ws_pins, prefer, &mut used_pins)?;
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
        let have = existing_have(root);
        nodes_to_lock(collect_graph(root, config, &have, prefer)?)
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
        let have = existing_have(ws_root);
        nodes_to_lock(collect_workspace_graph(ws_root, members, &have, prefer)?)
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
    check_nodes(collect_graph(root, config, &have, prefer)?, lock)
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
    check_nodes(collect_workspace_graph(ws_root, members, &have, prefer)?, lock)
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
}
