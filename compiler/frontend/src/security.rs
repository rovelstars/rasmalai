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

pub fn is_std_package_name(name: &str) -> bool {
    name.starts_with("@std/")
}

pub fn bare_rnx_error(name: &str, span: diagnostics::Span) -> Diagnostic {
    Diagnostic::new(
        Code::S103,
        format!("cannot reference runtime-internal `{name}` outside `@std/*`"),
    )
    .with_span(span)
    .with_hint("call the owning `@std/*` wrapper instead of the `__rnx_*` internal (rule S103)")
}

fn nearest_manifest_name(path: &Path) -> Option<String> {
    let mut dir = if path.is_file() {
        path.parent()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    loop {
        if dir.join(crate::project::MANIFEST_FILE).is_file() {
            let cfg = ProjectConfig::load_from_dir(&dir).ok()??;
            return Some(cfg.name);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub fn file_in_std_package(path: &Path, cache: &mut BTreeMap<PathBuf, bool>) -> bool {
    if let Some(hit) = cache.get(path) {
        return *hit;
    }
    let std = path.to_string_lossy().starts_with("@std/")
        || nearest_manifest_name(path).is_some_and(|name| is_std_package_name(&name));
    cache.insert(path.to_path_buf(), std);
    std
}

pub fn s103_in_module(
    module: &crate::ast::Module,
    files: &diagnostics::DeclFiles,
    extra_globals: &BTreeSet<String>,
) -> Vec<Diagnostic> {
    crate::semantic::check_with_files_and_globals(module, files, extra_globals)
        .into_iter()
        .filter(|d| d.code == Code::S103)
        .collect()
}

pub fn s103_frame(modules: &[(&Path, &crate::ast::Module)]) -> Vec<Diagnostic> {
    let mut extra = BTreeSet::new();
    for (_, module) in modules {
        extra.extend(declared_rnx_names(module));
    }
    let mut out = Vec::new();
    for (path, module) in modules {
        for mut diag in s103_in_module(module, &diagnostics::DeclFiles::new(), &extra) {
            if diag.file.is_none() && !path.as_os_str().is_empty() {
                diag.file = Some(path.to_path_buf());
            }
            out.push(diag);
        }
    }
    out.sort_by(|a, b| {
        a.file.cmp(&b.file).then(
            a.span
                .map(|s| s.start)
                .unwrap_or(0)
                .cmp(&b.span.map(|s| s.start).unwrap_or(0)),
        )
    });
    out
}

pub fn s103_graph(graph: &crate::modules::ModuleGraph) -> Vec<Diagnostic> {
    let mut cache: BTreeMap<PathBuf, bool> = BTreeMap::new();
    let mut user: Vec<(PathBuf, crate::ast::Module)> = Vec::new();
    for f in &graph.files {
        if file_in_std_package(&f.path, &mut cache) {
            continue;
        }
        user.push((f.path.clone(), f.module.clone()));
    }
    let refs: Vec<(&Path, &crate::ast::Module)> = user
        .iter()
        .map(|(path, module)| (path.as_path(), module))
        .collect();
    s103_frame(&refs)
}

fn collect_prefix_sources(dir: &Path) -> Result<Vec<(PathBuf, crate::ast::Module)>, Diagnostic> {
    let mut files: Vec<PathBuf> = Vec::new();
    collect_rnx(dir, &mut files);
    files.sort();
    let mut modules = Vec::with_capacity(files.len());
    for path in &files {
        let src = std::fs::read_to_string(path).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display()))
        })?;
        let module = crate::parser::Parser::parse_module(&src).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot parse `{}`: {}", path.display(), e.message),
            )
        })?;
        modules.push((path.clone(), module));
    }
    Ok(modules)
}

