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

pub(crate) fn rollup_effective_capabilities(
    own: &BTreeMap<String, BTreeSet<Capability>>,
    deps: &BTreeMap<String, Vec<String>>,
) -> Result<BTreeMap<String, BTreeSet<Capability>>, Diagnostic> {
    fn visit(
        name: &str,
        own: &BTreeMap<String, BTreeSet<Capability>>,
        deps: &BTreeMap<String, Vec<String>>,
        done: &mut BTreeMap<String, BTreeSet<Capability>>,
        stack: &mut Vec<String>,
    ) -> Result<BTreeSet<Capability>, Diagnostic> {
        if let Some(hit) = done.get(name) {
            return Ok(hit.clone());
        }
        if let Some(pos) = stack.iter().position(|n| n == name) {
            let mut chain: Vec<String> = stack[pos..].to_vec();
            chain.push(name.to_string());
            return Err(Diagnostic::new(
                Code::E108,
                format!("dependency cycle detected: {}", chain.join(" -> ")),
            ));
        }
        stack.push(name.to_string());
        let mut effective = own.get(name).cloned().unwrap_or_default();
        let mut edges: Vec<String> = deps.get(name).cloned().unwrap_or_default();
        edges.sort();
        edges.dedup();
        for dep in &edges {
            if !own.contains_key(dep) && !deps.contains_key(dep) {
                stack.pop();
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("package `{name}` depends on unknown package `{dep}`"),
                ));
            }
            let sub = visit(dep, own, deps, done, stack)?;
            effective.extend(sub);
        }
        stack.pop();
        done.insert(name.to_string(), effective.clone());
        Ok(effective)
    }
    let mut all: BTreeSet<String> = BTreeSet::new();
    all.extend(own.keys().cloned());
    all.extend(deps.keys().cloned());
    let mut done: BTreeMap<String, BTreeSet<Capability>> = BTreeMap::new();
    let mut stack: Vec<String> = Vec::new();
    for name in &all {
        visit(name, own, deps, &mut done, &mut stack)?;
    }
    Ok(done)
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

fn sig_ty_name(ty: &crate::ast::Type) -> String {
    let mut out = ty.path.join(".");
    if !ty.args.is_empty() {
        out.push('<');
        out.push_str(
            &ty.args
                .iter()
                .map(sig_ty_name)
                .collect::<Vec<_>>()
                .join(", "),
        );
        out.push('>');
    }
    if out.is_empty() {
        out.push('?');
    }
    out
}

fn sig_ty_eq(declared: &str, manifest: &str) -> bool {
    let norm = |s: &str| s.split_whitespace().collect::<String>();
    if norm(declared) == norm(manifest) {
        return true;
    }
    let short = |s: &str| {
        let s = norm(s);
        let s = s.strip_prefix("std.prelude.").unwrap_or(&s).to_string();
        match s.rsplit_once('<') {
            Some((head, _)) => head.rsplit('.').next().unwrap_or(&s).to_string(),
            None => s.rsplit('.').next().unwrap_or(&s).to_string(),
        }
    };
    let map = |s: String| {
        if s == "Char" {
            "String".to_string()
        } else {
            s
        }
    };
    map(short(declared)) == map(short(manifest))
}

pub fn check_runtime_sig(
    module: &str,
    sig: &crate::ast::NativeFnSig,
    span: diagnostics::Span,
) -> Option<Diagnostic> {
    if !crate::runtime::is_runtime_module(module) {
        return Some(
            Diagnostic::new(Code::E108, format!("unknown runtime module `{module}`"))
                .with_span(span)
                .with_hint("runtime modules are: bytes, core, env, fs, io, json, math, net, os, process, random, sync, testing, time"),
        );
    }
    let Some(decl) = crate::runtime::lookup(&sig.name) else {
        return Some(
            Diagnostic::new(
                Code::E108,
                format!("unknown runtime symbol `{}` in module `{module}`", sig.name),
            )
            .with_span(span)
            .with_hint(format!(
                "call {} instead of the `__rnx_*` internal",
                crate::runtime::owner_hint(module)
            )),
        );
    };
    if decl.module != module {
        return Some(
            Diagnostic::new(
                Code::E108,
                format!(
                    "`{}` lives in runtime `{}` rather than `{module}`",
                    sig.name, decl.module
                ),
            )
            .with_span(span)
            .with_hint(format!(
                "import it with `import {{ fn {}(..): .. }} from runtime \"{}\"`",
                sig.name, decl.module
            )),
        );
    }
    if sig.params.len() != decl.params.len() {
        return Some(
            Diagnostic::new(
                Code::E108,
                format!(
                    "`{}` takes {} args, declared with {}",
                    sig.name,
                    decl.params.len(),
                    sig.params.len()
                ),
            )
            .with_span(span)
            .with_hint(format!(
                "declare it as `fn {}({}): {}`",
                sig.name,
                decl.params
                    .iter()
                    .enumerate()
                    .map(|(i, t)| format!("p{i}: {t}"))
                    .collect::<Vec<_>>()
                    .join(", "),
                decl.ret
            )),
        );
    }
    for (param, want) in sig.params.iter().zip(decl.params.iter()) {
        let got = param.ty.as_ref().map(sig_ty_name).unwrap_or_else(|| "?".to_string());
        if got != "?" && !sig_ty_eq(&got, want) {
            return Some(
                Diagnostic::new(
                    Code::E108,
                    format!(
                        "`{}` param `{}` must be `{want}`, declared `{got}`",
                        sig.name, param.name
                    ),
                )
                .with_span(param.span)
                .with_hint("match the runtime manifest signature"),
            );
        }
    }
    if let Some(ret) = &sig.ret {
        let got = sig_ty_name(ret);
        if !sig_ty_eq(&got, decl.ret) {
            return Some(
                Diagnostic::new(
                    Code::E108,
                    format!(
                        "`{}` must return `{}`, declared `{got}`",
                        sig.name, decl.ret
                    ),
                )
                .with_span(span)
                .with_hint("match the runtime manifest signature"),
            );
        }
    }
    None
}

