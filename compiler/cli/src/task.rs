use diagnostics::{Code, Diagnostic};
use frontend::checksum::Sha256;
use frontend::project::{
    BindgenDriver, ProjectConfig, RunsIn, TaskDef, TaskMeta, TaskOnFailure, TaskRef, TaskStep,
    TaskStrategy,
};
use std::collections::{BTreeMap, BTreeSet};
use std::io::IsTerminal;
use std::path::{Path, PathBuf};

pub const BINDGEN_TASK: &str = "bindgen";
pub const PUSH_URL_ENV: &str = "RNX_OCI_PUSH_URL";
pub const PUSH_TOKEN_ENV: &str = "RNX_OCI_PUSH_TOKEN";
pub const ALLOW_ENV: &str = "RNX_ALLOW_TASKS";

fn task_error(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(Code::E108, message)
}

fn ref_hint() -> &'static str {
    "local tasks are addressed by their name: `rnx task <name>`; imported tasks are `<package>#<task>`, for example `rnx task @std/app#bundle`"
}

pub fn parse_cli_ref(raw: &str) -> Result<TaskRef, Diagnostic> {
    let bad = |message: String| task_error(message).with_hint(ref_hint().to_string());
    if let Some(task) = raw.strip_prefix("self#") {
        return Err(bad(format!(
            "`{raw}` names the current package; the local task is `{task}`"
        )));
    }
    if let Some(task) = raw.strip_prefix('#') {
        return Err(bad(format!(
            "`{raw}` names no package; the local task is `{task}`"
        )));
    }
    let hashes = raw.matches('#').count();
    if hashes > 1 {
        return Err(bad(format!("`{raw}` contains more than one `#`")));
    }
    if let Some((package, task)) = raw.split_once('#') {
        if package.is_empty() || task.is_empty() {
            return Err(bad(format!("`{raw}` is not a task reference")));
        }
    }
    TaskRef::parse(raw).map_err(|e| bad(format!("`{raw}` is not a task reference: {e}")))
}

pub fn task_kind(def: &TaskDef) -> &'static str {
    match def {
        TaskDef::File { .. } => "file",
        TaskDef::Steps { matrix, .. } if !matrix.is_empty() => "matrix",
        TaskDef::Steps { .. } => "steps",
        TaskDef::Group { .. } => "metatask",
    }
}

pub fn list_tasks(cfg: &ProjectConfig) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = cfg
        .tasks
        .iter()
        .map(|(name, def)| {
            let detail = match def {
                TaskDef::File { path, .. } => format!("{path}"),
                TaskDef::Steps { steps, matrix, .. } => {
                    if matrix.is_empty() {
                        format!("{} step(s)", steps.len())
                    } else {
                        format!("{} step(s) x {} target(s)", steps.len(), matrix.len())
                    }
                }
                TaskDef::Group { tasks, strategy, on_failure, .. } => {
                    let s = match strategy {
                        TaskStrategy::Serial => "serial",
                        TaskStrategy::Parallel => "parallel",
                    };
                    let f = match on_failure {
                        TaskOnFailure::Stop => "stop",
                        TaskOnFailure::Continue => "continue",
                    };
                    format!("{} task(s), {s}, on-failure {f}", tasks.len())
                }
            };
            (name.clone(), format!("{} ({detail})", task_kind(def)))
        })
        .collect();
    out.sort();
    out
}

pub struct TaskProject {
    pub dir: PathBuf,
    pub scope_root: PathBuf,
    pub config: ProjectConfig,
    pub text: String,
}

impl TaskProject {
    pub fn load(dir: PathBuf, scope_root: PathBuf) -> Result<TaskProject, Diagnostic> {
        let manifest = frontend::project::load_manifest(&dir)?.ok_or_else(|| {
            task_error(format!("no {} in {}", frontend::project::MANIFEST_FILE, dir.display()))
        })?;
        let config = manifest.project.ok_or_else(|| {
            task_error(format!(
                "{} has no `project` table; tasks live beside `project`",
                dir.display()
            ))
        })?;
        let path = dir.join(frontend::project::MANIFEST_FILE);
        let text = std::fs::read_to_string(&path)
            .map_err(|e| task_error(format!("cannot read {}: {e}", path.display())))?;
        Ok(TaskProject { dir, scope_root, config, text })
    }
}

struct PackageEntry {
    name: String,
    version: String,
    dir: PathBuf,
    config: ProjectConfig,
    text: String,
    trusted: bool,
}

pub struct TaskRegistry {
    pub project_dir: PathBuf,
    pub scope_root: PathBuf,
    pub selected: String,
    packages: BTreeMap<String, PackageEntry>,
    unresolved: BTreeMap<String, String>,
}

fn package_dirs(scope_root: &Path) -> Result<BTreeMap<String, (PathBuf, ProjectConfig)>, Diagnostic> {
    let manifest = match frontend::project::load_manifest(scope_root)? {
        Some(m) => m,
        None => return Ok(BTreeMap::new()),
    };
    match manifest.workspace {
        Some(ws) => frontend::project::resolve_workspace_members(scope_root, &ws),
        None => Ok(BTreeMap::new()),
    }
}

fn canonical_or(dir: PathBuf) -> PathBuf {
    std::fs::canonicalize(&dir).unwrap_or(dir)
}

fn cached_registry_dir(project: &TaskProject, name: &str) -> Option<PathBuf> {
    let lock = frontend::deplock::ProjectDepLock::load(&project.scope_root).ok().flatten()?;
    let pinned = lock.packages.iter().find(|p| p.name == name)?;
    let base = frontend::fetch::registry_base_for(
        name,
        project.config.registry.as_ref(),
        &project.config.registries,
    );
    frontend::fetch::find_cached_package_dir(&base, name, &pinned.version)
}

impl TaskRegistry {
    pub fn is_trusted(&self, package: &str) -> bool {
        self.packages.get(package).is_some_and(|entry| entry.trusted)
    }

    pub fn build(project: &TaskProject) -> Result<TaskRegistry, Diagnostic> {
        let selected = project.config.name.clone();
        let project_dir = canonical_or(project.dir.clone());
        let mut packages: BTreeMap<String, PackageEntry> = BTreeMap::new();
        packages.insert(
            selected.clone(),
            PackageEntry {
                name: selected.clone(),
                version: project.config.version.clone(),
                dir: project_dir.clone(),
                config: project.config.clone(),
                text: project.text.clone(),
                trusted: true,
            },
        );
        let mut unresolved: BTreeMap<String, String> = BTreeMap::new();

        for (name, (dir, cfg)) in package_dirs(&project.scope_root)? {
            if packages.contains_key(&name) {
                continue;
            }
            let dir = canonical_or(dir);
            let text = std::fs::read_to_string(dir.join(frontend::project::MANIFEST_FILE))
                .map_err(|e| task_error(format!("cannot read {}: {e}", dir.display())))?;
            packages.insert(
                name.clone(),
                PackageEntry {
                    name,
                    version: cfg.version.clone(),
                    dir,
                    config: cfg,
                    text,
                    trusted: true,
                },
            );
        }

        for (dep_name, spec) in &project.config.dependencies {
            if packages.contains_key(dep_name) {
                continue;
            }
            match spec {
                frontend::project::DependencySpec::Path { path, .. } => {
                    let dir = project.dir.join(path);
                    match ProjectConfig::load_from_dir(&dir) {
                        Ok(Some(cfg)) => {
                            if &cfg.name != dep_name {
                                unresolved.insert(
                                    dep_name.clone(),
                                    format!(
                                        "path dependency `{dep_name}` declares package name `{}` in {}",
                                        cfg.name,
                                        dir.display()
                                    ),
                                );
                                continue;
                            }
                            let text =
                                std::fs::read_to_string(dir.join(frontend::project::MANIFEST_FILE))
                                    .map_err(|e| {
                                        task_error(format!("cannot read {}: {e}", dir.display()))
                                    })?;
                            let key = cfg.name.clone();
                            let dir = canonical_or(dir);
                            packages.entry(key.clone()).or_insert(PackageEntry {
                                name: key,
                                version: cfg.version.clone(),
                                dir,
                                config: cfg,
                                text,
                                trusted: false,
                            });
                        }
                        Ok(None) => {
                            unresolved.insert(
                                dep_name.clone(),
                                format!(
                                    "path dependency `{dep_name}` has no {} in {}",
                                    frontend::project::MANIFEST_FILE,
                                    dir.display()
                                ),
                            );
                        }
                        Err(e) => {
                            unresolved.insert(dep_name.clone(), e.message.clone());
                        }
                    }
                }
                frontend::project::DependencySpec::Semver { .. } => {
                    let Some(dir) = cached_registry_dir(project, dep_name) else {
                        unresolved.insert(
                            dep_name.clone(),
                            format!(
                                "registry dependency `{dep_name}` is not fetched; run `rnx lock` and `rnx download` first"
                            ),
                        );
                        continue;
                    };
                    match ProjectConfig::load_from_dir(&dir) {
                        Ok(Some(cfg)) => {
                            if &cfg.name != dep_name {
                                unresolved.insert(
                                    dep_name.clone(),
                                    format!(
                                        "registry dependency `{dep_name}` declares package name `{}` in {}",
                                        cfg.name,
                                        dir.display()
                                    ),
                                );
                                continue;
                            }
                            let text = std::fs::read_to_string(
                                dir.join(frontend::project::MANIFEST_FILE),
                            )
                            .map_err(|e| {
                                task_error(format!("cannot read {}: {e}", dir.display()))
                            })?;
                            let key = cfg.name.clone();
                            let dir = canonical_or(dir);
                            packages.entry(key.clone()).or_insert(PackageEntry {
                                name: key,
                                version: cfg.version.clone(),
                                dir,
                                config: cfg,
                                text,
                                trusted: false,
                            });
                        }
                        Ok(None) => {
                            unresolved.insert(
                                dep_name.clone(),
                                format!(
                                    "registry dependency `{dep_name}` is missing its {}; run `rnx download`",
                                    frontend::project::MANIFEST_FILE
                                ),
                            );
                        }
                        Err(e) => {
                            unresolved.insert(dep_name.clone(), e.message.clone());
                        }
                    }
                }
                frontend::project::DependencySpec::Git { git, rev, .. } => {
                    unresolved.insert(
                        dep_name.clone(),
                        format!(
                            "git dependency `{dep_name}` ({git}#{rev}) has no fetched source; run `rnx lock` and `rnx download` first"
                        ),
                    );
                }
                frontend::project::DependencySpec::Url { url, .. } => {
                    unresolved.insert(
                        dep_name.clone(),
                        format!(
                            "tarball dependency `{dep_name}` ({url}) has no fetched source; run `rnx lock` and `rnx download` first"
                        ),
                    );
                }
                frontend::project::DependencySpec::Native { .. } => {}
            }
        }

        Ok(TaskRegistry {
            project_dir,
            scope_root: project.scope_root.clone(),
            selected,
            packages,
            unresolved,
        })
    }

