use crate::capabilities::{self, CallChain, Capability, CapabilityTier};
use crate::deplock::ProjectDepLock;
use crate::modules::ModuleGraph;
use crate::project::{self, DependencySpec, ProjectConfig};
use diagnostics::{Code, Diagnostic};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};
use std::str::FromStr;

pub(crate) fn tier_rank(tier: &str) -> Option<u8> {
    Some(match tier {
        "pure" => 0,
        "delegated" => 1,
        "ambient" => 2,
        "hazard" => 3,
        _ => return None,
    })
}

pub(crate) fn tier_of_caps(caps: &BTreeSet<Capability>) -> CapabilityTier {
    caps.iter().map(|c| c.tier()).max().unwrap_or(CapabilityTier::Pure)
}

pub(crate) fn trace_loc(chain: &CallChain) -> String {
    match chain.nodes.last() {
        Some(n) => format!("{}:{}:{}: {}", n.file, n.line, n.col, n.expression_snippet),
        None => "unknown location".to_string(),
    }
}

pub(crate) fn first_trace<'a>(traces: &'a [CallChain], cap: &Capability) -> Option<&'a CallChain> {
    traces.iter().find(|t| &t.capability == cap)
}

pub(crate) fn parse_cap(s: &str) -> Result<Capability, Diagnostic> {
    Capability::from_str(s).map_err(|e| {
        if e.starts_with("unknown capability") {
            Diagnostic::new(Code::E108, format!("unrecognized capability `{s}` in manifest"))
        } else {
            Diagnostic::new(
                Code::S102,
                format!("malformed capability `{s}` in manifest: {e}"),
            )
        }
    })
}

pub struct DepDir {
    pub name: String,
    pub dir: PathBuf,
}

fn dep_dirs(root: &Path, config: &ProjectConfig) -> Vec<DepDir> {
    let mut out = Vec::new();
    for (name, spec) in &config.dependencies {
        let dir = match spec {
            DependencySpec::Path { path } => Some(root.join(path)),
            DependencySpec::Git { .. } => {
                let vendor = root.join("vendor").join(name);
                if vendor.join("Project.config").is_file() {
                    Some(vendor)
                } else {
                    git_cache_dir(root, name)
                }
            }
            DependencySpec::Semver { .. } | DependencySpec::Url { .. } => None,
            DependencySpec::Native { path, .. } => path.as_ref().map(|p| root.join(p)),
        };
        if let Some(dir) = dir {
            let canon = std::fs::canonicalize(&dir).unwrap_or(dir);
            out.push(DepDir {
                name: name.clone(),
                dir: canon,
            });
        }
    }
    out.sort_by(|a, b| b.dir.as_os_str().len().cmp(&a.dir.as_os_str().len()));
    out
}

fn git_cache_dir(root: &Path, name: &str) -> Option<PathBuf> {
    let cache = root.join(".rnx-cache").join("cache").join("git");
    let entries = std::fs::read_dir(&cache).ok()?;
    let prefix = format!("{name}-");
    let mut hits: Vec<PathBuf> = entries
        .filter_map(|e| e.ok())
        .map(|e| e.path())
        .filter(|p| {
            p.file_name()
                .is_some_and(|n| n.to_string_lossy().starts_with(&prefix))
        })
        .collect();
    hits.sort();
    hits.into_iter().next()
}

fn is_std_path(path: &Path) -> bool {
    path.to_string_lossy().starts_with("@std/")
}

pub fn verify_entry(entry: &Path) -> Vec<Diagnostic> {
    let start = if entry.is_file() {
        entry.parent().unwrap_or(Path::new(".")).to_path_buf()
    } else {
        entry.to_path_buf()
    };
    let Some(proj_root) = project::find_project_root(&start) else {
        return Vec::new();
    };
    let boundary = project::find_workspace_root(&start).unwrap_or_else(|| proj_root.clone());
    let manifest = match project::load_manifest(&proj_root) {
        Ok(Some(m)) => m,
        _ => return Vec::new(),
    };
    let Some(config) = manifest.project else {
        return Vec::new();
    };
    let lock = match ProjectDepLock::load(&boundary) {
        Ok(l) => l,
        Err(_) => return Vec::new(),
    };
    let graph = match ModuleGraph::build(entry) {
        Ok(g) => g,
        Err(_) => return Vec::new(),
    };
    verify_graph(&boundary, &proj_root, &config, lock.as_ref(), &graph)
}