pub struct RuntimeImportRef {
    pub module: String,
    pub symbol: String,
    pub local: String,
    pub span: diagnostics::Span,
}

pub fn collect_runtime_imports(module: &crate::ast::Module) -> Vec<RuntimeImportRef> {
    let mut out = Vec::new();
    for decl in &module.decls {
        let crate::ast::Decl::Import(d) = &decl.node else {
            continue;
        };
        let crate::ast::ImportSource::Runtime(module_name) = &d.source else {
            continue;
        };
        let crate::ast::ImportClause::Named(specs) = &d.clause else {
            continue;
        };
        for spec in specs {
            let Some(sig) = &spec.native_fn else {
                continue;
            };
            out.push(RuntimeImportRef {
                module: module_name.clone(),
                symbol: sig.name.clone(),
                local: sig.alias.clone().unwrap_or_else(|| sig.name.clone()),
                span: d.source_span,
            });
        }
    }
    out
}

fn is_std_path(path: &Path) -> bool {
    path.to_string_lossy().starts_with("@std/")
}

fn owning_package_dir(path: &Path) -> Option<PathBuf> {
    let mut dir = if path.is_file() {
        path.parent()?.to_path_buf()
    } else {
        path.to_path_buf()
    };
    loop {
        if dir.join(crate::project::MANIFEST_FILE).is_file() {
            return Some(dir);
        }
        if !dir.pop() {
            return None;
        }
    }
}

pub(crate) fn runtime_import_allowed(path: &Path) -> bool {
    if is_std_path(path) {
        return true;
    }
    let Some(dir) = owning_package_dir(path) else {
        return false;
    };
    let Ok(Some(cfg)) = crate::project::ProjectConfig::load_from_dir(&dir) else {
        return false;
    };
    if !is_std_package_name(&cfg.name) {
        return false;
    }
    verify_std_package_bytes(&dir).unwrap_or(false)
}

pub fn runtime_frame_rejection(module: &str, symbol: &str, span: diagnostics::Span) -> Diagnostic {
    Diagnostic::new(
        Code::E108,
        format!("cannot import `{symbol}` from runtime `{module}` outside registry-verified `@std/*`; call {} instead of the `__rnx_*` internal", crate::runtime::owner_hint(module)),
    )
    .with_span(span)
    .with_hint(format!(
        "call {} instead of the `__rnx_*` internal",
        crate::runtime::owner_hint(module)
    ))
}

pub fn verify_runtime_imports(graph: &crate::modules::ModuleGraph) -> Vec<Diagnostic> {
    let mut diags = Vec::new();
    let mut files: Vec<&PathBuf> = graph.files.iter().map(|f| &f.path).collect();
    files.sort();
    for path in files {
        let Some(file) = graph.files.iter().find(|f| &f.path == path) else {
            continue;
        };
        if runtime_import_allowed(&file.path) {
            continue;
        }
        let mut refs = collect_runtime_imports(&file.module);
        refs.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.span.start.cmp(&b.span.start)));
        for r in refs {
            let mut diag = runtime_frame_rejection(&r.module, &r.symbol, r.span);
            diag.file = Some(file.path.clone());
            diags.push(diag);
        }
    }
    diags.sort_by(|a, b| {
        a.file.cmp(&b.file).then(
            a.span
                .map(|s| s.start)
                .unwrap_or(0)
                .cmp(&b.span.map(|s| s.start).unwrap_or(0)),
        )
    });
    diags
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