    fn package(&self, name: &str) -> Result<&PackageEntry, Diagnostic> {
        if let Some(entry) = self.packages.get(name) {
            return Ok(entry);
        }
        if let Some(reason) = self.unresolved.get(name) {
            return Err(task_error(format!("package `{name}` is unavailable: {reason}")));
        }
        let mut names: Vec<&str> = self.packages.keys().map(String::as_str).collect();
        names.sort();
        let hint = if names.is_empty() {
            format!("available packages: only the selected package `{}`", self.selected)
        } else {
            format!(
                "available packages: {} (plus the selected package `{}`)",
                names.join(", "),
                self.selected
            )
        };
        Err(task_error(format!("unknown package `{name}` in a task reference"))
            .with_hint(format!(
                "{hint}; only direct dependencies and workspace members are visible (no transitive re-exports)"
            )))
    }
}

pub fn list_all_tasks(reg: &TaskRegistry) -> Vec<(String, String)> {
    let mut out: Vec<(String, String)> = Vec::new();
    for (pkg, entry) in &reg.packages {
        for (task, def) in &entry.config.tasks {
            if *pkg == reg.selected {
                let detail = list_detail(def);
                out.push((task.clone(), format!("{detail} [{} at {}]", entry.name, entry.dir.display())));
            } else if def.is_exported() {
                let detail = list_detail(def);
                out.push((
                    format!("{}#{task}", entry.name),
                    format!(
                        "{detail} [{} {} at {}]",
                        entry.name,
                        entry.version,
                        entry.dir.display()
                    ),
                ));
            }
        }
    }
    out.sort();
    out
}

fn list_detail(def: &TaskDef) -> String {
    let kind = task_kind(def);
    let description = def
        .meta()
        .description
        .as_deref()
        .map(|d| format!(" - {d}"))
        .unwrap_or_default();
    format!("{kind}{description}")
}

#[derive(Clone, Debug)]
struct GraphLeg {
    target: String,
    steps: Vec<GraphStep>,
}

#[derive(Clone, Debug)]
enum GraphStep {
    Shell(String),
    Task(GraphNode),
}

#[derive(Clone, Debug)]
enum GraphBody {
    File { path: PathBuf },
    Steps { legs: Vec<GraphLeg> },
    Group { children: Vec<GraphNode>, strategy: TaskStrategy, on_failure: TaskOnFailure },
    Bindgen,
}

#[derive(Clone, Debug)]
struct GraphNode {
    package: String,
    package_dir: PathBuf,
    version: String,
    name: String,
    meta: TaskMeta,
    cwd: PathBuf,
    body: GraphBody,
}

impl GraphNode {
    fn id(&self) -> String {
        format!("{}#{}", self.package, self.name)
    }
}

struct GraphBuilder<'a> {
    reg: &'a TaskRegistry,
    stack: Vec<String>,
}