fn verify_graph(
    boundary: &Path,
    proj_root: &Path,
    config: &ProjectConfig,
    lock: Option<&ProjectDepLock>,
    graph: &ModuleGraph,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let boundary_canon = std::fs::canonicalize(boundary).unwrap_or_else(|_| boundary.to_path_buf());
    let proj_canon = std::fs::canonicalize(proj_root).unwrap_or_else(|_| proj_root.to_path_buf());
    let deps = dep_dirs(boundary, config);
    let mut local_caps: BTreeSet<Capability> = BTreeSet::new();
    let mut local_traces: Vec<CallChain> = Vec::new();
    let mut root_caps: BTreeSet<Capability> = BTreeSet::new();
    let mut root_traces: Vec<CallChain> = Vec::new();
    let mut dep_caps: BTreeMap<String, BTreeSet<Capability>> = BTreeMap::new();
    let mut dep_traces: BTreeMap<String, Vec<CallChain>> = BTreeMap::new();

    for f in &graph.files {
        if is_std_path(&f.path) {
            continue;
        }
        let src = match std::fs::read_to_string(&f.path) {
            Ok(s) => s,
            Err(_) => continue,
        };
        let display = f.path.to_string_lossy().into_owned();
        let report = match capabilities::analyze(&f.module, &src, &display) {
            Ok(r) => r,
            Err(e) => {
                let span = diagnostics::Span { start: 0, end: 0 };
                diags.push(e.to_diagnostic(span).with_file(f.path.clone()));
                continue;
            }
        };
        if report.capabilities.is_empty() {
            continue;
        }
        let canon = std::fs::canonicalize(&f.path).unwrap_or_else(|_| f.path.clone());
        if canon.starts_with(&boundary_canon) {
            local_caps.extend(report.capabilities.iter().cloned());
            local_traces.extend(report.traces.clone());
            if canon.starts_with(&proj_canon) {
                root_caps.extend(report.capabilities.iter().cloned());
                root_traces.extend(report.traces);
            }
            continue;
        }
        let owner = deps
            .iter()
            .find(|d| canon.starts_with(&d.dir))
            .map(|d| d.name.clone());
        match owner {
            Some(name) => {
                dep_caps
                    .entry(name.clone())
                    .or_default()
                    .extend(report.capabilities.iter().cloned());
                dep_traces.entry(name).or_default().extend(report.traces);
            }
            None => continue,
        }
    }

    let locked: BTreeMap<&str, &crate::deplock::LockedPackage> = match lock {
        Some(l) => l.packages.iter().map(|p| (p.name.as_str(), p)).collect(),
        None => BTreeMap::new(),
    };

    for (name, deduced) in &dep_caps {
        let traces = &dep_traces[name.as_str()];
        match locked.get(name.as_str()) {
            Some(entry) => {
                let loc = |cap: &Capability| {
                    first_trace(traces, cap)
                        .map(trace_loc)
                        .unwrap_or_else(|| "unknown location".to_string())
                };
                diags.extend(entry_scan_diags(name, deduced, entry, &loc));
            }
            None => {
                if lock.is_some() && !deduced.is_empty() {
                    let mut all: Vec<&Capability> = deduced.iter().collect();
                    all.sort();
                    let list = all
                        .iter()
                        .map(|c| format!("`{c}`"))
                        .collect::<Vec<_>>()
                        .join(", ");
                    diags.push(Diagnostic::new(
                        Code::S101,
                        format!("package `{name}` is not in Project.deplock and requires {list}"),
                    ));
                }
            }
        }
    }

    match locked.get(config.name.as_str()) {
        Some(entry) => {
            let loc = |cap: &Capability| {
                first_trace(&root_traces, cap)
                    .map(trace_loc)
                    .unwrap_or_else(|| "unknown location".to_string())
            };
            diags.extend(entry_scan_diags(&config.name, &root_caps, entry, &loc));
        }
        None => {
            if lock.is_some() && !root_caps.is_empty() {
                let mut all: Vec<&Capability> = root_caps.iter().collect();
                all.sort();
                let list = all
                    .iter()
                    .map(|c| format!("`{c}`"))
                    .collect::<Vec<_>>()
                    .join(", ");
                diags.push(Diagnostic::new(
                    Code::S101,
                    format!(
                        "package `{}` is not in Project.deplock and requires {list}",
                        config.name,
                    ),
                ));
            }
        }
    }

    if let Some(allowed) = &config.permissions {
        let mut ceiling = BTreeSet::new();
        for decl in allowed {
            match parse_cap(&decl.perm) {
                Ok(c) => {
                    ceiling.insert(c);
                }
                Err(d) => diags.push(d),
            }
        }
        let mut union: BTreeMap<&Capability, &CallChain> = BTreeMap::new();
        for t in local_traces.iter() {
            union.entry(&t.capability).or_insert(t);
        }
        for traces in dep_traces.values() {
            for t in traces {
                union.entry(&t.capability).or_insert(t);
            }
        }
        let mut over: Vec<(&Capability, &CallChain)> = union
            .into_iter()
            .filter(|(c, _)| !ceiling.contains(*c))
            .collect();
        over.sort_by(|a, b| a.0.cmp(b.0));
        for (cap, trace) in over {
            diags.push(Diagnostic::new(
                Code::S102,
                format!(
                    "capability `{cap}` exceeds the `[permissions]` ceiling at {}",
                    trace_loc(trace),
                ),
            ));
        }
    }

    diags
}