pub fn scan_dep_runtime_imports(dir: &Path) -> Result<Vec<Diagnostic>, Diagnostic> {
    if verify_std_package_bytes(dir).unwrap_or(false) {
        return Ok(Vec::new());
    }
    let modules = collect_prefix_sources(dir)?;
    let mut out = Vec::new();
    for (path, module) in &modules {
        for mut diag in crate::semantic::check(module) {
            if diag.code.is_warning() {
                continue;
            }
            if diag.file.is_none() {
                diag.file = Some(path.clone());
            }
            out.push(diag);
        }
        let mut refs = collect_runtime_imports(module);
        refs.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.span.start.cmp(&b.span.start)));
        for r in refs {
            let mut diag = runtime_frame_rejection(&r.module, &r.symbol, r.span);
            diag.file = Some(path.clone());
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
    Ok(out)
}

pub fn publish_precheck_reports(diag: &Diagnostic) -> bool {
    match diag.code {
        Code::E303 => diag.message.contains("__rnx_"),
        Code::E108 => diag.message.contains("runtime"),
        _ => false,
    }
}

pub fn scan_publish_sources(
    sources: &[(PathBuf, String)],
    is_std: bool,
) -> Result<Vec<Diagnostic>, Diagnostic> {
    let mut out = Vec::new();
    let mut sorted: Vec<&(PathBuf, String)> = sources.iter().collect();
    sorted.sort_by(|a, b| a.0.cmp(&b.0));
    for (path, src) in sorted {
        let module = crate::parser::Parser::parse_module(src).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot parse `{}`: {}", path.display(), e.message),
            )
        })?;
        for mut diag in crate::semantic::check(&module) {
            if diag.code.is_warning() {
                continue;
            }
            if !publish_precheck_reports(&diag) {
                continue;
            }
            if diag.file.is_none() {
                diag.file = Some(path.clone());
            }
            out.push(diag);
        }
        if !is_std {
            let mut refs = collect_runtime_imports(&module);
            refs.sort_by(|a, b| a.symbol.cmp(&b.symbol).then(a.span.start.cmp(&b.span.start)));
            for r in refs {
                let mut diag = runtime_frame_rejection(&r.module, &r.symbol, r.span);
                diag.file = Some(path.clone());
                out.push(diag);
            }
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
    Ok(out)
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

pub fn verify_entry(entry: &Path) -> Vec<Diagnostic> {    let start = if entry.is_file() {
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
    let mut diags = verify_runtime_imports(graph);
    let boundary_canon = std::fs::canonicalize(boundary).unwrap_or_else(|_| boundary.to_path_buf());
    let proj_canon = std::fs::canonicalize(proj_root).unwrap_or_else(|_| proj_root.to_path_buf());
    let deps = dep_dirs(boundary, config);

    #[derive(Clone, PartialEq, Eq, PartialOrd, Ord)]
    enum Owner {
        Root,
        Local,
        Dep(String),
        Std,
        Outside,
    }
    fn owner_of(
        path: &Path,
        boundary_canon: &Path,
        proj_canon: &Path,
        deps: &[DepDir],
    ) -> Owner {
        if path.to_string_lossy().starts_with("@std/") {
            return Owner::Std;
        }
        let canon = std::fs::canonicalize(path).unwrap_or_else(|_| path.to_path_buf());
        if canon.starts_with(boundary_canon) {
            if canon.starts_with(proj_canon) {
                return Owner::Root;
            }
            return Owner::Local;
        }
        if let Some(d) = deps.iter().find(|d| canon.starts_with(&d.dir)) {
            return Owner::Dep(d.name.clone());
        }
        Owner::Outside
    }
    let mut owner_by_display: BTreeMap<String, Owner> = BTreeMap::new();
    for f in &graph.files {
        let display = f.path.to_string_lossy().replace('\\', "/");
        owner_by_display.insert(
            display,
            owner_of(&f.path, &boundary_canon, &proj_canon, &deps),
        );
    }

    let mut owned_sources: Vec<(PathBuf, String)> = Vec::new();
    let mut sorted_files: Vec<&crate::modules::ModuleFile> = graph.files.iter().collect();
    sorted_files.sort_by(|a, b| a.path.cmp(&b.path));
    for f in &sorted_files {
        if let Some(src) = crate::modules::graph_file_source(&f.path) {
            owned_sources.push((f.path.clone(), src));
        }
    }
    let inputs: Vec<capabilities::GraphInput> = sorted_files
        .iter()
        .filter_map(|f| {
            owned_sources
                .iter()
                .find(|(p, _)| p == &f.path)
                .map(|(_, src)| capabilities::GraphInput {
                    path: f.path.clone(),
                    module: &f.module,
                    source: src.as_str(),
                })
        })
        .collect();
    let report = match capabilities::analyze_file_graph(&inputs, capabilities::DEFAULT_FUEL) {
        Ok(r) => r,
        Err(e) => {
            let span = diagnostics::Span { start: 0, end: 0 };
            diags.push(e.to_diagnostic(span));
            return diags;
        }
    };

    let mut pkg_caps: BTreeMap<Owner, BTreeSet<Capability>> = BTreeMap::new();
    let mut pkg_traces: BTreeMap<Owner, Vec<CallChain>> = BTreeMap::new();
    for trace in &report.traces {
        let mut owners: BTreeSet<Owner> = BTreeSet::new();
        for node in &trace.nodes {
            if let Some(owner) = owner_by_display.get(&node.file) {
                owners.insert(owner.clone());
            }
        }
        if owners.is_empty() {
            continue;
        }
        for owner in owners {
            pkg_caps
                .entry(owner.clone())
                .or_default()
                .insert(trace.capability.clone());
            pkg_traces.entry(owner).or_default().push(trace.clone());
        }
    }
    let caps_of = |want: &Owner| pkg_caps.get(want).cloned().unwrap_or_default();
    let traces_of = |want: &Owner| pkg_traces.get(want).cloned().unwrap_or_default();
    let mut local_caps = caps_of(&Owner::Root);
    local_caps.extend(caps_of(&Owner::Local).iter().cloned());
    let mut local_traces = traces_of(&Owner::Root);
    local_traces.extend(traces_of(&Owner::Local).iter().cloned());
    let root_caps = caps_of(&Owner::Root);
    let root_traces = traces_of(&Owner::Root);
    let mut dep_caps: BTreeMap<String, BTreeSet<Capability>> = BTreeMap::new();
    let mut dep_traces: BTreeMap<String, Vec<CallChain>> = BTreeMap::new();
    for (owner, caps) in &pkg_caps {
        if let Owner::Dep(name) = owner {
            dep_caps.insert(name.clone(), caps.clone());
            if let Some(traces) = pkg_traces.get(owner) {
                dep_traces.insert(name.clone(), traces.clone());
            }
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

#[derive(Debug, Clone)]
pub struct DepAnalysis {
    pub name: String,
    pub version: String,
    pub tier: CapabilityTier,
    pub capabilities: Vec<String>,
    pub traces: Vec<CallChain>,
}

pub fn analyze_dep_dir(dir: &Path) -> Result<DepAnalysis, Diagnostic> {
    let mut memo: BTreeMap<PathBuf, DepAnalysis> = BTreeMap::new();
    let mut stack: Vec<PathBuf> = Vec::new();
    analyze_dep_effective(dir, &mut memo, &mut stack)
}

fn analyze_dep_effective(
    dir: &Path,
    memo: &mut BTreeMap<PathBuf, DepAnalysis>,
    stack: &mut Vec<PathBuf>,
) -> Result<DepAnalysis, Diagnostic> {
    let canon = std::fs::canonicalize(dir).unwrap_or_else(|_| dir.to_path_buf());
    if let Some(hit) = memo.get(&canon) {
        return Ok(hit.clone());
    }
    if stack.contains(&canon) {
        let mut chain: Vec<String> = stack
            .iter()
            .map(|p| p.to_string_lossy().replace('\\', "/"))
            .collect();
        chain.push(canon.to_string_lossy().replace('\\', "/"));
        return Err(Diagnostic::new(
            Code::E108,
            format!("dependency cycle detected: {}", chain.join(" -> ")),
        ));
    }
    stack.push(canon.clone());
    let own = analyze_dep_own(dir)?;
    let config = project::load_manifest(dir)
        .map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot read dependency manifest in `{}`: {e}", dir.display()),
            )
        })?
        .and_then(|m| m.project)
        .ok_or_else(|| {
            Diagnostic::new(
                Code::E108,
                format!("dependency `{}` has no [project] section", dir.display()),
            )
        })?;
    let mut caps: BTreeSet<Capability> = BTreeSet::new();
    for s in &own.capabilities {
        caps.insert(parse_cap(s)?);
    }
    let mut traces = own.traces.clone();
    let mut sub: BTreeMap<String, PathBuf> = BTreeMap::new();
    for (name, spec) in &config.dependencies {
        match spec {
            project::DependencySpec::Path { path, .. } => {
                let resolved = std::fs::canonicalize(dir.join(path)).map_err(|_| {
                    Diagnostic::new(Code::E108, format!("cannot resolve package `{name}`"))
                })?;
                sub.insert(name.clone(), resolved);
            }
            project::DependencySpec::Git { .. } => {
                let vendor = dir.join(crate::fetch::VENDOR_DIR).join(name);
                if vendor.join(project::MANIFEST_FILE).is_file() {
                    if let Ok(resolved) = std::fs::canonicalize(&vendor) {
                        sub.insert(name.clone(), resolved);
                    }
                } else if let Some(cached) = git_cache_dir(dir, name) {
                    sub.insert(name.clone(), cached);
                }
            }
            project::DependencySpec::Semver { .. }
            | project::DependencySpec::Url { .. }
            | project::DependencySpec::Native { .. } => {}
        }
    }
    for subdir in sub.values() {
        let dep = analyze_dep_effective(subdir, memo, stack)?;
        for s in &dep.capabilities {
            caps.insert(parse_cap(s)?);
        }
        traces.extend(dep.traces.clone());
    }
    stack.pop();
    let tier = tier_of_caps(&caps);
    let mut capabilities: Vec<String> = caps.iter().map(|c| c.to_string()).collect();
    capabilities.sort();
    let analysis = DepAnalysis {
        name: own.name,
        version: own.version,
        tier,
        capabilities,
        traces,
    };
    memo.insert(canon, analysis.clone());
    Ok(analysis)
}

pub(crate) fn analyze_dep_own(dir: &Path) -> Result<DepAnalysis, Diagnostic> {
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
    let mut owned: Vec<(PathBuf, String, crate::ast::Module)> = Vec::new();
    for path in &files {
        let src = std::fs::read_to_string(path).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", path.display()))
        })?;
        let module = crate::parser::Parser::parse_module(&src).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot parse `{}`: {e}", path.display()))
        })?;
        owned.push((path.clone(), src, module));
    }
    let mut std_queue: Vec<String> = Vec::new();
    for (_, _, module) in &owned {
        for decl in &module.decls {
            let crate::ast::Decl::Import(d) = &decl.node else {
                continue;
            };
            let crate::ast::ImportSource::Module(source) = &d.source else {
                continue;
            };
            if !crate::modules::is_std_spec(source) {
                continue;
            }
            let rest = crate::modules::std_rest(source);
            if !rest.is_empty() && !std_queue.iter().any(|n| n == rest) {
                std_queue.push(rest.to_string());
            }
        }
    }
    let mut std_owned: Vec<(String, String, crate::ast::Module)> = Vec::new();
    while let Some(rest) = std_queue.pop() {
        if std_owned.iter().any(|(n, _, _)| n == &rest) {
            continue;
        }
        let src = crate::modules::load_std_module_source(&rest).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot load `@std/{rest}` for capability analysis: {}", e.message),
            )
        })?;
        let module = crate::parser::Parser::parse_module(&src).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot parse `@std/{rest}`: {}", e.message),
            )
        })?;
        for decl in &module.decls {
            let crate::ast::Decl::Import(d) = &decl.node else {
                continue;
            };
            let crate::ast::ImportSource::Module(source) = &d.source else {
                continue;
            };
            if !crate::modules::is_std_spec(source) {
                continue;
            }
            let dep = crate::modules::std_rest(source);
            if !dep.is_empty()
                && !std_owned.iter().any(|(n, _, _)| n == &dep)
                && !std_queue.iter().any(|n| n == &dep)
            {
                std_queue.push(dep.to_string());
            }
        }
        std_owned.push((rest, src, module));
    }
    let mut inputs: Vec<capabilities::GraphInput> = Vec::new();
    for (path, src, module) in &owned {
        inputs.push(capabilities::GraphInput {
            path: path.clone(),
            module,
            source: src.as_str(),
        });
    }
    for (rest, src, module) in &std_owned {
        inputs.push(capabilities::GraphInput {
            path: PathBuf::from(format!("@std/{rest}")),
            module,
            source: src.as_str(),
        });
    }
    inputs.sort_by(|a, b| a.path.cmp(&b.path));
    let dir_prefix = dir.to_string_lossy().replace('\\', "/");
    let report = match capabilities::analyze_file_graph(&inputs, capabilities::DEFAULT_FUEL) {
        Ok(r) => r,
        Err(e) => {
            let span = diagnostics::Span { start: 0, end: 0 };
            let d = e.to_diagnostic(span);
            return Err(d);
        }
    };
    let mut caps: BTreeSet<Capability> = BTreeSet::new();
    let mut traces: Vec<CallChain> = Vec::new();
    for trace in &report.traces {
        let touches_dir = trace.nodes.iter().any(|n| {
            let f = n.file.replace('\\', "/");
            f == dir_prefix || f.starts_with(&format!("{dir_prefix}/"))
        });
        if !touches_dir {
            continue;
        }
        caps.insert(trace.capability.clone());
        traces.push(trace.clone());
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
        let main = b"fn f(): Int {\n    return 1;\n}\n";
        let (_base, cached) = seed_verified_package("@std/probe", "1.0.0", main);
        let mirror = crate::scratch::dir("verify-mirror");
        mirror_tree(&cached, mirror.path());
        assert!(verify_std_package_bytes(mirror.path()).unwrap());
        assert!(scan_dep_runtime_imports(mirror.path()).unwrap().is_empty());
        std::fs::write(
            mirror.path().join("src").join("main.rnx"),
            b"fn f(): Int {\n    return __rnx_probe_tampered();\n}\n",
        )
        .unwrap();
        assert!(!verify_std_package_bytes(mirror.path()).unwrap());
        let hits = scan_dep_runtime_imports(mirror.path()).unwrap();
        assert!(
            hits.iter().any(|d| d.code == Code::E303),
            "tampered bytes surface as scope errors: {hits:?}"
        );
    }

    #[test]
    fn unverifiable_std_claim_rejects_runtime_import() {
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
            "import { fn __rnx_clock_mono(): Int } from runtime \"time\"\nfn f(): Int {\n    return __rnx_clock_mono();\n}\n",
        )
        .unwrap();
        assert!(!verify_std_package_bytes(dir.path()).unwrap());
        let hits = scan_dep_runtime_imports(dir.path()).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].code, Code::E108);
        assert!(hits[0].message.contains("__rnx_clock_mono"), "{}", hits[0].message);
        assert!(
            hits[0]
                .hint
                .as_deref()
                .unwrap_or_default()
                .contains("@std/time"),
            "{:?}",
            hits[0].hint
        );
    }

    #[test]
    fn verified_std_tree_skips_runtime_scan() {
        let (_guard, _dir) = isolate_cache("verifyrt");
        let main = b"import { fn __rnx_clock_mono(): Int } from runtime \"time\"\nfn f(): Int {\n    return __rnx_clock_mono();\n}\n";
        let (_base, cached) = seed_verified_package("@std/probe", "1.0.0", main);
        let mirror = crate::scratch::dir("verify-rt-mirror");
        mirror_tree(&cached, mirror.path());
        assert!(verify_std_package_bytes(mirror.path()).unwrap());
        assert!(scan_dep_runtime_imports(mirror.path()).unwrap().is_empty());
    }

    fn publish_sources(path: &str, src: &str) -> Vec<(PathBuf, String)> {
        vec![(PathBuf::from(path), src.to_string())]
    }

    fn unfiltered_errors(src: &str) -> Vec<Diagnostic> {
        let module = crate::parser::Parser::parse_module(src).unwrap();
        crate::semantic::check(&module)
            .into_iter()
            .filter(|d| !d.code.is_warning())
            .collect()
    }

    #[test]
    fn publish_precheck_accepts_prelude_protocol_conformance() {
        let src = "import { ArrayIter, Iterable, Iterator } from \"@std/prelude\";\n\nexport class Bag<T> with Iterable<T> {\n    let items: Array<T>;\n    init(items: Array<T>) {\n        this.items = items;\n    }\n    fn iterator(): Iterator<T> {\n        return new ArrayIter<T>(this.items);\n    }\n}\n";
        assert!(
            unfiltered_errors(src).iter().any(|d| d.code == Code::E304),
            "fixture must exercise the isolated conformance blind spot"
        );
        let hits = scan_publish_sources(&publish_sources("src/lib.rnx", src), true).unwrap();
        assert!(hits.is_empty(), "prelude-declared conformance is server-verified: {hits:?}");
    }

    #[test]
    fn publish_precheck_accepts_prelude_extended_builtin_method() {
        let src = "export fn hasPrefix(s: String, prefix: String): Bool {\n    return s.startsWith(prefix);\n}\n";
        assert!(
            unfiltered_errors(src).iter().any(|d| d.code == Code::E108),
            "fixture must exercise the isolated method blind spot"
        );
        let hits = scan_publish_sources(&publish_sources("src/lib.rnx", src), true).unwrap();
        assert!(hits.is_empty(), "prelude-extended methods are server-verified: {hits:?}");
    }

    #[test]
    fn publish_precheck_accepts_local_cross_file_conformance() {
        let main = "import { Pager, Page } from \"./types\";\n\nexport fn first(): Pager {\n    return new Page();\n}\n\nfn Main(): Int {\n    return first().next();\n}\n";
        let types = "export interface Pager {\n    fn next(): Int;\n}\n\nexport class Page with Pager {\n    fn next(): Int {\n        return 1;\n    }\n}\n";
        let sources = vec![
            (PathBuf::from("src/main.rnx"), main.to_string()),
            (PathBuf::from("src/types.rnx"), types.to_string()),
        ];
        let hits = scan_publish_sources(&sources, false).unwrap();
        assert!(hits.is_empty(), "cross-file decls resolve in the merged graph: {hits:?}");
    }

    #[test]
    fn publish_precheck_rejects_bare_rnx_use() {
        let src = "fn Main(): Int {\n    return __rnx_clock_mono();\n}\n";
        let hits = scan_publish_sources(&publish_sources("src/main.rnx", src), false).unwrap();
        assert!(
            hits.iter().any(|d| d.code == Code::E303 && d.message.contains("__rnx_clock_mono")),
            "{hits:?}"
        );
    }

    #[test]
    fn publish_precheck_rejects_declared_runtime_import_outside_std() {
        let src = "import { fn __rnx_clock_mono(): Int } from runtime \"time\";\nfn Main(): Int {\n    return __rnx_clock_mono();\n}\n";
        let hits = scan_publish_sources(&publish_sources("src/main.rnx", src), false).unwrap();
        assert_eq!(hits.len(), 1, "{hits:?}");
        assert_eq!(hits[0].code, Code::E108);
        assert!(hits[0].message.contains("__rnx_clock_mono"), "{}", hits[0].message);
    }

    #[test]
    fn publish_precheck_rejects_unknown_runtime_module_in_std_frame() {
        let src = "import { fn probe(): Int } from runtime \"bogus\";\nfn Main(): Int {\n    return probe();\n}\n";
        let hits = scan_publish_sources(&publish_sources("src/lib.rnx", src), true).unwrap();
        assert!(
            hits.iter().any(|d| d.code == Code::E108 && d.message.contains("runtime")),
            "{hits:?}"
        );
    }

    fn cap_set(names: &[&str]) -> BTreeSet<Capability> {
        names
            .iter()
            .map(|s| Capability::from_str(s).unwrap())
            .collect()
    }

    fn rollup_fixture(
        own: &[(&str, &[&str])],
        deps: &[(&str, &[&str])],
    ) -> (
        BTreeMap<String, BTreeSet<Capability>>,
        BTreeMap<String, Vec<String>>,
    ) {
        let mut o: BTreeMap<String, BTreeSet<Capability>> = BTreeMap::new();
        for (name, caps) in own {
            o.insert((*name).to_string(), cap_set(caps));
        }
        let mut d: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for (name, edges) in deps {
            d.insert(
                (*name).to_string(),
                edges.iter().map(|s| (*s).to_string()).collect(),
            );
        }
        (o, d)
    }

    #[test]
    fn rollup_chain_closes_transitively() {
        let (own, deps) = rollup_fixture(
            &[("app", &[]), ("mid", &[]), ("leaf", &["fs:read:/tmp/**"])],
            &[("app", &["mid"]), ("mid", &["leaf"]), ("leaf", &[])],
        );
        let out = rollup_effective_capabilities(&own, &deps).unwrap();
        assert_eq!(out[&"leaf".to_string()], cap_set(&["fs:read:/tmp/**"]));
        assert_eq!(out[&"mid".to_string()], cap_set(&["fs:read:/tmp/**"]));
        assert_eq!(out[&"app".to_string()], cap_set(&["fs:read:/tmp/**"]));
    }

    #[test]
    fn rollup_diamond_counts_shared_dep_once() {
        let (own, deps) = rollup_fixture(
            &[
                ("app", &[]),
                ("left", &["term:write"]),
                ("right", &[]),
                ("shared", &["fs:read:/tmp/**"]),
            ],
            &[
                ("app", &["left", "right"]),
                ("left", &["shared"]),
                ("right", &["shared"]),
                ("shared", &[]),
            ],
        );
        let out = rollup_effective_capabilities(&own, &deps).unwrap();
        assert_eq!(out[&"shared".to_string()], cap_set(&["fs:read:/tmp/**"]));
        assert_eq!(
            out[&"left".to_string()],
            cap_set(&["fs:read:/tmp/**", "term:write"])
        );
        assert_eq!(out[&"right".to_string()], cap_set(&["fs:read:/tmp/**"]));
        assert_eq!(
            out[&"app".to_string()],
            cap_set(&["fs:read:/tmp/**", "term:write"])
        );
    }

    #[test]
    fn rollup_deep_chain_stays_memoized() {
        let depth = 500usize;
        let mut own: BTreeMap<String, BTreeSet<Capability>> = BTreeMap::new();
        let mut deps: BTreeMap<String, Vec<String>> = BTreeMap::new();
        for i in 0..depth {
            let name = format!("n{i:04}");
            own.insert(
                name.clone(),
                cap_set(&[&format!("env:read:VAR_{i:04}")]),
            );
            let next = if i + 1 < depth {
                vec![format!("n{:04}", i + 1)]
            } else {
                Vec::new()
            };
            deps.insert(name, next);
        }
        let out = rollup_effective_capabilities(&own, &deps).unwrap();
        assert_eq!(out.len(), depth);
        assert_eq!(out["n0000"].len(), depth);
        assert_eq!(out["n0499"].len(), 1);
        for i in 0..depth {
            assert_eq!(out[&format!("n{i:04}")].len(), depth - i, "node {i}");
            assert!(
                out[&format!("n{i:04}")]
                    .contains(&Capability::from_str(&format!("env:read:VAR_{i:04}")).unwrap())
            );
        }
    }

    #[test]
    fn rollup_cycle_fails_closed() {
        let (own, deps) = rollup_fixture(
            &[("a", &[]), ("b", &["fs:read:/tmp/**"])],
            &[("a", &["b"]), ("b", &["a"])],
        );
        let err = rollup_effective_capabilities(&own, &deps).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("dependency cycle detected"), "{}", err.message);
        assert!(err.message.contains('a') && err.message.contains('b'), "{}", err.message);
    }

    #[test]
    fn rollup_self_cycle_fails_closed() {
        let (own, deps) = rollup_fixture(&[("a", &["term:write"])], &[("a", &["a"])]);
        let err = rollup_effective_capabilities(&own, &deps).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("dependency cycle detected"), "{}", err.message);
    }

    #[test]
    fn rollup_edge_order_does_not_change_output() {
        let (own, _) = rollup_fixture(
            &[("app", &[]), ("left", &["term:write"]), ("shared", &["fs:read:/tmp/**"])],
            &[],
        );
        let fwd: BTreeMap<String, Vec<String>> = [
            ("app".to_string(), vec!["left".to_string(), "shared".to_string()]),
            ("left".to_string(), vec!["shared".to_string()]),
            ("shared".to_string(), Vec::new()),
        ]
        .into_iter()
        .collect();
        let mut bwd = fwd.clone();
        for edges in bwd.values_mut() {
            edges.reverse();
        }
        let a = rollup_effective_capabilities(&own, &fwd).unwrap();
        let b = rollup_effective_capabilities(&own, &bwd).unwrap();
        assert_eq!(a, b);
        assert_eq!(a[&"app".to_string()], cap_set(&["fs:read:/tmp/**", "term:write"]));
    }

    fn pkg_tree(tag: &str, pkgs: &[(&str, &str, &str, &str)]) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("rollup-{tag}"));
        for (name, version, deps, lib) in pkgs {
            let pkg = dir.join(name);
            std::fs::create_dir_all(pkg.join("src")).unwrap();
            let mut manifest = format!(
                "export default {{\n    project: {{\n        name: \"{name}\",\n        version: \"{version}\"\n    }},\n    entries: {{ main: \"src/lib.rnx\" }}",
            );
            if !deps.is_empty() {
                manifest.push_str(&format!(",\n    dependencies: {{\n{deps}    }}"));
            }
            manifest.push_str("\n}\n");
            std::fs::write(pkg.join("Project.config"), manifest).unwrap();
            std::fs::write(pkg.join("src").join("lib.rnx"), lib).unwrap();
        }
        dir
    }

    const PURE_LIB: &str = "export fn f(): Int {\n    return 1;\n}\n";
    const SINK_LIB: &str =
        "export fn f(): Int {\n    File.open(\"/tmp/chain.txt\", FileMode.Read);\n    return 0;\n}\n";

    #[test]
    fn analyze_dep_dir_rolls_up_path_chain() {
        let dir = pkg_tree(
            "chain",
            &[
                ("app", "0.1.0", "        mid: { path: \"../mid\" }\n", PURE_LIB),
                ("mid", "0.1.0", "        leaf: { path: \"../leaf\" }\n", PURE_LIB),
                ("leaf", "0.1.0", "", SINK_LIB),
            ],
        );
        let analysis = analyze_dep_dir(&dir.join("app")).unwrap();
        assert_eq!(analysis.name, "app");
        assert!(
            analysis.capabilities.contains(&"fs:read:/tmp/**".to_string()),
            "{:?}",
            analysis.capabilities
        );
        assert_eq!(analysis.tier, CapabilityTier::Ambient);
        let mid = analyze_dep_dir(&dir.join("mid")).unwrap();
        assert!(mid.capabilities.contains(&"fs:read:/tmp/**".to_string()), "{mid:?}");
        let own = analyze_dep_own(&dir.join("app")).unwrap();
        assert!(own.capabilities.is_empty(), "{own:?}");
    }

    #[test]
    fn analyze_dep_dir_cycle_fails_closed() {
        let dir = pkg_tree(
            "cycle",
            &[
                ("a", "0.1.0", "        b: { path: \"../b\" }\n", PURE_LIB),
                ("b", "0.1.0", "        a: { path: \"../a\" }\n", SINK_LIB),
            ],
        );
        let err = analyze_dep_dir(&dir.join("a")).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("dependency cycle detected"), "{}", err.message);
    }
}