impl<'a> GraphBuilder<'a> {
    fn new(reg: &'a TaskRegistry) -> GraphBuilder<'a> {
        GraphBuilder { reg, stack: Vec::new() }
    }

    fn top(&mut self, reference: &TaskRef) -> Result<GraphNode, Diagnostic> {
        let selected = self.reg.selected.clone();
        match reference {
            TaskRef::Local(name) => self.node(&selected, name, false),
            TaskRef::Imported { package, task } => self.node(package, task, true),
        }
    }

    fn resolve_ref(&mut self, from: &str, reference: &TaskRef) -> Result<GraphNode, Diagnostic> {
        match reference {
            TaskRef::Local(name) => self.node(from, name, false),
            TaskRef::Imported { package, task } => self.node(package, task, true),
        }
    }

    fn node(&mut self, package: &str, task: &str, require_export: bool) -> Result<GraphNode, Diagnostic> {
        let reg = self.reg;
        let entry = reg.package(package)?;
        let id = format!("{}#{}", entry.name, task);
        if self.stack.iter().any(|s| s == &id) {
            let mut chain = self.stack.clone();
            chain.push(id);
            return Err(task_error(format!("task cycle: {}", chain.join(" -> "))));
        }
        if task == BINDGEN_TASK && entry.name == reg.selected && !entry.config.tasks.contains_key(task) {
            return Ok(GraphNode {
                package: entry.name.clone(),
                package_dir: entry.dir.clone(),
                version: entry.version.clone(),
                name: task.to_string(),
                meta: TaskMeta::default(),
                cwd: entry.dir.clone(),
                body: GraphBody::Bindgen,
            });
        }
        let def = entry.config.tasks.get(task).ok_or_else(|| {
            let mut names: Vec<&str> = entry.config.tasks.keys().map(String::as_str).collect();
            names.sort();
            let detail = if names.is_empty() {
                "the package declares no tasks".to_string()
            } else {
                format!("available: {}", names.join(", "))
            };
            task_error(format!("unknown task `{task}` in package `{}`", entry.name))
                .with_hint(detail)
        })?;
        if require_export && entry.name != reg.selected && !def.is_exported() {
            return Err(task_error(format!(
                "task `{}#{task}` is not exported",
                entry.name
            ))
            .with_hint(format!(
                "add `export: true` and a `description` in {}",
                entry.dir.join(frontend::project::MANIFEST_FILE).display()
            )));
        }
        let cwd = match def.meta().runs_in {
            RunsIn::Package => entry.dir.clone(),
            RunsIn::Project => reg.project_dir.clone(),
        };
        self.stack.push(id);
        let result = self.body(entry, task, def, cwd);
        self.stack.pop();
        result
    }

    fn body(
        &mut self,
        entry: &'a PackageEntry,
        name: &str,
        def: &TaskDef,
        cwd: PathBuf,
    ) -> Result<GraphNode, Diagnostic> {
        let meta = def.meta().clone();
        let body = match def {
            TaskDef::File { path, .. } => {
                let joined = entry.dir.join(path);
                let abs = std::fs::canonicalize(&joined).map_err(|e| {
                    task_error(format!(
                        "task `{}#{}`: file `{path}` is not readable in {}: {e}",
                        entry.name,
                        name,
                        entry.dir.display()
                    ))
                })?;
                let root = std::fs::canonicalize(&entry.dir).map_err(|e| {
                    task_error(format!("cannot resolve {}: {e}", entry.dir.display()))
                })?;
                if !abs.starts_with(&root) {
                    return Err(task_error(format!(
                        "task `{}#{}`: file `{path}` escapes the package directory",
                        entry.name, name
                    )));
                }
                GraphBody::File { path: abs }
            }
            TaskDef::Steps { steps, matrix, .. } => {
                let mut legs = Vec::new();
                if matrix.is_empty() {
                    let target = frontend::native::host_triple();
                    legs.push(GraphLeg {
                        target: target.clone(),
                        steps: self.steps_for(entry, steps)?,
                    });
                } else {
                    for triple in matrix {
                        let target = frontend::native::triple_target(triple).ok_or_else(|| {
                            task_error(format!(
                                "task `{}#{name}`: bad matrix triple `{triple}`",
                                entry.name
                            ))
                        })?;
                        let defs =
                            frontend::project::parse_tasks_for_target(&entry.text, target).map_err(
                                |e| {
                                    task_error(format!(
                                        "task `{}#{name}`: cannot evaluate matrix leg `{triple}`: {e}",
                                        entry.name
                                    ))
                                },
                            )?;
                        let leg = defs.get(name).ok_or_else(|| {
                            task_error(format!(
                                "task `{}#{name}`: matrix leg `{triple}` drops the task (keep every leg total)",
                                entry.name
                            ))
                        })?;
                        match leg {
                            TaskDef::Steps { steps, .. } => legs.push(GraphLeg {
                                target: triple.clone(),
                                steps: self.steps_for(entry, steps)?,
                            }),
                            _ => {
                                return Err(task_error(format!(
                                    "task `{}#{name}`: matrix leg `{triple}` must stay a step list",
                                    entry.name
                                )));
                            }
                        }
                    }
                }
                GraphBody::Steps { legs }
            }
            TaskDef::Group { tasks, strategy, on_failure, .. } => {
                let mut children = Vec::with_capacity(tasks.len());
                for reference in tasks {
                    children.push(self.resolve_ref(&entry.name, reference)?);
                }
                GraphBody::Group {
                    children,
                    strategy: *strategy,
                    on_failure: *on_failure,
                }
            }
        };
        Ok(GraphNode {
            package: entry.name.clone(),
            package_dir: entry.dir.clone(),
            version: entry.version.clone(),
            name: name.to_string(),
            meta,
            cwd,
            body,
        })
    }

    fn steps_for(
        &mut self,
        entry: &'a PackageEntry,
        steps: &[TaskStep],
    ) -> Result<Vec<GraphStep>, Diagnostic> {
        let mut out = Vec::with_capacity(steps.len());
        for step in steps {
            match step {
                TaskStep::Shell(cmd) => out.push(GraphStep::Shell(cmd.clone())),
                TaskStep::Invoke(reference) => {
                    out.push(GraphStep::Task(self.resolve_ref(&entry.name, reference)?))
                }
            }
        }
        Ok(out)
    }
}

fn node_label(reg: &TaskRegistry, node: &GraphNode) -> String {
    if node.package == reg.selected {
        node.name.clone()
    } else {
        format!("{}#{}", node.package, node.name)
    }
}

fn children_of(node: &GraphNode) -> Vec<&GraphNode> {
    let mut out = Vec::new();
    match &node.body {
        GraphBody::Group { children, .. } => out.extend(children.iter()),
        GraphBody::Steps { legs } => {
            for leg in legs {
                for step in &leg.steps {
                    if let GraphStep::Task(child) = step {
                        out.push(child);
                    }
                }
            }
        }
        GraphBody::File { .. } | GraphBody::Bindgen => {}
    }
    out
}

fn print_graph(reg: &TaskRegistry, node: &GraphNode) {
    println!("{}", node_label(reg, node));
    print_children(reg, node, 1);
}

fn print_children(reg: &TaskRegistry, node: &GraphNode, depth: usize) {
    let indent = "  ".repeat(depth);
    for child in children_of(node) {
        println!("{indent}- {}", node_label(reg, child));
        print_children(reg, child, depth + 1);
    }
}

#[derive(Clone)]
enum LeafKind {
    File { path: PathBuf, sha256: String },
    Shell { text: String },
}

#[derive(Clone)]
struct Leaf {
    package: String,
    package_dir: PathBuf,
    version: String,
    task: String,
    imported: bool,
    needs_consent: bool,
    cwd: PathBuf,
    kind: LeafKind,
}

fn leaf_sha(kind: &LeafKind) -> String {
    match kind {
        LeafKind::File { sha256, .. } => sha256.clone(),
        LeafKind::Shell { text } => Sha256::hexdigest(text.as_bytes()),
    }
}

fn leaf_label(leaf: &Leaf) -> String {
    if leaf.imported {
        format!("{}#{}", leaf.package, leaf.task)
    } else {
        leaf.task.clone()
    }
}

fn file_leaf(node: &GraphNode, path: &Path) -> Result<Leaf, Diagnostic> {
    let bytes = std::fs::read(path).map_err(|e| {
        task_error(format!(
            "task `{}#{}`: cannot read task file {}: {e}",
            node.package,
            node.name,
            path.display()
        ))
    })?;
    Ok(Leaf {
        package: node.package.clone(),
        package_dir: node.package_dir.clone(),
        version: node.version.clone(),
        task: node.name.clone(),
        imported: false,
        needs_consent: false,
        cwd: node.cwd.clone(),
        kind: LeafKind::File { path: path.to_path_buf(), sha256: Sha256::hexdigest(&bytes) },
    })
}

fn collect_leaves(reg: &TaskRegistry, node: &GraphNode) -> Result<Vec<Leaf>, Diagnostic> {
    let mut out = Vec::new();
    collect_node_leaves(reg, node, &mut out)?;
    Ok(out)
}

fn collect_node_leaves(
    reg: &TaskRegistry,
    node: &GraphNode,
    out: &mut Vec<Leaf>,
) -> Result<(), Diagnostic> {
    let imported = node.package != reg.selected;
    let needs_consent = !reg.is_trusted(&node.package);
    match &node.body {
        GraphBody::File { path } => {
            let mut leaf = file_leaf(node, path)?;
            leaf.imported = imported;
            leaf.needs_consent = needs_consent;
            out.push(leaf);
        }
        GraphBody::Steps { legs } => {
            for leg in legs {
                for step in &leg.steps {
                    match step {
                        GraphStep::Shell(text) => out.push(Leaf {
                            package: node.package.clone(),
                            package_dir: node.package_dir.clone(),
                            version: node.version.clone(),
                            task: node.name.clone(),
                            imported,
                            needs_consent,
                            cwd: node.cwd.clone(),
                            kind: LeafKind::Shell { text: text.clone() },
                        }),
                        GraphStep::Task(child) => collect_node_leaves(reg, child, out)?,
                    }
                }
            }
        }
        GraphBody::Group { children, .. } => {
            for child in children {
                collect_node_leaves(reg, child, out)?;
            }
        }
        GraphBody::Bindgen => {}
    }
    Ok(())
}

fn leaf_block(leaf: &Leaf) -> String {
    let label = leaf_label(leaf);
    let mut out = String::new();
    match &leaf.kind {
        LeafKind::File { path, sha256 } => {
            let shown = path.strip_prefix(&leaf.package_dir).unwrap_or(path).display().to_string();
            let short = &sha256[..sha256.len().min(12)];
            out.push_str(&format!("{label} runs a Rasmalai file:\n"));
            out.push_str(&format!("  {shown}   sha256 {short}\n"));
            out.push_str(&format!("  in {}\n", leaf.cwd.display()));
        }
        LeafKind::Shell { text } => {
            out.push_str(&format!("{label} runs a shell script:\n"));
            for line in text.lines() {
                out.push_str(&format!("  {line}\n"));
            }
            out.push_str(&format!("  in {}\n", leaf.cwd.display()));
        }
    }
    out
}

fn print_leaves(leaves: &[&Leaf]) {
    for leaf in leaves {
        println!("{}", leaf_block(leaf));
    }
}

fn consent_key(leaf: &Leaf) -> (String, String, String, String) {
    (leaf.package.clone(), leaf.version.clone(), leaf.task.clone(), leaf_sha(&leaf.kind))
}

fn consent_path(reg: &TaskRegistry) -> PathBuf {
    reg.scope_root.join(".rnx").join("tasks.consent")
}

fn read_consent(reg: &TaskRegistry) -> Vec<(String, String, String, String)> {
    let Ok(text) = std::fs::read_to_string(consent_path(reg)) else {
        return Vec::new();
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let parts: Vec<&str> = line.split('\t').collect();
        if parts.len() == 4 && parts.iter().all(|p| !p.is_empty()) {
            out.push((
                parts[0].to_string(),
                parts[1].to_string(),
                parts[2].to_string(),
                parts[3].to_string(),
            ));
        }
    }
    out
}

fn write_consent(
    reg: &TaskRegistry,
    records: &[(String, String, String, String)],
) -> Result<(), Diagnostic> {
    if records.is_empty() {
        return Ok(());
    }
    let path = consent_path(reg);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| task_error(format!("cannot create {}: {e}", parent.display())))?;
    }
    let mut all = read_consent(reg);
    all.extend(records.iter().cloned());
    all.sort();
    all.dedup();
    let mut text = String::new();
    for (package, version, task, hash) in &all {
        text.push_str(&format!("{package}\t{version}\t{task}\t{hash}\n"));
    }
    std::fs::write(&path, text)
        .map_err(|e| task_error(format!("cannot write {}: {e}", path.display())))
}