pub(crate) fn entry_scan_diags(
    name: &str,
    deduced: &BTreeSet<Capability>,
    entry: &crate::deplock::LockedPackage,
    loc: &dyn Fn(&Capability) -> String,
) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut approved = BTreeSet::new();
    for s in &entry.capabilities {
        match parse_cap(s) {
            Ok(c) => {
                approved.insert(c);
            }
            Err(d) => diags.push(d),
        }
    }
    let mut excess: Vec<&Capability> = deduced.difference(&approved).collect();
    excess.sort();
    for cap in excess {
        diags.push(Diagnostic::new(
            Code::S101,
            format!(
                "package `{name}` requires unapproved capability `{cap}` (locked: [{}]) at {}",
                entry.capabilities.join(", "),
                loc(cap),
            ),
        ));
    }
    let deduced_tier = tier_of_caps(deduced);
    let locked_rank = tier_rank(&entry.tier);
    let deduced_rank = tier_rank(&deduced_tier.to_string());
    match (locked_rank, deduced_rank) {
        (Some(l), Some(d)) if d > l => diags.push(Diagnostic::new(
            Code::S101,
            format!(
                "package `{name}` escalated to tier `{deduced_tier}` (locked: `{}`)",
                entry.tier,
            ),
        )),
        (None, _) => diags.push(Diagnostic::new(
            Code::E108,
            format!("locked package `{name}` has unknown tier `{}`", entry.tier,),
        )),
        _ => {}
    }
    diags
}

pub struct DepAnalysis {
    pub name: String,
    pub version: String,
    pub tier: CapabilityTier,
    pub capabilities: Vec<String>,
    pub traces: Vec<CallChain>,
}

pub fn analyze_dep_dir(dir: &Path) -> Result<DepAnalysis, Diagnostic> {
    let manifest = project::load_manifest(dir).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("cannot read dependency manifest in `{}`: {e}", dir.display()),
        )
    })?;
    let Some(config) = manifest.and_then(|m| m.project) else {
        return Err(Diagnostic::new(
            Code::E108,
            format!("dependency `{}` has no [project] section", dir.display()),
        ));
    };
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rnx(dir, &mut files);
    files.sort();
    let mut caps: BTreeSet<Capability> = BTreeSet::new();
    let mut traces: Vec<CallChain> = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display()))
        })?;
        let module = crate::parser::Parser::parse_module(&src).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot parse `{}`: {e}", path.display()))
        })?;
        let display = path.to_string_lossy().into_owned();
        match capabilities::analyze(&module, &src, &display) {
            Ok(r) => {
                caps.extend(r.capabilities);
                traces.extend(r.traces);
            }
            Err(e) => {
                let span = diagnostics::Span { start: 0, end: 0 };
                let mut d = e.to_diagnostic(span);
                d.message = format!("{}: {}", path.display(), d.message);
                d.file = Some(path.clone());
                return Err(d);
            }
        }
    }
    let tier = tier_of_caps(&caps);
    let mut capabilities: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
    capabilities.sort();
    Ok(DepAnalysis {
        name: config.name,
        version: config.version,
        tier,
        capabilities,
        traces,
    })
}

fn collect_rnx(dir: &Path, out: &mut Vec<PathBuf>) {
    let entries = match std::fs::read_dir(dir) {
        Ok(e) => e,
        Err(_) => return,
    };
    for entry in entries.filter_map(|e| e.ok()) {
        let path = entry.path();
        let name = path
            .file_name()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        if name.starts_with('.') || name == "target" || name == "tests" || name == ".git" {
            continue;
        }
        if path.is_dir() {
            collect_rnx(&path, out);
        } else if path.extension().is_some_and(|e| e == "rnx") {
            out.push(path);
        }
    }
}