pub fn verify_std_package_bytes(dir: &Path) -> Result<bool, Diagnostic> {
    let Some(cfg) = ProjectConfig::load_from_dir(dir).map_err(|e| {
        Diagnostic::new(
            Code::E108,
            format!("cannot read dependency manifest in `{}`: {e}", dir.display()),
        )
    })?
    else {
        return Ok(false);
    };
    if !is_std_package_name(&cfg.name) {
        return Ok(false);
    }
    let Some(pin) = super::stdlib_seed::read_std_pin() else {
        return Ok(false);
    };
    let Some(pinned) = pin.packages.get(&cfg.name) else {
        return Ok(false);
    };
    if pinned != &cfg.version {
        return Ok(false);
    }
    let Some(cached) = super::fetch::find_cached_package_dir(&pin.registry, &cfg.name, pinned)
    else {
        return Ok(false);
    };
    if super::fetch::cached_integrity(&cached).is_none() {
        return Ok(false);
    }
    let mut have: Vec<String> = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(top) = stack.pop() {
        let entries = match std::fs::read_dir(&top) {
            Ok(e) => e,
            Err(_) => return Ok(false),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            let name = path
                .file_name()
                .map(|s| s.to_string_lossy().into_owned())
                .unwrap_or_default();
            if path.is_dir() {
                if name.starts_with('.') || name == "target" || name == "tests" || name == ".git" {
                    continue;
                }
                stack.push(path);
                continue;
            }
            if path.extension().is_some_and(|e| e == "rnx") {
                let rel = match path.strip_prefix(dir) {
                    Ok(r) => r.to_string_lossy().replace('\\', "/"),
                    Err(_) => return Ok(false),
                };
                have.push(rel);
            }
        }
    }
    let mut want: Vec<String> = Vec::new();
    let mut cache_stack = vec![cached.clone()];
    while let Some(top) = cache_stack.pop() {
        let entries = match std::fs::read_dir(&top) {
            Ok(e) => e,
            Err(_) => return Ok(false),
        };
        for entry in entries.flatten() {
            let path = entry.path();
            if path.is_dir() {
                cache_stack.push(path);
                continue;
            }
            if path.extension().is_some_and(|e| e == "rnx") {
                let rel = match path.strip_prefix(&cached) {
                    Ok(r) => r.to_string_lossy().replace('\\', "/"),
                    Err(_) => return Ok(false),
                };
                want.push(rel);
            }
        }
    }
    have.sort();
    want.sort();
    if have != want {
        return Ok(false);
    }
    for rel in &have {
        let a = std::fs::read(dir.join(rel)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", dir.display()))
        })?;
        let Ok(b) = std::fs::read(cached.join(rel)) else {
            return Ok(false);
        };
        if a != b {
            return Ok(false);
        }
    }
    Ok(true)
}

pub fn scan_dep_prefix(dir: &Path) -> Result<Vec<Diagnostic>, Diagnostic> {
    if verify_std_package_bytes(dir).unwrap_or(false) {
        return Ok(Vec::new());
    }
    let modules = collect_prefix_sources(dir)?;
    let refs: Vec<(&Path, &crate::ast::Module)> = modules
        .iter()
        .map(|(path, module)| (path.as_path(), module))
        .collect();
    Ok(s103_frame(&refs))
}

pub fn s103_sources(sources: &[(PathBuf, String)]) -> Result<Vec<Diagnostic>, Diagnostic> {
    let mut modules = Vec::with_capacity(sources.len());
    for (path, src) in sources {
        let module = crate::parser::Parser::parse_module(src).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot parse `{}`: {}", path.display(), e.message),
            )
        })?;
        modules.push((path.clone(), module));
    }
    let refs: Vec<(&Path, &crate::ast::Module)> = modules
        .iter()
        .map(|(path, module)| (path.as_path(), module))
        .collect();
    Ok(s103_frame(&refs))
}

pub fn declared_rnx_names(module: &crate::ast::Module) -> BTreeSet<String> {
    let mut out = BTreeSet::new();
    let mut take = |name: &str| {
        if name.starts_with("__rnx_") {
            out.insert(name.to_string());
        }
    };
    for decl in &module.decls {
        match &decl.node {
            crate::ast::Decl::Fn(f) => take(&f.name),
            crate::ast::Decl::Const { name, .. } => take(name),
            crate::ast::Decl::Class { name, .. }
            | crate::ast::Decl::Struct { name, .. }
            | crate::ast::Decl::Enum { name, .. }
            | crate::ast::Decl::Record { name, .. } => take(name),
            crate::ast::Decl::Import(d) => {
                for local in crate::modules::native_import_locals(&d.clause) {
                    take(&local);
                }
                match &d.clause {
                    crate::ast::ImportClause::Named(specs) => {
                        for spec in specs {
                            take(&crate::modules::import_local_name(spec));
                        }
                    }
                    crate::ast::ImportClause::DefaultAndNamed(name, specs) => {
                        take(name);
                        for spec in specs {
                            take(&crate::modules::import_local_name(spec));
                        }
                    }
                    crate::ast::ImportClause::Namespace(ns)
                    | crate::ast::ImportClause::Default(ns) => take(ns),
                    _ => {}
                }
            }
            crate::ast::Decl::ExportFrom(d) => {
                if let crate::ast::ImportSource::Native(_) = &d.source {
                    if let crate::ast::ExportClause::Native(sigs) = &d.clause {
                        for sig in sigs {
                            take(sig.alias.as_ref().unwrap_or(&sig.name));
                        }
                    }
                }
            }
            crate::ast::Decl::ExportList(specs) => {
                for spec in specs {
                    take(spec.alias.as_ref().unwrap_or(&spec.name));
                }
            }
            _ => {}
        }
    }
    out
}