fn node_ids(node: &GraphNode, out: &mut BTreeSet<String>) {
    out.insert(node.id());
    for child in children_of(node) {
        node_ids(child, out);
    }
}

fn gather_consent(
    reg: &TaskRegistry,
    top: &str,
    ids: &BTreeSet<String>,
    leaves: &[Leaf],
    allow: &[String],
    interactive: bool,
) -> Result<(), Diagnostic> {
    let imported: Vec<&Leaf> = leaves.iter().filter(|l| l.needs_consent).collect();
    if imported.is_empty() {
        return Ok(());
    }
    let records = read_consent(reg);
    let allow_graph = allow.iter().any(|entry| entry == top || ids.contains(entry));
    let mut new_records: Vec<(String, String, String, String)> = Vec::new();
    let mut missing: Vec<String> = Vec::new();
    for leaf in &imported {
        let key = consent_key(leaf);
        if records.contains(&key) || new_records.contains(&key) {
            continue;
        }
        if allow_graph {
            new_records.push(key);
            continue;
        }
        if !interactive {
            missing.push(leaf_label(leaf));
            continue;
        }
        println!("{}", leaf_block(leaf));
        print!("Run it? [y/N] ");
        use std::io::Write;
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().read_line(&mut answer).is_err() {
            return Err(task_error(format!(
                "cannot read consent for `{}`; pass `--allow {}`",
                leaf_label(leaf),
                leaf_label(leaf)
            )));
        }
        let answer = answer.trim();
        if answer.eq_ignore_ascii_case("y") || answer.eq_ignore_ascii_case("yes") {
            new_records.push(key);
        } else {
            return Err(task_error(format!(
                "`{}` was not approved; nothing was run",
                leaf_label(leaf)
            )));
        }
    }
    if !missing.is_empty() {
        missing.sort();
        missing.dedup();
        let flags: Vec<String> = missing.iter().map(|m| format!("--allow {m}")).collect();
        return Err(task_error(format!(
            "imported task consent is required for: {}",
            missing.join(", ")
        ))
        .with_hint(format!(
            "rerun with {} or set {ALLOW_ENV}=<comma-separated pkg#task>",
            flags.join(" ")
        )));
    }
    write_consent(reg, &new_records)
}

fn split_segments(text: &str) -> Vec<&str> {
    text.split('/').filter(|s| !s.is_empty() && *s != ".").collect()
}

fn glob_match(pattern: &str, path: &str) -> bool {
    matches_segments(&split_segments(pattern), &split_segments(path))
}

fn matches_segments(pattern: &[&str], parts: &[&str]) -> bool {
    match pattern.split_first() {
        None => parts.is_empty(),
        Some((&"**", rest)) => {
            if matches_segments(rest, parts) {
                return true;
            }
            match parts.split_first() {
                Some((_, tail)) => matches_segments(pattern, tail),
                None => false,
            }
        }
        Some((segment, rest)) => match parts.split_first() {
            Some((part, tail)) => match_segment(segment, part) && matches_segments(rest, tail),
            None => false,
        },
    }
}

fn match_segment(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.chars().collect();
    let t: Vec<char> = text.chars().collect();
    let (mut i, mut j, mut star, mut mark) = (0usize, 0usize, usize::MAX, 0usize);
    while j < t.len() {
        if i < p.len() && (p[i] == '?' || p[i] == t[j]) {
            i += 1;
            j += 1;
        } else if i < p.len() && p[i] == '*' {
            star = i;
            mark = j;
            i += 1;
        } else if star != usize::MAX {
            i = star + 1;
            mark += 1;
            j = mark;
        } else {
            return false;
        }
    }
    while i < p.len() && p[i] == '*' {
        i += 1;
    }
    i == p.len()
}

fn collect_matching(cwd: &Path, pattern: &str) -> Vec<PathBuf> {
    let mut out = Vec::new();
    if pattern.is_empty() {
        return out;
    }
    if !pattern.contains('*') && !pattern.contains('?') {
        let path = cwd.join(pattern);
        if path.is_file() {
            out.push(path);
        }
        return out;
    }
    let mut stack = vec![cwd.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            let Ok(file_type) = entry.file_type() else {
                continue;
            };
            if file_type.is_symlink() {
                continue;
            }
            let path = entry.path();
            if file_type.is_dir() {
                if name == "target" || name.starts_with('.') {
                    continue;
                }
                stack.push(path);
            } else if file_type.is_file() {
                if let Ok(rel) = path.strip_prefix(cwd) {
                    let rel = rel
                        .components()
                        .map(|c| c.as_os_str().to_string_lossy().into_owned())
                        .collect::<Vec<_>>()
                        .join("/");
                    if glob_match(pattern, &rel) {
                        out.push(path);
                    }
                }
            }
        }
    }
    out.sort();
    out
}

fn input_hash(node: &GraphNode) -> String {
    let mut files: Vec<PathBuf> = Vec::new();
    for pattern in &node.meta.inputs {
        files.extend(collect_matching(&node.cwd, pattern));
    }
    files.sort();
    files.dedup();
    let mut hasher = Sha256::new();
    for path in &files {
        let shown = path.to_string_lossy().replace('\\', "/");
        hasher.update(shown.as_bytes());
        hasher.update(&[0]);
        if let Ok(bytes) = std::fs::read(path) {
            hasher.update(&bytes);
        }
    }
    frontend::checksum::hex_of(&hasher.finish())
}

fn state_file_name(node: &GraphNode) -> String {
    let sanitize = |text: &str| -> String {
        text.chars()
            .map(|c| {
                if c.is_ascii_alphanumeric() || c == '-' || c == '_' || c == '.' || c == '@' {
                    c
                } else {
                    '_'
                }
            })
            .collect()
    };
    format!("{}__{}.state", sanitize(&node.package), sanitize(&node.name))
}

fn state_path(reg: &TaskRegistry, node: &GraphNode) -> PathBuf {
    reg.scope_root.join(".rnx").join("tasks").join(state_file_name(node))
}

fn read_state(path: &Path) -> Option<(String, Vec<String>)> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut lines = text.lines();
    let hash = lines.next()?.trim().to_string();
    if hash.is_empty() {
        return None;
    }
    Some((hash, lines.map(|l| l.to_string()).collect()))
}

fn write_state(
    reg: &TaskRegistry,
    node: &GraphNode,
    hash: &str,
) -> Result<(), Diagnostic> {
    let path = state_path(reg, node);
    if let Some(parent) = path.parent() {
        std::fs::create_dir_all(parent)
            .map_err(|e| task_error(format!("cannot create {}: {e}", parent.display())))?;
    }
    let mut text = format!("{hash}\n");
    for output in &node.meta.outputs {
        text.push_str(output);
        text.push('\n');
    }
    std::fs::write(&path, text)
        .map_err(|e| task_error(format!("cannot write {}: {e}", path.display())))
}

fn outputs_present(node: &GraphNode) -> bool {
    node.meta
        .outputs
        .iter()
        .all(|pattern| !collect_matching(&node.cwd, pattern).is_empty())
}

fn up_to_date(reg: &TaskRegistry, node: &GraphNode, force: bool) -> bool {
    if force {
        return false;
    }
    if node.meta.inputs.is_empty() && node.meta.outputs.is_empty() {
        return false;
    }
    let path = state_path(reg, node);
    let Some((recorded, _)) = read_state(&path) else {
        return false;
    };
    if recorded != input_hash(node) {
        return false;
    }
    outputs_present(node)
}

struct ExecCtx<'a> {
    reg: &'a TaskRegistry,
    force: bool,
    prefer_env: Option<&'a str>,
}

fn origin_of(reg: &TaskRegistry, node: &GraphNode) -> String {
    if node.package == reg.selected {
        node.name.clone()
    } else {
        node.id()
    }
}

fn apply_task_env(
    command: &mut std::process::Command,
    reg: &TaskRegistry,
    node: &GraphNode,
    target: &str,
    prefer_env: Option<&str>,
) {
    command.env("RNX_TASK", &node.name);
    command.env("RNX_TASK_ORIGIN", origin_of(reg, node));
    command.env("RNX_TASK_DIR", &node.cwd);
    command.env("RNX_TASK_PACKAGE_DIR", &node.package_dir);
    command.env("RNX_TARGET", target);
    if let Some(prefer) = prefer_env {
        command.env("RNX_PREFER_NATIVE", prefer);
    }
}