pub struct DepDir {
    pub name: String,
    pub dir: PathBuf,
}

fn dep_dirs(root: &Path, config: &ProjectConfig) -> Vec<DepDir> {
    let mut out = Vec::new();
    for (name, spec) in &config.dependencies {
        let dir = match spec {
            DependencySpec::Path { path, .. } => Some(root.join(path)),
            DependencySpec::Git { .. } => {
                let vendor = root.join(crate::fetch::VENDOR_DIR).join(name);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::fetch::testkit::*;

    fn seed_verified_package(full: &str, version: &str, main: &[u8]) -> (String, PathBuf) {
        let base = "http://127.0.0.1:9".to_string();
        let digest = crate::checksum::Sha256::hexdigest(main);
        let dir = plant_store_package(
            full,
            version,
            &digest,
            "",
            std::str::from_utf8(main).unwrap_or("export fn hello(): Int { return 1; }\n"),
        );
        crate::stdlib_seed::pin_report(
            &crate::stdlib_seed::std_pin_path(),
            base.clone(),
            vec![(full.to_string(), version.to_string())],
        )
        .unwrap();
        (base, dir)
    }

    fn mirror_tree(from: &Path, to: &Path) {
        let mut stack = vec![from.to_path_buf()];
        while let Some(top) = stack.pop() {
            for entry in std::fs::read_dir(&top).unwrap().flatten() {
                let path = entry.path();
                let rel = path.strip_prefix(from).unwrap();
                if path.is_dir() {
                    std::fs::create_dir_all(to.join(rel)).unwrap();
                    stack.push(path);
                } else {
                    std::fs::copy(&path, to.join(rel)).unwrap();
                }
            }
        }
    }

    #[test]
    fn verified_std_bytes_pass_and_tampered_fail() {
        let (_guard, _dir) = isolate_cache("verifybytes");
        let main = b"fn f(): Int {\n    return __rnx_probe();\n}\n";
        let (_base, cached) = seed_verified_package("@std/probe", "1.0.0", main);
        let mirror = crate::scratch::dir("verify-mirror");
        mirror_tree(&cached, mirror.path());
        assert!(verify_std_package_bytes(mirror.path()).unwrap());
        assert!(scan_dep_prefix(mirror.path()).unwrap().is_empty());
        std::fs::write(
            mirror.path().join("src").join("main.rnx"),
            b"fn f(): Int {\n    return __rnx_probe_tampered();\n}\n",
        )
        .unwrap();
        assert!(!verify_std_package_bytes(mirror.path()).unwrap());
        let hits = scan_dep_prefix(mirror.path()).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].code, Code::S103);
        assert!(hits[0].message.contains("__rnx_probe_tampered"), "{}", hits[0].message);
    }

    #[test]
    fn unverifiable_std_claim_is_scanned() {
        let (_guard, _dir) = isolate_cache("verifycold");
        let dir = crate::scratch::dir("verify-cold");
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join(crate::project::MANIFEST_FILE),
            "export default {\n    project: {\n        name: \"@std/probe\",\n        version: \"9.9.9\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(
            dir.join("src").join("main.rnx"),
            "fn f(): Int {\n    return __rnx_probe();\n}\n",
        )
        .unwrap();
        assert!(!verify_std_package_bytes(dir.path()).unwrap());
        let hits = scan_dep_prefix(dir.path()).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].code, Code::S103);
    }
}