fn shell_program() -> &'static str {
    #[cfg(windows)]
    {
        "cmd"
    }
    #[cfg(not(windows))]
    {
        "sh"
    }
}

fn shell_flag() -> &'static str {
    #[cfg(windows)]
    {
        "/C"
    }
    #[cfg(not(windows))]
    {
        "-c"
    }
}

fn status_text(status: &std::process::ExitStatus) -> String {
    match status.code() {
        Some(code) => format!("exit code {code}"),
        None => "a signal".to_string(),
    }
}

fn status_result(label: &str, what: &str, status: std::process::ExitStatus) -> Result<(), Diagnostic> {
    if status.success() {
        return Ok(());
    }
    Err(task_error(format!("task `{label}`: {what} failed with {}", status_text(&status))))
}

fn run_shell(ctx: &ExecCtx<'_>, node: &GraphNode, text: &str, target: &str) -> Result<(), Diagnostic> {
    let mut command = std::process::Command::new(shell_program());
    command.arg(shell_flag()).arg(text);
    command.current_dir(&node.cwd);
    apply_task_env(&mut command, ctx.reg, node, target, ctx.prefer_env);
    let status = command.status().map_err(|e| {
        task_error(format!("task `{}`: cannot run `{text}`: {e}", node_label(ctx.reg, node)))
    })?;
    status_result(&node_label(ctx.reg, node), &format!("step `{text}`"), status)
}

fn file_command(ctx: &ExecCtx<'_>, node: &GraphNode, path: &Path) -> Result<std::process::Command, Diagnostic> {
    let exe = std::env::current_exe().map_err(|e| {
        task_error(format!("task `{}`: cannot locate the rnx binary: {e}", node_label(ctx.reg, node)))
    })?;
    let mut command = std::process::Command::new(exe);
    command.arg("run").arg(path);
    command.current_dir(&node.cwd);
    apply_task_env(&mut command, ctx.reg, node, &frontend::native::host_triple(), ctx.prefer_env);
    Ok(command)
}

fn run_file(ctx: &ExecCtx<'_>, node: &GraphNode, path: &Path) -> Result<(), Diagnostic> {
    let mut command = file_command(ctx, node, path)?;
    let status = command.status().map_err(|e| {
        task_error(format!(
            "task `{}`: cannot run {}: {e}",
            node_label(ctx.reg, node),
            path.display()
        ))
    })?;
    status_result(
        &node_label(ctx.reg, node),
        &format!("file {}", path.display()),
        status,
    )
}

fn execute(node: &GraphNode, ctx: &ExecCtx<'_>) -> Result<(), Diagnostic> {
    match &node.body {
        GraphBody::Group { children, strategy, on_failure } => {
            execute_group(node, children, *strategy, *on_failure, ctx)
        }
        GraphBody::Bindgen => {
            let entry = ctx.reg.package(&node.package)?;
            run_bindgen_driver(&entry.dir, &entry.config)
        }
        GraphBody::File { path } => {
            if up_to_date(ctx.reg, node, ctx.force) {
                println!("{}: up to date", node_label(ctx.reg, node));
                return Ok(());
            }
            run_file(ctx, node, path)?;
            finish_incremental(ctx, node)
        }
        GraphBody::Steps { legs } => {
            if up_to_date(ctx.reg, node, ctx.force) {
                println!("{}: up to date", node_label(ctx.reg, node));
                return Ok(());
            }
            for leg in legs {
                for step in &leg.steps {
                    match step {
                        GraphStep::Shell(text) => run_shell(ctx, node, text, &leg.target)?,
                        GraphStep::Task(child) => execute(child, ctx)?,
                    }
                }
            }
            finish_incremental(ctx, node)
        }
    }
}

fn finish_incremental(ctx: &ExecCtx<'_>, node: &GraphNode) -> Result<(), Diagnostic> {
    if node.meta.inputs.is_empty() && node.meta.outputs.is_empty() {
        return Ok(());
    }
    let hash = input_hash(node);
    write_state(ctx.reg, node, &hash)
}

fn execute_group(
    node: &GraphNode,
    children: &[GraphNode],
    strategy: TaskStrategy,
    on_failure: TaskOnFailure,
    ctx: &ExecCtx<'_>,
) -> Result<(), Diagnostic> {
    let label = node_label(ctx.reg, node);
    match strategy {
        TaskStrategy::Serial => {
            let mut failures: Vec<String> = Vec::new();
            for child in children {
                match execute(child, ctx) {
                    Ok(()) => {}
                    Err(e) => {
                        if matches!(on_failure, TaskOnFailure::Stop) {
                            return Err(e);
                        }
                        failures.push(format!("{}: {}", node_label(ctx.reg, child), e.message));
                    }
                }
            }
            if failures.is_empty() {
                Ok(())
            } else {
                Err(task_error(format!(
                    "task `{label}`: {} subtask(s) failed:\n{}",
                    failures.len(),
                    failures.join("\n")
                )))
            }
        }
        TaskStrategy::Parallel => {
            let outcomes: Vec<(String, Result<(), String>)> = std::thread::scope(|scope| {
                let mut handles = Vec::with_capacity(children.len());
                for child in children {
                    handles.push((
                        node_label(ctx.reg, child),
                        scope.spawn(move || execute(child, ctx).map_err(|e| e.message.clone())),
                    ));
                }
                handles
                    .into_iter()
                    .map(|(label, handle)| {
                        let outcome =
                            handle.join().unwrap_or_else(|_| Err("task thread panicked".to_string()));
                        (label, outcome)
                    })
                    .collect()
            });
            let failures: Vec<String> = outcomes
                .into_iter()
                .filter_map(|(label, result)| result.err().map(|m| format!("{label}: {m}")))
                .collect();
            if failures.is_empty() {
                Ok(())
            } else {
                Err(task_error(format!(
                    "task `{label}`: {} subtask(s) failed:\n{}",
                    failures.len(),
                    failures.join("\n")
                )))
            }
        }
    }
}

pub struct RunOptions {
    pub allow: Vec<String>,
    pub force: bool,
    pub explain: bool,
}

impl RunOptions {
    pub fn new(allow: Vec<String>, force: bool, explain: bool) -> RunOptions {
        RunOptions { allow, force, explain }
    }
}

fn prefer_env(prefer: frontend::native::NativePrefer) -> Option<&'static str> {
    match prefer {
        frontend::native::NativePrefer::System => Some("system"),
        frontend::native::NativePrefer::Bundled => Some("bundled"),
        frontend::native::NativePrefer::None => None,
    }
}

fn env_allow() -> Vec<String> {
    std::env::var(ALLOW_ENV)
        .map(|text| {
            text.split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default()
}

pub fn run_task(
    project: &TaskProject,
    raw: &str,
    options: &RunOptions,
    prefer: frontend::native::NativePrefer,
) -> Result<(), Diagnostic> {
    run_task_with(project, raw, options, prefer, std::io::stdin().is_terminal())
}

fn run_task_with(
    project: &TaskProject,
    raw: &str,
    options: &RunOptions,
    prefer: frontend::native::NativePrefer,
    interactive: bool,
) -> Result<(), Diagnostic> {
    let registry = TaskRegistry::build(project)?;
    let reference = parse_cli_ref(raw)?;
    let node = GraphBuilder::new(&registry).top(&reference)?;
    let leaves = collect_leaves(&registry, &node)?;
    if options.explain {
        print_graph(&registry, &node);
        let all: Vec<&Leaf> = leaves.iter().collect();
        print_leaves(&all);
        return Ok(());
    }
    let imported: Vec<&Leaf> = leaves.iter().filter(|l| l.imported).collect();
    if !imported.is_empty() {
        print_graph(&registry, &node);
        print_leaves(&imported);
    }
    let mut allow = options.allow.clone();
    allow.extend(env_allow());
    let mut ids = BTreeSet::new();
    node_ids(&node, &mut ids);
    gather_consent(&registry, &reference.display(), &ids, &leaves, &allow, interactive)?;
    let ctx = ExecCtx { reg: &registry, force: options.force, prefer_env: prefer_env(prefer) };
    execute(&node, &ctx)
}

pub fn explain_task(
    project: &TaskProject,
    raw: &str,
    prefer: frontend::native::NativePrefer,
) -> Result<(), Diagnostic> {
    let options = RunOptions::new(Vec::new(), false, true);
    run_task_with(project, raw, &options, prefer, false)
}

fn driver_options(
    root: &Path,
    pkg_name: &str,
    driver: &BindgenDriver,
    dir: &str,
    title: &str,
    target: &str,
) -> Result<frontend::bindgen::BindgenOptions, Diagnostic> {
    let native_lib = driver.native_lib.clone().ok_or_else(|| {
        Diagnostic::new(
            Code::E108,
            "task `bindgen`: the driver is missing `native_lib` (declare `bindgen: { ..., native_lib: \"foo\", ... }`)",
        )
    })?;
    let mut headers = Vec::with_capacity(driver.headers.len());
    for h in &driver.headers {
        let path = root.join(h);
        if !path.is_file() {
            return Err(Diagnostic::new(
                Code::E108,
                format!("task `bindgen`: header `{h}` does not exist (output `{dir}/{title}`)"),
            ));
        }
        headers.push(path);
    }
    Ok(frontend::bindgen::BindgenOptions {
        headers,
        include_dirs: driver.include.iter().map(|i| root.join(i)).collect(),
        native_lib,
        pkg_name: pkg_name.to_string(),
        target: target.to_string(),
        prefixes: driver.prefix.clone(),
        skips: driver.skip.clone(),
        case: match &driver.case {
            Some(s) => frontend::bindgen::CaseMode::parse(s).map_err(|e| {
                Diagnostic::new(Code::E108, format!("task `bindgen`: bad `case`: {e}"))
            })?,
            None => frontend::bindgen::CaseMode::Keep,
        },
        renames: {
            let mut out = Vec::with_capacity(driver.rename.len());
            for r in &driver.rename {
                out.push(frontend::bindgen::parse_rename(r).map_err(|e| {
                    Diagnostic::new(Code::E108, format!("task `bindgen`: bad `rename`: {e}"))
                })?);
            }
            out
        },
        upstream_name: driver.upstream.clone(),
        title: Some(title.to_string()),
    })
}

pub fn run_bindgen_driver(root: &Path, cfg: &ProjectConfig) -> Result<(), Diagnostic> {
    let driver = cfg.bindgen.as_ref().ok_or_else(|| {
        Diagnostic::new(
            Code::E108,
            "no `bindgen` driver table: declare `bindgen: { headers: [...], outputs: { <dir>: { title: ... } } }` or run `rnx task <name>`",
        )
    })?;
    for (dir, decl) in &driver.outputs {
        let target = decl
            .target
            .clone()
            .unwrap_or_else(frontend::native::host_triple);
        let opts = driver_options(root, &cfg.name, driver, dir, &decl.title, &target)?;
        let output = frontend::bindgen::generate(&opts).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: `{dir}/{}`: {e}", decl.title))
        })?;
        for warning in &output.warnings {
            eprintln!("warning: {warning}");
        }
        let out_dir = root.join(dir);
        std::fs::create_dir_all(&out_dir).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: cannot create `{}`: {e}", out_dir.display()))
        })?;
        let rnx_path = out_dir.join(&decl.title);
        std::fs::write(&rnx_path, &output.rnx).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: cannot write `{}`: {e}", rnx_path.display()))
        })?;
        let oci_path = out_dir.join("bindgen.oci.json");
        std::fs::write(&oci_path, &output.oci_json).map_err(|e| {
            Diagnostic::new(Code::E108, format!("task `bindgen`: cannot write `{}`: {e}", oci_path.display()))
        })?;
        eprintln!(
            "bindgen: wrote {} ({} functions, {} skipped) for {target}",
            rnx_path.display(),
            output.emitted,
            output.skipped.len()
        );
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn write_project(dir: &Path, body: &str) {
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(dir.join("Project.config"), body).unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
    }

    fn project_with(tag: &str, body: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("task-{tag}"));
        write_project(&dir, body);
        dir
    }

    fn context_of(dir: &Path) -> TaskProject {
        TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap()
    }

    fn ids_of(node: &GraphNode) -> BTreeSet<String> {
        let mut ids = BTreeSet::new();
        node_ids(node, &mut ids);
        ids
    }

    fn resolve(project: &TaskProject, raw: &str) -> Result<GraphNode, Diagnostic> {
        let registry = TaskRegistry::build(project)?;
        let reference = parse_cli_ref(raw)?;
        GraphBuilder::new(&registry).top(&reference)
    }

    fn run(project: &TaskProject, raw: &str) -> Result<(), Diagnostic> {
        let options = RunOptions::new(Vec::new(), false, false);
        run_task_with(project, raw, &options, frontend::native::NativePrefer::None, false)
    }

    fn dep_manifest(name: &str, version: &str, tasks: &str) -> String {
        format!(
            "export default {{\n    project: {{ name: \"{name}\", version: \"{version}\" }},\n    tasks: {tasks}\n}}\n"
        )
    }

    #[test]
    fn local_and_imported_refs_resolve_to_different_tasks() {
        let dir = crate::scratch::dir("task-local-imported");
        let dep = dir.join("dep");
        write_project(
            &dep,
            &dep_manifest(
                "dep",
                "0.1.0",
                "{ build: { steps: [{ shell: \"true\" }], export: true, description: \"Dep build\" } }",
            ),
        );
        write_project(
            &dir,
            &dep_manifest(
                "app",
                "0.1.0",
                "{ build: { steps: [{ shell: \"true\" }] }, dist: [\"dep#build\"] }",
            ),
        );
        let mut text = std::fs::read_to_string(dir.join("Project.config")).unwrap();
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        std::fs::write(dir.join("Project.config"), text).unwrap();
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();

        let local = resolve(&project, "build").unwrap();
        let imported = resolve(&project, "dep#build").unwrap();
        assert_eq!(local.package, "app");
        assert_eq!(imported.package, "dep");
        assert_ne!(local.package_dir, imported.package_dir);

        let dist = resolve(&project, "dist").unwrap();
        let children = children_of(&dist);
        assert_eq!(children.len(), 1);
        assert_eq!(children[0].package, "dep");
    }

    #[test]
    fn hash_prefixed_and_self_refs_are_rejected() {
        for raw in ["#icons", "self#icons", "a#b#c"] {
            let err = parse_cli_ref(raw).unwrap_err();
            assert_eq!(err.code, Code::E108);
            assert!(err.hint.as_deref().unwrap_or("").contains("local tasks"), "{raw}: {err}");
        }
        assert!(parse_cli_ref("#icons").unwrap_err().message.contains("names no package"));
        assert!(parse_cli_ref("self#icons").unwrap_err().message.contains("current package"));
        assert!(parse_cli_ref("a#b#c").unwrap_err().message.contains("more than one"));
        assert_eq!(parse_cli_ref("icons").unwrap(), TaskRef::Local("icons".to_string()));
        assert_eq!(
            parse_cli_ref("dep#icons").unwrap(),
            TaskRef::Imported { package: "dep".to_string(), task: "icons".to_string() }
        );
    }

    #[test]
    fn private_task_is_refused() {
        let dir = crate::scratch::dir("task-private");
        let dep = dir.join("dep");
        write_project(&dep, &dep_manifest("dep", "0.1.0", "{ secret: { steps: [{ shell: \"true\" }] } }"));
        let mut text = dep_manifest("app", "0.1.0", "{}");
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        write_project(&dir, &text);
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let err = resolve(&project, "dep#secret").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("not exported"), "{}", err.message);
        assert!(err.hint.as_deref().unwrap_or("").contains("export: true"));
    }

    #[test]
    fn cross_package_cycles_name_the_chain() {
        let dir = crate::scratch::dir("task-cross-cycle");
        let dep = dir.join("dep");
        write_project(
            &dep,
            &dep_manifest(
                "dep",
                "0.1.0",
                "{ b: { tasks: [\"app#a\"], export: true, description: \"B\" } }",
            ),
        );
        let mut text = dep_manifest(
            "app",
            "0.1.0",
            "{ a: { tasks: [\"dep#b\"], export: true, description: \"A\" } }",
        );
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        write_project(&dir, &text);
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let err = resolve(&project, "a").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("cycle"), "{}", err.message);
        assert!(err.message.contains("app#a"), "{}", err.message);
        assert!(err.message.contains("dep#b"), "{}", err.message);
    }

    #[test]
    fn file_body_uses_runs_in_directory_and_command_construction() {
        let dir = crate::scratch::dir("task-file-body");
        let dep = dir.join("dep");
        write_project(
            &dep,
            &dep_manifest(
                "dep",
                "0.1.0",
                "{ pack: { file: \"tools/pack.rnx\", export: true, description: \"Pack\" }, ship: { file: \"tools/pack.rnx\", runs_in: \"project\", export: true, description: \"Ship\" } }",
            ),
        );
        std::fs::create_dir_all(dep.join("tools")).unwrap();
        std::fs::write(dep.join("tools").join("pack.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        let mut text = dep_manifest("app", "0.1.0", "{}");
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        write_project(&dir, &text);
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let registry = TaskRegistry::build(&project).unwrap();
        let ctx = ExecCtx { reg: &registry, force: false, prefer_env: None };

        let pack = resolve(&project, "dep#pack").unwrap();
        assert_eq!(pack.cwd, dep.canonicalize().unwrap());
        let GraphBody::File { path } = &pack.body else {
            panic!("expected a file body");
        };
        let command = file_command(&ctx, &pack, path).unwrap();
        assert_eq!(command.get_current_dir(), Some(pack.cwd.as_path()));
        let args: Vec<String> = command
            .get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect();
        assert_eq!(args, vec!["run".to_string(), path.to_string_lossy().into_owned()]);
        let env: BTreeMap<String, String> = command
            .get_envs()
            .filter_map(|(k, v)| {
                v.map(|v| (k.to_string_lossy().into_owned(), v.to_string_lossy().into_owned()))
            })
            .collect();
        assert_eq!(env.get("RNX_TASK").map(String::as_str), Some("pack"));
        assert_eq!(env.get("RNX_TASK_ORIGIN").map(String::as_str), Some("dep#pack"));
        assert_eq!(env.get("RNX_TASK_DIR").map(String::as_str), Some(pack.cwd.to_str().unwrap()));
        assert_eq!(
            env.get("RNX_TASK_PACKAGE_DIR").map(String::as_str),
            Some(pack.package_dir.to_str().unwrap())
        );

        let ship = resolve(&project, "dep#ship").unwrap();
        assert_eq!(ship.cwd, dir.canonicalize().unwrap());
    }

    #[test]
    #[cfg(unix)]
    fn nonzero_exit_is_reported() {
        let status = std::process::Command::new("sh")
            .arg("-c")
            .arg("exit 7")
            .status()
            .unwrap();
        let err = status_result("demo", "file demo.rnx", status).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("exit code 7"), "{}", err.message);
    }

    fn consent_project(tag: &str) -> (crate::scratch::Scratch, PathBuf) {
        let dir = crate::scratch::dir(tag);
        let dep = dir.join("dep");
        write_project(
            &dep,
            &dep_manifest(
                "dep",
                "0.1.0",
                "{ build: { file: \"tools/build.rnx\", export: true, description: \"Build\" } }",
            ),
        );
        std::fs::create_dir_all(dep.join("tools")).unwrap();
        std::fs::write(dep.join("tools").join("build.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        let mut text = dep_manifest("app", "0.1.0", "{ dist: [\"dep#build\"] }");
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        write_project(&dir, &text);
        let script = dep.join("tools").join("build.rnx");
        (dir, script)
    }

    #[test]
    fn consent_is_content_keyed_and_invalidated_by_change() {
        let (dir, script) = consent_project("task-consent");
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let registry = TaskRegistry::build(&project).unwrap();
        let node = resolve(&project, "dist").unwrap();
        let leaves = collect_leaves(&registry, &node).unwrap();

        let err = gather_consent(&registry, "dist", &ids_of(&node), &leaves, &[], false).unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("dep#build"), "{}", err.message);
        assert!(
            err.hint.as_deref().unwrap_or("").contains("--allow dep#build"),
            "{err}"
        );

        gather_consent(&registry, "dist", &ids_of(&node), &leaves, &["dep#build".to_string()], false).unwrap();
        let text = std::fs::read_to_string(dir.join(".rnx").join("tasks.consent")).unwrap();
        assert!(text.contains("dep\t0.1.0\tbuild\t"), "{text}");
        assert_eq!(read_consent(&registry).len(), 1);
        gather_consent(&registry, "dist", &ids_of(&node), &leaves, &[], false).unwrap();

        std::fs::write(&script, "fn Main(): Int { return 1; }\n").unwrap();
        let changed = collect_leaves(&registry, &node).unwrap();
        let err = gather_consent(&registry, "dist", &ids_of(&node), &changed, &[], false).unwrap_err();
        assert!(err.message.contains("dep#build"), "{}", err.message);
        assert_eq!(read_consent(&registry).len(), 1);
    }

    #[test]
    fn allow_naming_a_group_approves_its_graph() {
        let dir = crate::scratch::dir("task-allow-group");
        let dep = dir.join("dep");
        write_project(
            &dep,
            &dep_manifest(
                "dep",
                "0.1.0",
                "{ inner: { file: \"tools/inner.rnx\", export: true, description: \"Inner\" }, bundle: { tasks: [\"inner\"], export: true, description: \"Bundle\" } }",
            ),
        );
        std::fs::create_dir_all(dep.join("tools")).unwrap();
        std::fs::write(dep.join("tools").join("inner.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        let mut text = dep_manifest("app", "0.1.0", "{ dist: [\"dep#bundle\"] }");
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        write_project(&dir, &text);
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let registry = TaskRegistry::build(&project).unwrap();
        let node = resolve(&project, "dist").unwrap();
        let leaves = collect_leaves(&registry, &node).unwrap();
        assert_eq!(leaves.iter().filter(|l| l.imported).count(), 1);

        let err = gather_consent(&registry, "dist", &ids_of(&node), &leaves, &[], false).unwrap_err();
        assert!(err.message.contains("dep#inner"), "{}", err.message);

        gather_consent(
            &registry,
            "dist",
            &ids_of(&node),
            &leaves,
            &["dep#bundle".to_string()],
            false,
        )
        .unwrap();
        let records = read_consent(&registry);
        assert_eq!(records.len(), 1);
        assert_eq!(records[0].2, "inner");
    }

    #[test]
    fn consent_record_round_trips() {
        let dir = crate::scratch::dir("task-consent-record");
        write_project(&dir, &dep_manifest("app", "0.1.0", "{}"));
        let project = context_of(&dir);
        let registry = TaskRegistry::build(&project).unwrap();
        let record = ("dep".to_string(), "0.1.0".to_string(), "build".to_string(), "ab".repeat(32));
        write_consent(&registry, std::slice::from_ref(&record)).unwrap();
        assert_eq!(read_consent(&registry), vec![record.clone()]);
        write_consent(&registry, std::slice::from_ref(&record)).unwrap();
        assert_eq!(read_consent(&registry), vec![record]);
    }

    #[test]
    #[cfg(unix)]
    fn incremental_skips_up_to_date_outputs_and_force_reruns() {
        let dir = project_with(
            "task-incremental",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        build: { steps: [{ shell: \"touch out.txt\" }], outputs: [\"out.txt\"] }\n    }\n}\n",
        );
        let project = context_of(&dir);
        let registry = TaskRegistry::build(&project).unwrap();
        let node = resolve(&project, "build").unwrap();
        let ctx = ExecCtx { reg: &registry, force: false, prefer_env: None };

        assert!(!up_to_date(&registry, &node, false));
        run(&project, "build").unwrap();
        assert!(dir.join("out.txt").is_file());
        assert!(up_to_date(&registry, &node, false));
        assert!(!up_to_date(&registry, &node, true));

        std::fs::remove_file(dir.join("out.txt")).unwrap();
        assert!(!up_to_date(&registry, &node, false));

        let forced = ExecCtx { reg: &registry, force: true, prefer_env: None };
        execute(&node, &forced).unwrap();
        assert!(dir.join("out.txt").is_file());
        assert!(!up_to_date(&registry, &node, true));
        let _ = ctx;
    }

    #[test]
    fn unknown_task_names_available_tasks() {
        let dir = project_with(
            "unknown",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { check: { steps: [{ shell: \"true\" }] } }\n}\n",
        );
        let err = run(&context_of(&dir), "missing").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("unknown task"), "{}", err.message);
    }

    #[test]
    #[cfg(unix)]
    fn serial_group_stops_on_first_failure() {
        let dir = project_with(
            "serial",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        first: { steps: [{ shell: \"touch first.done\" }] },\n        bad: { steps: [{ shell: \"exit 3\" }] },\n        last: { steps: [{ shell: \"touch last.done\" }] },\n        all: { tasks: [\"first\", \"bad\", \"last\"] }\n    }\n}\n",
        );
        let err = run(&context_of(&dir), "all").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("exit code 3"), "{}", err.message);
        assert!(dir.join("first.done").is_file());
        assert!(!dir.join("last.done").exists());
    }

    #[test]
    #[cfg(unix)]
    fn group_continue_runs_everything_and_reports() {
        let dir = project_with(
            "continue",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        bad: { steps: [{ shell: \"exit 3\" }] },\n        last: { steps: [{ shell: \"touch last.done\" }] },\n        all: { tasks: [\"bad\", \"last\"], on_failure: \"continue\" }\n    }\n}\n",
        );
        let err = run(&context_of(&dir), "all").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("bad"), "{}", err.message);
        assert!(dir.join("last.done").is_file());
    }

    #[test]
    #[cfg(unix)]
    fn parallel_group_runs_all_branches() {
        let dir = project_with(
            "parallel",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        a: { steps: [{ shell: \"touch a.done\" }] },\n        b: { steps: [{ shell: \"touch b.done\" }] },\n        all: { tasks: [\"a\", \"b\"], strategy: \"parallel\" }\n    }\n}\n",
        );
        run(&context_of(&dir), "all").unwrap();
        assert!(dir.join("a.done").is_file());
        assert!(dir.join("b.done").is_file());
    }

    #[test]
    fn cycles_fail_closed() {
        let dir = project_with(
            "cycle",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        a: [\"b\"],\n        b: [\"a\"]\n    }\n}\n",
        );
        let err = run(&context_of(&dir), "a").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("cycle"), "{}", err.message);
    }

    #[test]
    #[cfg(unix)]
    fn bindgen_driver_generates_per_output_titles() {
        let dir = project_with(
            "driver",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: {\n        headers: [\"include/add.h\"],\n        prefix: [\"demo_\"],\n        native_lib: \"demo\",\n        outputs: { \"src/gen\": { title: \"demo.rnx\" } }\n    }\n}\n",
        );
        std::fs::create_dir_all(dir.join("include")).unwrap();
        std::fs::write(dir.join("include").join("add.h"), "int demo_add(int a, int b);\n").unwrap();
        run(&context_of(&dir), "bindgen").unwrap();
        let generated = std::fs::read_to_string(dir.join("src").join("gen").join("demo.rnx")).unwrap();
        assert!(generated.contains("from native \"demo\""), "{generated}");
        let oci = std::fs::read_to_string(dir.join("src").join("gen").join("bindgen.oci.json")).unwrap();
        assert!(oci.contains("demo.rnx"), "{oci}");
        assert!(oci.contains("run.rasmalai.role"), "{oci}");
    }

    #[test]
    #[cfg(unix)]
    fn bindgen_driver_applies_case_and_rename() {
        let dir = project_with(
            "driver-case",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: {\n        headers: [\"include/add.h\"],\n        prefix: [\"demo_\"],\n        case: \"camel\",\n        rename: [\"demo_add=addNums\"],\n        native_lib: \"demo\",\n        outputs: { \"src/gen\": { title: \"demo.rnx\" } }\n    }\n}\n",
        );
        std::fs::create_dir_all(dir.join("include")).unwrap();
        std::fs::write(
            dir.join("include").join("add.h"),
            "int demo_add(int first_value, int second_value);\nint demo_sub(int x, int y);\n",
        )
        .unwrap();
        run(&context_of(&dir), "bindgen").unwrap();
        let generated = std::fs::read_to_string(dir.join("src").join("gen").join("demo.rnx")).unwrap();
        assert!(generated.contains("unsafe fn addNums(firstValue: Int, secondValue: Int)"), "{generated}");
        assert!(
            generated.contains("fn demo_add(firstValue: Int, secondValue: Int): Int as __ffi_demo_add"),
            "{generated}"
        );
        assert!(generated.contains("unsafe fn demoSub(x: Int, y: Int)"), "{generated}");
    }

    #[test]
    #[cfg(unix)]
    fn matrix_evaluates_steps_per_leg() {
        let dir = project_with(
            "matrix",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        build: {\n            steps: [{ shell: target.os == \"linux\" ? \"touch linux.done\" : \"touch other.done\" }],\n            matrix: [\"x86_64-unknown-linux-gnu\", \"aarch64-apple-darwin\"]\n        }\n    }\n}\n",
        );
        run(&context_of(&dir), "build").unwrap();
        assert!(dir.join("linux.done").is_file());
        assert!(dir.join("other.done").is_file());
    }

    #[test]
    fn bindgen_task_needs_driver_table() {
        let dir = project_with(
            "nodriver",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { check: { steps: [{ shell: \"true\" }] } }\n}\n",
        );
        let err = run(&context_of(&dir), "bindgen").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("driver"), "{}", err.message);
    }

    #[test]
    fn listing_names_kinds() {
        let dir = project_with(
            "list",
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: {\n        check: { steps: [{ shell: \"true\" }] },\n        docs: { file: \"src/main.rnx\" },\n        all: { tasks: [\"check\"], strategy: \"parallel\" }\n    }\n}\n",
        );
        let cfg = frontend::project::load_manifest(&dir).unwrap().unwrap().project.unwrap();
        let listed = list_tasks(&cfg);
        assert_eq!(listed.len(), 3);
        assert!(listed.iter().any(|(name, kind)| name == "docs" && kind.starts_with("file")));
        assert!(listed.iter().any(|(name, kind)| name == "all" && kind.starts_with("metatask")));
        assert!(listed.iter().any(|(name, kind)| name == "check" && kind.starts_with("steps")));
    }

    #[test]
    fn glob_matcher_handles_star_and_double_star() {
        assert!(glob_match("*.png", "a.png"));
        assert!(!glob_match("*.png", "sub/a.png"));
        assert!(glob_match("**/*.png", "sub/a.png"));
        assert!(glob_match("build/**", "build/icons/x.png"));
        assert!(glob_match("build/**", "build/x.png"));
        assert!(!glob_match("build/**", "buildx/x.png"));
        assert!(glob_match("a?c", "abc"));
    }

    #[test]
    fn workspace_member_resolves_without_dependency_line() {
        let dir = crate::scratch::dir("task-workspace");
        let member = dir.join("member");
        write_project(
            &member,
            &dep_manifest(
                "member",
                "0.1.0",
                "{ check: { steps: [{ shell: \"true\" }], export: true, description: \"Check\" } }",
            ),
        );
        write_project(
            &dir,
            &dep_manifest("root", "0.1.0", "{ all: [\"member#check\"] }"),
        );
        let mut text = std::fs::read_to_string(dir.join("Project.config")).unwrap();
        text = text.replace(
            "export default {",
            "export default {\n    workspace: { members: [\"member\"] },",
        );
        std::fs::write(dir.join("Project.config"), text).unwrap();
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let node = resolve(&project, "member#check").unwrap();
        assert_eq!(node.package, "member");
    }

    #[test]
    fn missing_package_lists_candidates() {
        let dir = crate::scratch::dir("task-missing-pkg");
        let dep = dir.join("dep");
        write_project(&dep, &dep_manifest("dep", "0.1.0", "{}"));
        let mut text = dep_manifest("app", "0.1.0", "{}");
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        write_project(&dir, &text);
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let err = resolve(&project, "nope#x").unwrap_err();
        assert_eq!(err.code, Code::E108);
        assert!(err.message.contains("unknown package"), "{}", err.message);
        assert!(err.hint.as_deref().unwrap_or("").contains("dep"), "{err}");
    }

    #[test]
    fn all_listing_marks_provenance_and_skips_private_tasks() {
        let dir = crate::scratch::dir("task-all");
        let dep = dir.join("dep");
        write_project(
            &dep,
            &dep_manifest(
                "dep",
                "1.2.3",
                "{ shown: { steps: [{ shell: \"true\" }], export: true, description: \"Shown\" }, hidden: { steps: [{ shell: \"true\" }] } }",
            ),
        );
        let mut text = dep_manifest("app", "0.1.0", "{ local: { steps: [{ shell: \"true\" }] } }");
        text = text.replace(
            "    tasks:",
            "    dependencies: { dep: { path: \"./dep\" } },\n    tasks:",
        );
        write_project(&dir, &text);
        let project = TaskProject::load(dir.to_path_buf(), dir.to_path_buf()).unwrap();
        let registry = TaskRegistry::build(&project).unwrap();
        let listed = list_all_tasks(&registry);
        let text = listed
            .iter()
            .map(|(name, detail)| format!("{name}: {detail}"))
            .collect::<Vec<_>>()
            .join("\n");
        assert!(text.contains("local: steps"), "{text}");
        assert!(text.contains("dep#shown: steps"), "{text}");
        assert!(!text.contains("dep#hidden"), "{text}");
    }

    #[test]
    fn merge_allow_list_does_not_duplicate() {
        let dir = project_with(
            "task-allow",
            "export default {\n    project: { name: \"app\", version: \"0.1.0\" },\n    tasks: { check: { steps: [{ shell: \"true\" }] } }\n}\n",
        );
        let project = context_of(&dir);
        let registry = TaskRegistry::build(&project).unwrap();
        let node = resolve(&project, "check").unwrap();
        let leaves = collect_leaves(&registry, &node).unwrap();
        gather_consent(&registry, "check", &ids_of(&node), &leaves, &[], false).unwrap();
        assert!(run(&project, "check").is_ok());
    }
}
