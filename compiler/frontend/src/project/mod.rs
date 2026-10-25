pub mod eval;
pub mod redact;

use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "Project.config";
pub const DEFAULT_ENTRY: &str = "src/main.rnx";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySpec {
    Semver { version: String, link: crate::license::LinkPreference },
    Path { path: PathBuf, link: crate::license::LinkPreference },
    Git { git: String, rev: String, link: crate::license::LinkPreference },
    Url { version: String, url: String, checksum: Option<String>, link: crate::license::LinkPreference },
    Native { lib: String, system: bool, path: Option<PathBuf>, version: Option<String>, from: Option<String>, link: crate::license::LinkPreference },
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct NativePin {
    pub system: bool,
    pub version: Option<String>,
    pub from: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskStep {
    Run(String),
    Invoke(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskStrategy {
    #[default]
    Serial,
    Parallel,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum TaskOnFailure {
    #[default]
    Stop,
    Continue,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TaskDef {
    Steps { steps: Vec<TaskStep>, matrix: Vec<String> },
    Group { tasks: Vec<String>, strategy: TaskStrategy, on_failure: TaskOnFailure },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BindgenOutputDecl {
    pub title: String,
    pub target: Option<String>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct BindgenDriver {
    pub headers: Vec<String>,
    pub include: Vec<String>,
    pub prefix: Vec<String>,
    pub skip: Vec<String>,
    pub native_lib: Option<String>,
    pub upstream: Option<String>,
    pub outputs: BTreeMap<String, BindgenOutputDecl>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct RegistryConfig {
    pub url: String,
    pub token_env: Option<String>,
    pub ca_cert: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Entries {
    pub main: String,
    pub lib: Option<String>,
    pub docs: Option<String>,
    pub bins: BTreeMap<String, String>,
}

impl Default for Entries {
    fn default() -> Self {
        Entries {
            main: DEFAULT_ENTRY.to_string(),
            lib: None,
            docs: None,
            bins: BTreeMap::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PermissionDecl {
    pub perm: String,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct ProjectConfig {
    pub name: String,
    pub version: String,
    pub description: String,
    pub engine: String,
    pub entries: Entries,
    pub registry: Option<RegistryConfig>,
    pub registries: BTreeMap<String, RegistryConfig>,
    pub dependencies: BTreeMap<String, DependencySpec>,
    pub permissions: Option<Vec<PermissionDecl>>,
    pub keywords: Vec<String>,
    pub artifacts: Option<String>,
    pub license: Option<String>,
    pub native_pins: BTreeMap<String, NativePin>,
    pub tasks: BTreeMap<String, TaskDef>,
    pub bindgen: Option<BindgenDriver>,
}

impl ProjectConfig {
    pub fn load_from_dir(dir: &Path) -> Result<Option<ProjectConfig>, Diagnostic> {
        let path = dir.join(MANIFEST_FILE);
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
        match parse_config(&text) {
            Ok(c) => Ok(Some(c)),
            Err(msg) => Err(Diagnostic::new(
                Code::E108,
                format!("{}: {msg}", path.display()),
            )
            .with_hint("Project.config is an `.rnx` module exporting a default object")),
        }
    }

    pub fn main_path(&self, root: &Path) -> PathBuf {
        root.join(&self.entries.main)
    }

    pub fn lib_path(&self, root: &Path) -> Option<PathBuf> {
        self.entries.lib.as_ref().map(|l| root.join(l))
    }

    pub fn bin_path(&self, root: &Path, name: &str) -> Option<PathBuf> {
        self.entries.bins.get(name).map(|b| root.join(b))
    }
}

#[derive(Debug, Clone, Default)]
pub struct WorkspaceConfig {
    pub members: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct Manifest {
    pub project: Option<ProjectConfig>,
    pub workspace: Option<WorkspaceConfig>,
}

pub fn load_manifest(dir: &Path) -> Result<Option<Manifest>, Diagnostic> {
    let path = dir.join(MANIFEST_FILE);
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
    match parse_manifest(&text) {
        Ok((project, workspace)) => Ok(Some(Manifest { project, workspace })),
        Err(msg) => Err(Diagnostic::new(
            Code::E108,
            format!("{}: {msg}", path.display()),
        )
        .with_hint("Project.config is an `.rnx` module exporting a default object")),
    }
}

fn manifest_has_workspace(dir: &Path) -> bool {
    match load_manifest(dir) {
        Ok(Some(m)) => m.workspace.is_some(),
        _ => false,
    }
}

pub fn find_workspace_root_strict(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    if dir.is_relative() {
        dir = std::env::current_dir().ok()?.join(dir);
    }
    loop {
        if dir.join(MANIFEST_FILE).is_file() && manifest_has_workspace(&dir) {
            return Some(std::fs::canonicalize(&dir).unwrap_or(dir));
        }
        dir = dir.parent()?.to_path_buf();
    }
}

pub fn find_workspace_root(start: &Path) -> Option<PathBuf> {
    match find_workspace_root_strict(start) {
        Some(r) => Some(r),
        None => find_project_root(start),
    }
}

pub fn resolve_workspace_members(
    ws_root: &Path,
    ws: &WorkspaceConfig,
) -> Result<BTreeMap<String, (PathBuf, ProjectConfig)>, Diagnostic> {
    let mut members: BTreeMap<String, (PathBuf, ProjectConfig)> = BTreeMap::new();
    for pattern in &ws.members {
        let mut roots: Vec<PathBuf> = Vec::new();
        if let Some(prefix) = pattern.strip_suffix("/*") {
            let base = ws_root.join(prefix);
            let mut names: Vec<String> = Vec::new();
            let entries = std::fs::read_dir(&base).map_err(|_| {
                Diagnostic::new(Code::E108, format!("workspace member `{pattern}` not found"))
            })?;
            for entry in entries {
                let entry = entry.map_err(|e| {
                    Diagnostic::new(Code::E108, format!("cannot read workspace member `{pattern}`: {e}"))
                })?;
                if entry.file_type().map(|t| t.is_dir()).unwrap_or(false)
                    && entry.path().join(MANIFEST_FILE).is_file()
                {
                    names.push(entry.file_name().to_string_lossy().into_owned());
                }
            }
            names.sort();
            for name in names {
                roots.push(base.join(name));
            }
        } else {
            roots.push(ws_root.join(pattern));
        }
        for root in roots {
            let cfg = ProjectConfig::load_from_dir(&root)?.ok_or_else(|| {
                Diagnostic::new(
                    Code::E108,
                    format!("workspace member `{pattern}` has no Project.config"),
                )
            })?;
            if cfg.name.is_empty() {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("workspace member `{pattern}` has an empty name"),
                ));
            }
            let root = std::fs::canonicalize(&root).unwrap_or(root);
            if members.contains_key(&cfg.name) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("duplicate workspace member `{}`", cfg.name),
                ));
            }
            members.insert(cfg.name.clone(), (root, cfg));
        }
    }
    Ok(members)
}

pub fn find_project_root(start: &Path) -> Option<PathBuf> {
    let mut dir = if start.is_file() {
        start.parent()?.to_path_buf()
    } else {
        start.to_path_buf()
    };
    if dir.is_relative() {
        dir = std::env::current_dir().ok()?.join(dir);
    }
    loop {
        if dir.join(MANIFEST_FILE).is_file() {
            return Some(std::fs::canonicalize(&dir).unwrap_or(dir));
        }
        dir = dir.parent()?.to_path_buf();
    }
}

use self::eval::{ConfigTarget, ConfigValue, eval_module};
use crate::parser::Parser;

fn eval_config_text_with(text: &str, target: ConfigTarget) -> Result<ConfigValue, String> {
    let module = Parser::parse_module(text).map_err(|e| e.message.clone())?;
    eval_module(&module, target).map_err(|e| e.message.clone())
}

fn obj_get<'a>(v: &'a ConfigValue, key: &str) -> Option<&'a ConfigValue> {
    v.get(key)
}

fn req_str(obj: &ConfigValue, key: &str) -> Result<String, String> {
    match obj_get(obj, key) {
        Some(ConfigValue::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("field `{key}` must be a string")),
        None => Err(format!("missing required field `{key}`")),
    }
}

fn opt_str(obj: &ConfigValue, key: &str, default: &str) -> Result<String, String> {
    match obj_get(obj, key) {
        Some(ConfigValue::String(s)) => Ok(s.clone()),
        Some(_) => Err(format!("field `{key}` must be a string")),
        None => Ok(default.to_string()),
    }
}

pub fn validate_artifact_ref(value: &str) -> Result<String, String> {
    if value.is_empty() {
        return Err("field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: \"ghcr.io/org/sqlite3\"`); digest-bearing or keyed `artifacts` tables are rejected".to_string());
    }
    if value.contains('@') || value.contains("sha256:") {
        return Err("field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: \"ghcr.io/org/sqlite3\"`); digest-bearing or keyed `artifacts` tables are rejected".to_string());
    }
    if value.chars().any(|c| c.is_whitespace() || c.is_ascii_uppercase()) {
        return Err("field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: \"ghcr.io/org/sqlite3\"`); digest-bearing or keyed `artifacts` tables are rejected".to_string());
    }
    let mut parts = value.split('/');
    let host = parts.next().unwrap_or_default();
    let rest: Vec<&str> = parts.collect();
    if !host.contains('.') || rest.is_empty() {
        return Err("field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: \"ghcr.io/org/sqlite3\"`); digest-bearing or keyed `artifacts` tables are rejected".to_string());
    }
    let host_ok = host
        .split(':')
        .enumerate()
        .all(|(i, seg)| {
            if i == 1 {
                !seg.is_empty() && seg.bytes().all(|b| b.is_ascii_digit())
            } else {
                !seg.is_empty()
                    && seg.bytes().all(|b| {
                        b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'-'
                    })
            }
        })
        && host.matches(':').count() <= 1;
    let seg_ok = |seg: &str| {
        !seg.is_empty()
            && seg.bytes().all(|b| {
                b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'.' || b == b'_' || b == b'-'
            })
    };
    if !host_ok || rest.iter().any(|s| !seg_ok(s)) {
        return Err("field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: \"ghcr.io/org/sqlite3\"`); digest-bearing or keyed `artifacts` tables are rejected".to_string());
    }
    if let Some(last) = rest.last()
        && last.contains(':')
    {
        return Err("field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: \"ghcr.io/org/sqlite3\"`); digest-bearing or keyed `artifacts` tables are rejected".to_string());
    }
    Ok(value.to_string())
}

pub fn parse_artifact_decl(val: &ConfigValue) -> Result<Option<String>, String> {
    match val {
        ConfigValue::String(s) => Ok(Some(validate_artifact_ref(s)?)),
        _ => Err("field `artifacts` must be a bare OCI repo ref string (e.g. `artifacts: \"ghcr.io/org/sqlite3\"`); digest-bearing or keyed `artifacts` tables are rejected".to_string()),
    }
}

fn parse_link_value(name: &str, val: &ConfigValue) -> Result<crate::license::LinkPreference, String> {
    match obj_get(val, "link") {
        None => Ok(crate::license::LinkPreference::Auto),
        Some(ConfigValue::String(s)) => crate::license::LinkPreference::parse(s)
            .map_err(|e| format!("dependency `{name}`: {e}")),
        Some(_) => Err(format!("dependency `{name}`: `link` must be a string")),
    }
}

fn parse_dependency(name: &str, val: &ConfigValue) -> Result<DependencySpec, String> {
    match val {
        ConfigValue::String(s) => {
            if s.starts_with('.') || s.contains('/') {
                return Ok(DependencySpec::Path {
                    path: PathBuf::from(s),
                    link: crate::license::LinkPreference::Auto,
                });
            }
            if s.is_empty() {
                return Err(format!("dependency `{name}`: version must not be empty"));
            }
            Ok(DependencySpec::Semver { version: s.clone(), link: crate::license::LinkPreference::Auto })
        }
        ConfigValue::Object(fields) => {
            let get_str = |key: &str| match obj_get(val, key) {
                Some(ConfigValue::String(s)) => Some(s.clone()),
                _ => None,
            };
            let _ = fields;
            let has = |key: &str| obj_get(val, key).is_some();
            let has_path = has("path");
            let has_git = has("git");
            let has_url = has("url");
            let has_version = has("version");
            let has_native = has("native");
            if has_native {
                if has_git || has_url {
                    return Err(format!("dependency `{name}`: `native` must not mix with `git` or `url`"));
                }
                let lib = match obj_get(val, "native") {
                    Some(ConfigValue::String(s)) if !s.is_empty() => s.clone(),
                    _ => return Err(format!("dependency `{name}`: `native` must be a non-empty string")),
                };
                let system = match obj_get(val, "system") {
                    Some(ConfigValue::Bool(b)) => *b,
                    Some(_) => return Err(format!("dependency `{name}`: `system` must be a boolean")),
                    None => false,
                };
                let path = match obj_get(val, "path") {
                    Some(ConfigValue::String(s)) => Some(PathBuf::from(s)),
                    Some(_) => return Err(format!("dependency `{name}`: `path` must be a string")),
                    None => None,
                };
                let version = match obj_get(val, "version") {
                    Some(ConfigValue::String(s)) => {
                        if !crate::fetch::is_valid_range(s) {
                            return Err(format!("dependency `{name}`: `version` `{s}` is not a valid range (expected `*`, `^1.2.0`, `~1.2.0`, `>=1.2.0`, or an exact `1.2.0`)"));
                        }
                        Some(s.clone())
                    }
                    Some(_) => return Err(format!("dependency `{name}`: `version` must be a string")),
                    None => None,
                };
                let from = match obj_get(val, "from") {
                    Some(ConfigValue::String(s)) if !s.is_empty() => Some(s.clone()),
                    Some(_) => return Err(format!("dependency `{name}`: `from` must be a non-empty string")),
                    None => None,
                };
                if system && from.is_some() {
                    return Err(format!("dependency `{name}`: `system: true` and `from` cannot combine (host libraries have no image titles)"));
                }
                if path.is_some() && system {
                    return Err(format!("dependency `{name}`: vendored `path` and `system: true` cannot combine (pick one source)"));
                }
                if path.is_some() && version.is_some() {
                    return Err(format!("dependency `{name}`: vendored `path` and `version` cannot combine (a local checkout carries no version to check)"));
                }
                if path.is_some() && from.is_some() {
                    return Err(format!("dependency `{name}`: vendored `path` and `from` cannot combine (titles live in the declared `artifacts` image, not in a checkout)"));
                }
                let link = parse_link_value(name, val)?;
                return Ok(DependencySpec::Native { lib, system, path, version, from, link });
            }
            let kinds = [has_path, has_git, has_url || (has_version && !has_git && !has_path)].iter().filter(|&&b| b).count();
            if kinds > 1 {
                return Err(format!("dependency `{name}`: specify only one of `path`, `git`, `url`, or `version`"));
            }
            if has_path {
                if has_git || has_url || has_version {
                    return Err(format!("dependency `{name}`: specify either `path`, `git`, or `version`/`url`, not a mix"));
                }
                let link = parse_link_value(name, val)?;
                if link == crate::license::LinkPreference::Dynamic {
                    return Err(format!("dependency `{name}`: `link: \"dynamic\"` needs a native dependency (path dependencies link statically)"));
                }
                return match obj_get(val, "path") {
                    Some(ConfigValue::String(s)) => Ok(DependencySpec::Path {
                        path: PathBuf::from(s),
                        link,
                    }),
                    _ => Err(format!("dependency `{name}`: `path` must be a string")),
                };
            }
            if has_git {
                let git = match obj_get(val, "git") {
                    Some(ConfigValue::String(s)) => s.clone(),
                    _ => return Err(format!("dependency `{name}`: `git` must be a string")),
                };
                let rev = get_str("rev")
                    .or_else(|| get_str("tag"))
                    .or_else(|| get_str("branch"));
                let rev = match rev {
                    Some(r) if !r.is_empty() => r,
                    _ => {
                        return Err(format!(
                            "dependency `{name}`: git dependencies require one of `rev`, `tag`, or `branch`"
                        ))
                    }
                };
                if obj_get(val, "rev").is_some() && matches!(rev.as_str(), "main" | "master" | "HEAD") {
                    return Err(format!(
                        "dependency `{name}`: pin a tag or commit in `rev`, not branch `{rev}` (use `branch = \"{rev}\"` for a moving target)"
                    ));
                }
                let link = parse_link_value(name, val)?;
                if link == crate::license::LinkPreference::Dynamic {
                    return Err(format!("dependency `{name}`: `link: \"dynamic\"` needs a native dependency (git dependencies link statically)"));
                }
                return Ok(DependencySpec::Git { git, rev, link });
            }
            if has_url {
                let url = match obj_get(val, "url") {
                    Some(ConfigValue::String(s)) => s.clone(),
                    _ => return Err(format!("dependency `{name}`: `url` must be a string")),
                };
                let version = get_str("version").unwrap_or_default();
                let checksum = get_str("checksum");
                let link = parse_link_value(name, val)?;
                if link == crate::license::LinkPreference::Dynamic {
                    return Err(format!("dependency `{name}`: `link: \"dynamic\"` needs a native dependency (tarball dependencies link statically)"));
                }
                return Ok(DependencySpec::Url { version, url, checksum, link });
            }
            if has_version {
                return match obj_get(val, "version") {
                    Some(ConfigValue::String(s)) => {
                        let link = parse_link_value(name, val)?;
                        if link == crate::license::LinkPreference::Dynamic {
                            return Err(format!("dependency `{name}`: `link: \"dynamic\"` needs a native dependency (registry sources link statically)"));
                        }
                        Ok(DependencySpec::Semver { version: s.clone(), link })
                    }
                    _ => Err(format!("dependency `{name}`: `version` must be a string")),
                };
            }
            Err(format!(
                "dependency `{name}`: expected a version string, `{{ path = ... }}`, `{{ git = ..., rev/tag/branch = ... }}`, `{{ version = ..., url = ... }}`, or `{{ native = ... }}`"
            ))
        }
        _ => Err(format!(
            "dependency `{name}`: expected a version string, path string, or inline object"
        )),
    }
}

fn parse_native_pin(name: &str, val: &ConfigValue) -> Result<NativePin, String> {
    let obj = match val {
        ConfigValue::Object(_) => val,
        _ => return Err(format!("field `native.{name}` must be an object (`{{ system = ... }}`)")),
    };
    for (key, _) in object_fields(obj) {
        if !["system", "version", "from"].contains(&key.as_str()) {
            return Err(format!("field `native.{name}.{key}` is not a pin field (expected `system`, `version`, or `from`)"));
        }
    }
    let system = match obj_get(obj, "system") {
        Some(ConfigValue::Bool(b)) => *b,
        Some(_) => return Err(format!("field `native.{name}.system` must be a boolean")),
        None => return Err(format!("field `native.{name}` is missing required field `system`")),
    };
    let version = match obj_get(obj, "version") {
        Some(ConfigValue::String(s)) => {
            if !crate::fetch::is_valid_range(s) {
                return Err(format!("field `native.{name}.version` `{s}` is not a valid range (expected `*`, `^1.2.0`, `~1.2.0`, `>=1.2.0`, or an exact `1.2.0`)"));
            }
            Some(s.clone())
        }
        Some(_) => return Err(format!("field `native.{name}.version` must be a string")),
        None => None,
    };
    let from = match obj_get(obj, "from") {
        Some(ConfigValue::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(_) => return Err(format!("field `native.{name}.from` must be a non-empty string")),
        None => None,
    };
    if system && from.is_some() {
        return Err(format!("field `native.{name}`: `system: true` and `from` cannot combine (host libraries have no image titles)"));
    }
    Ok(NativePin { system, version, from })
}

fn parse_task_step(task: &str, index: usize, val: &ConfigValue) -> Result<TaskStep, String> {
    match val {
        ConfigValue::String(s) if !s.is_empty() => Ok(TaskStep::Run(s.clone())),
        ConfigValue::String(_) => Err(format!("task `{task}` step {index}: command must not be empty")),
        ConfigValue::Object(_) => {
            for (key, _) in object_fields(val) {
                if !["run", "task"].contains(&key.as_str()) {
                    return Err(format!("task `{task}` step {index}: unknown field `{key}` (expected `run` or `task`)"));
                }
            }
            let has_run = obj_get(val, "run").is_some();
            let has_task = obj_get(val, "task").is_some();
            match (has_run, has_task) {
                (true, false) => match obj_get(val, "run") {
                    Some(ConfigValue::String(s)) if !s.is_empty() => Ok(TaskStep::Run(s.clone())),
                    _ => Err(format!("task `{task}` step {index}: `run` must be a non-empty string")),
                },
                (false, true) => match obj_get(val, "task") {
                    Some(ConfigValue::String(s)) if !s.is_empty() => Ok(TaskStep::Invoke(s.clone())),
                    _ => Err(format!("task `{task}` step {index}: `task` must be a non-empty task name")),
                },
                _ => Err(format!("task `{task}` step {index}: specify exactly one of `run` or `task`")),
            }
        }
        _ => Err(format!("task `{task}` step {index}: expected a command string or a `{{ run = ... }}` / `{{ task = ... }}` table")),
    }
}

fn parse_task_def(name: &str, val: &ConfigValue) -> Result<TaskDef, String> {
    match val {
        ConfigValue::Array(items) => {
            let mut steps = Vec::with_capacity(items.len());
            for (i, item) in items.iter().enumerate() {
                steps.push(parse_task_step(name, i, item)?);
            }
            Ok(TaskDef::Steps { steps, matrix: Vec::new() })
        }
        ConfigValue::Object(_) => {
            let has_steps = obj_get(val, "steps").is_some();
            let has_tasks = obj_get(val, "tasks").is_some();
            match (has_steps, has_tasks) {
                (true, false) => {
                    for (key, _) in object_fields(val) {
                        if !["steps", "matrix"].contains(&key.as_str()) {
                            return Err(format!("task `{name}.{key}` is not a step field (expected `steps` or `matrix`)"));
                        }
                    }
                    let items = match obj_get(val, "steps") {
                        Some(ConfigValue::Array(items)) => items,
                        _ => return Err(format!("task `{name}`: `steps` must be an array")),
                    };
                    let mut steps = Vec::with_capacity(items.len());
                    for (i, item) in items.iter().enumerate() {
                        steps.push(parse_task_step(name, i, item)?);
                    }
                    let matrix = match obj_get(val, "matrix") {
                        Some(ConfigValue::Array(targets)) => {
                            let mut out = Vec::with_capacity(targets.len());
                            for t in targets {
                                match t {
                                    ConfigValue::String(triple) if crate::oci::valid_target(triple) => out.push(triple.clone()),
                                    _ => return Err(format!("task `{name}`: `matrix` entries must be arch-vendor-os-env triples")),
                                }
                            }
                            out
                        }
                        Some(_) => return Err(format!("task `{name}`: `matrix` must be an array of target triples")),
                        None => Vec::new(),
                    };
                    Ok(TaskDef::Steps { steps, matrix })
                }
                (false, true) => {
                    for (key, _) in object_fields(val) {
                        if !["tasks", "strategy", "on-failure"].contains(&key.as_str()) {
                            return Err(format!("task `{name}.{key}` is not a metatask field (expected `tasks`, `strategy`, or `on-failure`)"));
                        }
                    }
                    let items = match obj_get(val, "tasks") {
                        Some(ConfigValue::Array(items)) if !items.is_empty() => items,
                        Some(ConfigValue::Array(_)) => return Err(format!("task `{name}`: `tasks` must not be empty")),
                        _ => return Err(format!("task `{name}`: `tasks` must be an array of task names")),
                    };
                    let mut tasks = Vec::with_capacity(items.len());
                    for item in items {
                        match item {
                            ConfigValue::String(s) if !s.is_empty() => tasks.push(s.clone()),
                            _ => return Err(format!("task `{name}`: `tasks` entries must be non-empty task names")),
                        }
                    }
                    let strategy = match obj_get(val, "strategy") {
                        Some(ConfigValue::String(s)) if s == "serial" => TaskStrategy::Serial,
                        Some(ConfigValue::String(s)) if s == "parallel" => TaskStrategy::Parallel,
                        Some(_) => return Err(format!("task `{name}`: `strategy` must be `serial` or `parallel`")),
                        None => TaskStrategy::Serial,
                    };
                    let on_failure = match obj_get(val, "on-failure") {
                        Some(ConfigValue::String(s)) if s == "stop" => TaskOnFailure::Stop,
                        Some(ConfigValue::String(s)) if s == "continue" => TaskOnFailure::Continue,
                        Some(_) => return Err(format!("task `{name}`: `on-failure` must be `stop` or `continue`")),
                        None => TaskOnFailure::Stop,
                    };
                    Ok(TaskDef::Group { tasks, strategy, on_failure })
                }
                _ => Err(format!("task `{name}`: specify exactly one of `steps` (or a bare step array) or `tasks`")),
            }
        }
        _ => Err(format!("task `{name}`: expected a step array or a `{{ steps = ... }}` / `{{ tasks = ... }}` table")),
    }
}

fn parse_string_list(field: &str, val: &ConfigValue) -> Result<Vec<String>, String> {
    match val {
        ConfigValue::Array(items) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item {
                    ConfigValue::String(s) if !s.is_empty() => out.push(s.clone()),
                    _ => return Err(format!("field `{field}` must hold non-empty strings")),
                }
            }
            Ok(out)
        }
        _ => Err(format!("field `{field}` must be an array of strings")),
    }
}

fn parse_bindgen_driver(val: &ConfigValue) -> Result<BindgenDriver, String> {
    let obj = match val {
        ConfigValue::Object(_) => val,
        _ => return Err("field `bindgen` must be an object".to_string()),
    };
    for (key, _) in object_fields(obj) {
        if !["headers", "include", "prefix", "skip", "native_lib", "upstream", "outputs"].contains(&key.as_str()) {
            return Err(format!("field `bindgen.{key}` is not a driver field (expected `headers`, `include`, `prefix`, `skip`, `native_lib`, `upstream`, or `outputs`)"));
        }
    }
    let headers = match obj_get(obj, "headers") {
        Some(v) => parse_string_list("bindgen.headers", v)?,
        None => return Err("field `bindgen` is missing required field `headers`".to_string()),
    };
    if headers.is_empty() {
        return Err("field `bindgen.headers` must hold at least one header".to_string());
    }
    let include = match obj_get(obj, "include") {
        Some(v) => parse_string_list("bindgen.include", v)?,
        None => Vec::new(),
    };
    let prefix = match obj_get(obj, "prefix") {
        Some(v) => parse_string_list("bindgen.prefix", v)?,
        None => Vec::new(),
    };
    let skip = match obj_get(obj, "skip") {
        Some(v) => parse_string_list("bindgen.skip", v)?,
        None => Vec::new(),
    };
    let native_lib = match obj_get(obj, "native_lib") {
        Some(ConfigValue::String(s)) if !s.is_empty() => Some(s.clone()),
        Some(_) => return Err("field `bindgen.native_lib` must be a non-empty string".to_string()),
        None => None,
    };
    let upstream = match obj_get(obj, "upstream") {
        Some(ConfigValue::String(s)) => {
            if !crate::oci::valid_name(s) {
                return Err(format!("field `bindgen.upstream` `{s}` must match `[A-Za-z0-9._-]+`"));
            }
            Some(s.clone())
        }
        Some(_) => return Err("field `bindgen.upstream` must be a string".to_string()),
        None => None,
    };
    let entries = match obj_get(obj, "outputs") {
        Some(ConfigValue::Object(entries)) if !entries.is_empty() => entries,
        Some(ConfigValue::Object(_)) => return Err("field `bindgen.outputs` must hold at least one output".to_string()),
        Some(_) => return Err("field `bindgen.outputs` must be an object".to_string()),
        None => return Err("field `bindgen` is missing required field `outputs`".to_string()),
    };
    let mut outputs = BTreeMap::new();
    for (dir, decl) in entries {
        if dir.is_empty() {
            return Err("field `bindgen.outputs` holds an empty output directory".to_string());
        }
        let table = match decl {
            ConfigValue::Object(_) => decl,
            _ => return Err(format!("field `bindgen.outputs.{dir}` must be an object (`{{ title = ... }}`)")),
        };
        for (key, _) in object_fields(table) {
            if !["title", "target"].contains(&key.as_str()) {
                return Err(format!("field `bindgen.outputs.{dir}.{key}` is not an output field (expected `title` or `target`)"));
            }
        }
        let title = match obj_get(table, "title") {
            Some(ConfigValue::String(s)) if crate::oci::valid_title(s) => s.clone(),
            _ => return Err(format!("field `bindgen.outputs.{dir}.title` must be a plain filename")),
        };
        let target = match obj_get(table, "target") {
            Some(ConfigValue::String(s)) if crate::oci::valid_target(s) => Some(s.clone()),
            Some(_) => return Err(format!("field `bindgen.outputs.{dir}.target` must be an arch-vendor-os-env triple")),
            None => None,
        };
        outputs.insert(dir.clone(), BindgenOutputDecl { title, target });
    }
    Ok(BindgenDriver { headers, include, prefix, skip, native_lib, upstream, outputs })
}

const MAX_KEYWORDS: usize = 12;
const MAX_KEYWORD_LEN: usize = 24;

fn check_permission_perm(s: &str) -> Result<(), String> {
    use std::str::FromStr;
    crate::capabilities::Capability::from_str(s)
        .map(|_| ())
        .map_err(|e| format!("invalid permission `{s}`: {e}"))
}

fn parse_permission_item(item: &ConfigValue) -> Result<PermissionDecl, String> {
    match item {
        ConfigValue::String(s) => {
            check_permission_perm(s)?;
            Ok(PermissionDecl { perm: s.clone(), reason: None })
        }
        ConfigValue::Object(_) => {
            let perm = match obj_get(item, "perm") {
                Some(ConfigValue::String(s)) => s.clone(),
                Some(_) => return Err("field `permissions[].perm` must be a string".to_string()),
                None => return Err("permission entry is missing required field `perm`".to_string()),
            };
            check_permission_perm(&perm)?;
            let reason = match obj_get(item, "reason") {
                Some(ConfigValue::String(s)) => Some(s.clone()),
                Some(_) => return Err("field `permissions[].reason` must be a string".to_string()),
                None => None,
            };
            Ok(PermissionDecl { perm, reason })
        }
        _ => Err("field `permissions` must be strings or `{ perm, reason }` tables".to_string()),
    }
}

fn parse_allowed_item(item: &ConfigValue) -> Result<PermissionDecl, String> {
    match item {
        ConfigValue::String(s) => {
            check_permission_perm(s)?;
            Ok(PermissionDecl { perm: s.clone(), reason: None })
        }
        _ => Err("field `permissions.allowed` must be strings".to_string()),
    }
}

fn check_keyword(word: &str) -> Result<(), String> {
    if word.is_empty() {
        return Err("keyword must not be empty".to_string());
    }
    if word.len() > MAX_KEYWORD_LEN {
        return Err(format!("keyword `{word}` exceeds {MAX_KEYWORD_LEN} characters"));
    }
    if !word
        .bytes()
        .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
    {
        return Err(format!(
            "keyword `{word}` must use lowercase letters, digits, and hyphens only"
        ));
    }
    Ok(())
}

fn parse_members(val: &ConfigValue) -> Result<Vec<String>, String> {
    match val {
        ConfigValue::Array(items) => {
            let mut members = Vec::new();
            for item in items {
                match item {
                    ConfigValue::String(s) => members.push(s.clone()),
                    _ => return Err("workspace `members` must be strings".to_string()),
                }
            }
            Ok(members)
        }
        _ => Err("workspace field `members` must be an array".to_string()),
    }
}

fn parse_registry_table(val: &ConfigValue) -> Result<RegistryConfig, String> {
    let obj = match val {
        ConfigValue::Object(_) => val,
        _ => return Err("registry must be an object".to_string()),
    };
    let url = match obj_get(obj, "url") {
        Some(ConfigValue::String(s)) => s.clone(),
        Some(_) => return Err("registry field `url` must be a string".to_string()),
        None => return Err("registry is missing required field `url`".to_string()),
    };
    let token_env = match obj_get(obj, "token_env") {
        Some(ConfigValue::String(s)) => Some(s.clone()),
        Some(_) => return Err("registry field `token_env` must be a string".to_string()),
        None => None,
    };
    let ca_cert = match obj_get(obj, "ca_cert") {
        Some(ConfigValue::String(s)) => Some(s.clone()),
        Some(_) => return Err("registry field `ca_cert` must be a string".to_string()),
        None => None,
    };
    Ok(RegistryConfig { url, token_env, ca_cert })
}

fn parse_entries(val: &ConfigValue) -> Result<Entries, String> {
    let obj = match val {
        ConfigValue::Object(_) => val,
        _ => return Err("field `entries` must be an object".to_string()),
    };
    let main = match obj_get(obj, "main") {
        Some(ConfigValue::String(s)) => s.clone(),
        Some(_) => return Err("field `entries.main` must be a string".to_string()),
        None => DEFAULT_ENTRY.to_string(),
    };
    let opt_entry = |key: &str| match obj_get(obj, key) {
        Some(ConfigValue::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(format!("field `entries.{key}` must be a string")),
        None => Ok(None),
    };
    let lib = opt_entry("lib")?;
    let docs = opt_entry("docs")?;
    let mut bins = BTreeMap::new();
    match obj_get(obj, "bins") {
        Some(ConfigValue::Object(items)) => {
            for (k, v) in items {
                match v {
                    ConfigValue::String(s) => {
                        bins.insert(k.clone(), s.clone());
                    }
                    _ => return Err(format!("field `entries.bins.{k}` must be a string")),
                }
            }
        }
        Some(_) => return Err("field `entries.bins` must be an object".to_string()),
        None => {}
    }
    Ok(Entries { main, lib, docs, bins })
}

pub fn parse_manifest_for_publish(
    text: &str,
) -> Result<(Option<ProjectConfig>, Option<WorkspaceConfig>), String> {
    parse_manifest(text)
}

pub(crate) fn parse_manifest(text: &str) -> Result<(Option<ProjectConfig>, Option<WorkspaceConfig>), String> {
    parse_manifest_with(text, ConfigTarget::host())
}

pub fn parse_tasks_for_target(
    text: &str,
    target: ConfigTarget,
) -> Result<BTreeMap<String, TaskDef>, String> {
    let (project, _) = parse_manifest_with(text, target)?;
    Ok(project.map(|p| p.tasks).unwrap_or_default())
}

pub(crate) fn parse_manifest_with(
    text: &str,
    target: ConfigTarget,
) -> Result<(Option<ProjectConfig>, Option<WorkspaceConfig>), String> {
    let root = eval_config_text_with(text, target)?;
    let top = match &root {
        ConfigValue::Object(_) => &root,
        _ => return Err("configuration `export default` must be an object".to_string()),
    };
    let project = match obj_get(top, "project") {
        Some(p) => {
            let obj = match p {
                ConfigValue::Object(_) => p,
                _ => return Err("field `project` must be an object".to_string()),
            };
            let name = req_str(obj, "name")?;
            let version = req_str(obj, "version")?;
            if obj_get(obj, "entry").is_some() {
                return Err("`entry` was removed, use `entries.main`".to_string());
            }
            if obj_get(obj, "edition").is_some() {
                return Err("`edition` was removed, engine range only".to_string());
            }
            let engine = opt_str(obj, "engine", "")?;
            let description = opt_str(obj, "description", "")?;
            if obj_get(top, "entry").is_some() {
                return Err("`entry` was removed, use `entries.main`".to_string());
            }
            if obj_get(top, "edition").is_some() {
                return Err("`edition` was removed, engine range only".to_string());
            }
            let entries = match obj_get(top, "entries") {
                None => Entries::default(),
                Some(v) => parse_entries(v)?,
            };
            let mut dependencies = BTreeMap::new();
            if let Some(deps) = obj_get(top, "dependencies") {
                match deps {
                    ConfigValue::Object(entries) => {
                        for (k, v) in entries {
                            dependencies.insert(k.clone(), parse_dependency(k, v)?);
                        }
                    }
                    _ => return Err("field `dependencies` must be an object".to_string()),
                }
            }
            let permissions = match obj_get(top, "permissions") {
                Some(ConfigValue::Array(items)) => {
                    let mut allowed = Vec::new();
                    for item in items {
                        allowed.push(parse_permission_item(item)?);
                    }
                    Some(allowed)
                }
                Some(p) => match obj_get(p, "allowed") {
                    Some(ConfigValue::Array(items)) => {
                        let mut allowed = Vec::new();
                        for item in items {
                            allowed.push(parse_allowed_item(item)?);
                        }
                        Some(allowed)
                    }
                    Some(_) => {
                        return Err("field `permissions.allowed` must be an array".to_string());
                    }
                    None => Some(Vec::new()),
                },
                None => None,
            };
            let keywords = match obj_get(top, "keywords") {
                Some(ConfigValue::Array(items)) => {
                    let mut words = Vec::new();
                    for item in items {
                        match item {
                            ConfigValue::String(s) => {
                                check_keyword(s).map_err(|m| format!("field `keywords`: {m}"))?;
                                words.push(s.clone());
                            }
                            _ => return Err("field `keywords` must be strings".to_string()),
                        }
                    }
                    if words.len() > MAX_KEYWORDS {
                        return Err(format!(
                            "field `keywords` holds {} entries, at most {MAX_KEYWORDS} allowed",
                            words.len()
                        ));
                    }
                    words
                }
                Some(_) => return Err("field `keywords` must be an array".to_string()),
                None => Vec::new(),
            };
            let registry = match obj_get(top, "registry") {
                Some(r) => Some(parse_registry_table(r)?),
                None => None,
            };
            let mut registries: BTreeMap<String, RegistryConfig> = BTreeMap::new();
            if let Some(ConfigValue::Object(entries)) = obj_get(top, "registries") {
                for (scope, table) in entries {
                    if scope.is_empty() {
                        return Err("empty scope in `registries`".to_string());
                    }
                    if registries.contains_key(scope) {
                        return Err(format!("duplicate registry scope `{scope}`"));
                    }
                    registries.insert(scope.clone(), parse_registry_table(table)?);
                }
            } else if obj_get(top, "registries").is_some() {
                return Err("field `registries` must be an object".to_string());
            }
            let artifacts = match obj_get(top, "artifacts") {
                Some(v) => parse_artifact_decl(v)?,
                None => None,
            };
            for (dep_name, spec) in &dependencies {
                if let DependencySpec::Native { from: Some(title), .. } = spec
                    && artifacts.is_none()
                {
                    return Err(format!("dependency `{dep_name}`: `from: \"{title}\"` names a title in the package's own `artifacts` image (declare `artifacts: \"host/org/repo\"` first)"));
                }
            }
            let mut native_pins = BTreeMap::new();
            if let Some(ConfigValue::Object(entries)) = obj_get(top, "native") {
                for (k, v) in entries {
                    native_pins.insert(k.clone(), parse_native_pin(k, v)?);
                }
            } else if obj_get(top, "native").is_some() {
                return Err("field `native` must be an object".to_string());
            }
            let mut tasks = BTreeMap::new();
            if let Some(ConfigValue::Object(entries)) = obj_get(top, "tasks") {
                for (k, v) in entries {
                    if k == "bindgen" {
                        return Err("task `bindgen` is reserved for the bindgen driver (declare `bindgen: { ... }` instead)".to_string());
                    }
                    tasks.insert(k.clone(), parse_task_def(k, v)?);
                }
            } else if obj_get(top, "tasks").is_some() {
                return Err("field `tasks` must be an object".to_string());
            }
            let bindgen = match obj_get(top, "bindgen") {
                Some(v) => Some(parse_bindgen_driver(v)?),
                None => None,
            };
            let license = match obj_get(top, "license") {
                Some(ConfigValue::String(s)) => {
                    if !crate::license::is_known_spdx(s) {
                        return Err(format!(
                            "field `license` `{s}` is not a known SPDX id (expected one of {})",
                            crate::license::KNOWN_SPDX.join(", ")
                        ));
                    }
                    Some(s.clone())
                }
                Some(_) => return Err("field `license` must be a string".to_string()),
                None => None,
            };
            Some(ProjectConfig {
                name,
                version,
                description,
                engine,
                entries,
                registry,
                registries,
                dependencies,
                permissions,
                keywords,
                artifacts,
                license,
                native_pins,
                tasks,
                bindgen,
            })
        }
        None => None,
    };
    let workspace = match obj_get(top, "workspace") {
        Some(w) => {
            let members = match obj_get(w, "members") {
                Some(m) => parse_members(m)?,
                None => Vec::new(),
            };
            Some(WorkspaceConfig { members })
        }
        None => None,
    };
    Ok((project, workspace))
}

fn parse_config(text: &str) -> Result<ProjectConfig, String> {
    match parse_manifest(text)? {
        (Some(config), _) => Ok(config),
        (None, _) => Err("missing `project` object".to_string()),
    }
}
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ManifestIssue {
    pub line: usize,
    pub error: bool,
    pub code: Code,
    pub message: String,
}

const PROJECT_KEYS: &[&str] = &["name", "version", "description", "engine"];
const ENTRIES_KEYS: &[&str] = &["main", "lib", "docs", "bins"];
const REGISTRY_KEYS: &[&str] = &["url", "token_env", "ca_cert"];
const WORKSPACE_KEYS: &[&str] = &["members"];
const DEP_KEYS: &[&str] = &["path", "git", "rev", "tag", "branch", "version", "url", "checksum", "native", "system", "from", "link"];
const TOP_LEVEL_KEYS: &[&str] = &["project", "entries", "dependencies", "permissions", "keywords", "registry", "registries", "workspace", "artifacts", "native", "tasks", "bindgen", "license"];

fn span_line(text: &str, offset: u32) -> usize {
    let off = (offset as usize).min(text.len());
    text[..off].bytes().filter(|&b| b == b'\n').count()
}

fn is_version_shape(version: &str) -> bool {
    let parts: Vec<&str> = version.split('.').collect();
    parts.len() == 3
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.chars().all(|c| c.is_ascii_digit()))
}

pub fn manifest_to_rnx(manifest: &Manifest) -> String {
    manifest_to_rnx_inner(manifest, false)
}

pub fn manifest_to_rnx_for_publish(manifest: &Manifest) -> String {
    manifest_to_rnx_inner(manifest, true)
}

fn manifest_to_rnx_inner(manifest: &Manifest, strip_aux: bool) -> String {
    let mut out = String::from("export default {\n");
    if let Some(p) = &manifest.project {
        let mut fields = vec![
            format!("name: {}", crate::deplock::rnx_string(&p.name)),
            format!("version: {}", crate::deplock::rnx_string(&p.version)),
        ];
        if !p.engine.is_empty() {
            fields.push(format!("engine: {}", crate::deplock::rnx_string(&p.engine)));
        }
        if !p.description.is_empty() {
            fields.push(format!("description: {}", crate::deplock::rnx_string(&p.description)));
        }
        out.push_str("    project: {\n");
        for (i, f) in fields.iter().enumerate() {
            out.push_str("        ");
            out.push_str(f);
            if i + 1 < fields.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("    },\n");
        let e = &p.entries;
        let mut efields: Vec<String> = Vec::new();
        if e.main != DEFAULT_ENTRY {
            efields.push(format!("main: {}", crate::deplock::rnx_string(&e.main)));
        }
        if let Some(l) = &e.lib {
            efields.push(format!("lib: {}", crate::deplock::rnx_string(l)));
        }
        if let Some(d) = &e.docs {
            efields.push(format!("docs: {}", crate::deplock::rnx_string(d)));
        }
        if !e.bins.is_empty() {
            let mut names: Vec<&String> = e.bins.keys().collect();
            names.sort();
            let inner: Vec<String> = names
                .iter()
                .map(|k| {
                    let key = if is_ident_key(k) {
                        (*k).clone()
                    } else {
                        crate::deplock::rnx_string(k)
                    };
                    format!("{key}: {}", crate::deplock::rnx_string(&e.bins[*k]))
                })
                .collect();
            efields.push(format!("bins: {{ {} }}", inner.join(", ")));
        }
        if !efields.is_empty() {
            out.push_str("    entries: {\n");
            for (i, f) in efields.iter().enumerate() {
                out.push_str("        ");
                out.push_str(f);
                if i + 1 < efields.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("    },\n");
        }
    }
    if let Some(p) = &manifest.project {
        if !p.dependencies.is_empty() {
            out.push_str("    dependencies: {\n");
            let mut deps: Vec<(&String, &DependencySpec)> = p.dependencies.iter().collect();
            deps.sort_by(|a, b| a.0.cmp(b.0));
            let quote_all = deps.iter().any(|(k, _)| !is_ident_key(k));
            for (i, (k, v)) in deps.iter().enumerate() {
                let key = if quote_all || !is_ident_key(k) {
                    crate::deplock::rnx_string(k)
                } else {
                    k.to_string()
                };
                out.push_str(&format!("        {key}: {}", dep_to_rnx(v)));
                if i + 1 < deps.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("    },\n");
        }
        if let Some(allowed) = &p.permissions {
            out.push_str("    permissions: [");
            for (i, decl) in allowed.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                match &decl.reason {
                    Some(r) => out.push_str(&format!(
                        "{{ perm: {}, reason: {} }}",
                        crate::deplock::rnx_string(&decl.perm),
                        crate::deplock::rnx_string(r)
                    )),
                    None => out.push_str(&crate::deplock::rnx_string(&decl.perm)),
                }
            }
            out.push_str("],\n");
        }
        if !p.keywords.is_empty() {
            out.push_str("    keywords: [");
            for (i, k) in p.keywords.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&crate::deplock::rnx_string(k));
            }
            out.push_str("],\n");
        }
    }
    if let Some(p) = &manifest.project {
        if let Some(r) = &p.registry {
            out.push_str("    registry: {\n");
            let mut fields =
                vec![format!("url: {}", crate::deplock::rnx_string(&r.url))];
            if let Some(t) = &r.token_env {
                fields.push(format!("token_env: {}", crate::deplock::rnx_string(t)));
            }
            if let Some(c) = &r.ca_cert {
                fields.push(format!("ca_cert: {}", crate::deplock::rnx_string(c)));
            }
            for (i, f) in fields.iter().enumerate() {
                out.push_str("        ");
                out.push_str(f);
                if i + 1 < fields.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("    },\n");
        }
        if !p.registries.is_empty() {
            out.push_str("    registries: {\n");
            let mut scopes: Vec<(&String, &RegistryConfig)> = p.registries.iter().collect();
            scopes.sort_by(|a, b| a.0.cmp(b.0));
            for (i, (scope, r)) in scopes.iter().enumerate() {
                out.push_str(&format!("        {}: {{\n", crate::deplock::rnx_string(scope)));
                let mut fields =
                    vec![format!("url: {}", crate::deplock::rnx_string(&r.url))];
                if let Some(t) = &r.token_env {
                    fields.push(format!("token_env: {}", crate::deplock::rnx_string(t)));
                }
                if let Some(c) = &r.ca_cert {
                    fields.push(format!("ca_cert: {}", crate::deplock::rnx_string(c)));
                }
                for (j, f) in fields.iter().enumerate() {
                    out.push_str("            ");
                    out.push_str(f);
                    if j + 1 < fields.len() {
                        out.push(',');
                    }
                    out.push('\n');
                }
                out.push_str("        }");
                if i + 1 < scopes.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("    },\n");
        }
        if let Some(a) = &p.artifacts {
            out.push_str(&format!("    artifacts: {},\n", crate::deplock::rnx_string(a)));
        }
        if let Some(l) = &p.license {
            out.push_str(&format!("    license: {},\n", crate::deplock::rnx_string(l)));
        }
        if !p.native_pins.is_empty() {
            out.push_str("    native: {\n");
            let mut names: Vec<&String> = p.native_pins.keys().collect();
            names.sort();
            for (i, name) in names.iter().enumerate() {
                let key = if is_ident_key(name) {
                    (*name).clone()
                } else {
                    crate::deplock::rnx_string(name)
                };
                out.push_str(&format!("        {key}: {}", native_pin_to_rnx(&p.native_pins[*name])));
                if i + 1 < names.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("    },\n");
        }
        if !p.tasks.is_empty() && !strip_aux {
            out.push_str("    tasks: {\n");
            let mut names: Vec<&String> = p.tasks.keys().collect();
            names.sort();
            for (i, name) in names.iter().enumerate() {
                let key = if is_ident_key(name) {
                    (*name).clone()
                } else {
                    crate::deplock::rnx_string(name)
                };
                out.push_str(&format!("        {key}: {}", task_def_to_rnx(&p.tasks[*name])));
                if i + 1 < names.len() {
                    out.push(',');
                }
                out.push('\n');
            }
            out.push_str("    },\n");
        }
        if let Some(b) = &p.bindgen
            && !strip_aux
        {
            out.push_str(&format!("    bindgen: {},\n", bindgen_driver_to_rnx(b)));
        }
    }
    if let Some(w) = &manifest.workspace {
        out.push_str("    workspace: {\n        members: [");
        for (i, m) in w.members.iter().enumerate() {
            if i > 0 {
                out.push_str(", ");
            }
            out.push_str(&crate::deplock::rnx_string(m));
        }
        out.push_str("]\n    },\n");
    }
    if out.ends_with(",\n") {
        out.pop();
        out.pop();
        out.push('\n');
    }
    out.push_str("}\n");
    out
}

fn task_step_to_rnx(step: &TaskStep) -> String {
    match step {
        TaskStep::Run(cmd) => crate::deplock::rnx_string(cmd),
        TaskStep::Invoke(name) => format!("{{ task: {} }}", crate::deplock::rnx_string(name)),
    }
}

fn task_def_to_rnx(def: &TaskDef) -> String {
    match def {
        TaskDef::Steps { steps, matrix } => {
            if matrix.is_empty() && steps.iter().all(|s| matches!(s, TaskStep::Run(_))) {
                let inner: Vec<String> = steps.iter().map(task_step_to_rnx).collect();
                return format!("[{}]", inner.join(", "));
            }
            let mut s = String::from("{ ");
            let inner: Vec<String> = steps.iter().map(task_step_to_rnx).collect();
            s.push_str(&format!("steps: [{}]", inner.join(", ")));
            if !matrix.is_empty() {
                let triples: Vec<String> = matrix.iter().map(|t| crate::deplock::rnx_string(t)).collect();
                s.push_str(&format!(", matrix: [{}]", triples.join(", ")));
            }
            s.push_str(" }");
            s
        }
        TaskDef::Group { tasks, strategy, on_failure } => {
            let names: Vec<String> = tasks.iter().map(|t| crate::deplock::rnx_string(t)).collect();
            let mut s = format!("{{ tasks: [{}]", names.join(", "));
            if *strategy != TaskStrategy::Serial {
                s.push_str(", strategy: \"parallel\"");
            }
            if *on_failure != TaskOnFailure::Stop {
                s.push_str(", \"on-failure\": \"continue\"");
            }
            s.push_str(" }");
            s
        }
    }
}

fn native_pin_to_rnx(pin: &NativePin) -> String {
    let mut s = format!("{{ system: {}", pin.system);
    if let Some(v) = &pin.version {
        s.push_str(&format!(", version: {}", crate::deplock::rnx_string(v)));
    }
    if let Some(f) = &pin.from {
        s.push_str(&format!(", from: {}", crate::deplock::rnx_string(f)));
    }
    s.push_str(" }");
    s
}

fn bindgen_driver_to_rnx(driver: &BindgenDriver) -> String {
    let mut parts = Vec::new();
    let list = |items: &[String]| {
        let quoted: Vec<String> = items.iter().map(|s| crate::deplock::rnx_string(s)).collect();
        format!("[{}]", quoted.join(", "))
    };
    parts.push(format!("headers: {}", list(&driver.headers)));
    if !driver.include.is_empty() {
        parts.push(format!("include: {}", list(&driver.include)));
    }
    if !driver.prefix.is_empty() {
        parts.push(format!("prefix: {}", list(&driver.prefix)));
    }
    if !driver.skip.is_empty() {
        parts.push(format!("skip: {}", list(&driver.skip)));
    }
    if let Some(lib) = &driver.native_lib {
        parts.push(format!("native_lib: {}", crate::deplock::rnx_string(lib)));
    }
    if let Some(upstream) = &driver.upstream {
        parts.push(format!("upstream: {}", crate::deplock::rnx_string(upstream)));
    }
    let mut outs: Vec<String> = Vec::new();
    for (dir, decl) in &driver.outputs {
        let mut o = format!("{{ title: {}", crate::deplock::rnx_string(&decl.title));
        if let Some(t) = &decl.target {
            o.push_str(&format!(", target: {}", crate::deplock::rnx_string(t)));
        }
        o.push_str(" }");
        outs.push(format!("{}: {o}", crate::deplock::rnx_string(dir)));
    }
    parts.push(format!("outputs: {{ {} }}", outs.join(", ")));
    format!("{{ {} }}", parts.join(", "))
}

fn is_ident_key(k: &str) -> bool {
    let mut chars = k.chars();
    match chars.next() {
        Some(c) if c.is_ascii_alphabetic() || c == '_' => {}
        _ => return false,
    }
    chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
}

fn is_scope_segment(s: &str, max: usize) -> bool {
    let mut chars = s.chars();
    match chars.next() {
        Some(c) if c.is_ascii_lowercase() || c.is_ascii_digit() => {}
        _ => return false,
    }
    let rest: String = chars.collect();
    rest.len() <= max && rest.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '-')
}

pub fn is_package_name(full: &str) -> bool {
    if let Some(scoped) = full.strip_prefix('@') {
        let Some(slash) = scoped.find('/') else {
            return false;
        };
        let (scope, name) = scoped.split_at(slash);
        let name = &name[1..];
        return !scope.is_empty()
            && !name.is_empty()
            && is_scope_segment(scope, 31)
            && is_scope_segment(name, 63);
    }
    !full.is_empty() && is_scope_segment(full, 63)
}

fn dep_to_rnx(spec: &DependencySpec) -> String {
    match spec {
        DependencySpec::Semver { version, link } => {
            if *link == crate::license::LinkPreference::Auto {
                crate::deplock::rnx_string(version)
            } else {
                format!(
                    "{{ version: {}, link: {} }}",
                    crate::deplock::rnx_string(version),
                    crate::deplock::rnx_string(link.as_str())
                )
            }
        }
        DependencySpec::Path { path, link } => {
            if *link == crate::license::LinkPreference::Auto {
                format!("{{ path: {} }}", crate::deplock::rnx_string(&path.to_string_lossy()))
            } else {
                format!(
                    "{{ path: {}, link: {} }}",
                    crate::deplock::rnx_string(&path.to_string_lossy()),
                    crate::deplock::rnx_string(link.as_str())
                )
            }
        }
        DependencySpec::Git { git, rev, link } => {
            let mut s = format!(
                "{{ git: {}, rev: {}",
                crate::deplock::rnx_string(git),
                crate::deplock::rnx_string(rev)
            );
            if *link != crate::license::LinkPreference::Auto {
                s.push_str(&format!(", link: {}", crate::deplock::rnx_string(link.as_str())));
            }
            s.push_str(" }");
            s
        }
        DependencySpec::Url { version, url, checksum, link } => {
            let mut s = String::from("{ ");
            if !version.is_empty() {
                s.push_str(&format!("version: {}, ", crate::deplock::rnx_string(version)));
            }
            s.push_str(&format!("url: {}", crate::deplock::rnx_string(url)));
            if let Some(c) = checksum {
                s.push_str(&format!(", checksum: {}", crate::deplock::rnx_string(c)));
            }
            if *link != crate::license::LinkPreference::Auto {
                s.push_str(&format!(", link: {}", crate::deplock::rnx_string(link.as_str())));
            }
            s.push_str(" }");
            s
        }
        DependencySpec::Native { lib, system, path, version, from, link } => {
            let mut s = format!("{{ native: {}", crate::deplock::rnx_string(lib));
            if *system {
                s.push_str(", system: true");
            }
            if let Some(p) = path {
                s.push_str(&format!(", path: {}", crate::deplock::rnx_string(&p.to_string_lossy())));
            }
            if let Some(v) = version {
                s.push_str(&format!(", version: {}", crate::deplock::rnx_string(v)));
            }
            if let Some(f) = from {
                s.push_str(&format!(", from: {}", crate::deplock::rnx_string(f)));
            }
            if *link != crate::license::LinkPreference::Auto {
                s.push_str(&format!(", link: {}", crate::deplock::rnx_string(link.as_str())));
            }
            s.push_str(" }");
            s
        }
    }
}

pub fn manifest_sections() -> &'static [&'static str] {
    &["project", "entries", "registry", "dependencies", "workspace", "permissions", "keywords", "registries", "artifacts", "license", "native", "tasks", "bindgen"]
}

pub fn manifest_keys(section: &str) -> &'static [&'static str] {
    match section {
        "project" => PROJECT_KEYS,
        "entries" => ENTRIES_KEYS,
        "workspace" => WORKSPACE_KEYS,
        _ => &[],
    }
}

pub fn manifest_section_doc(section: &str) -> Option<&'static str> {
    match section {
        "project" => Some("Package identity: `name` and `version` are required, `engine` states the minimum toolchain requirement."),
        "entries" => Some("Entry points: `main` defaults to `src/main.rnx`, `lib` names the library file, `docs` the guides folder, `bins` extra tool shims."),
        "registry" => Some("Default registry for unscoped packages: `url` is required, `token_env` names the auth token env var."),
        "dependencies" => Some("Semver (`\"^1.2.0\"`), path (`\"libs/x\"` or `{ path = \"...\" }`), git (`{ git = \"<url>\", rev/tag/branch = \"...\" }`), tarball (`{ version = \"...\", url = \"...\", checksum = \"...\" }`), or native (`{ native = \"z\", system = true }`, plus `version` constraints and `from` image titles)."),
        "workspace" => Some("Monorepo members: `members = [\"alpha\", \"beta\"]` or globs like `[\"crates/*\"]`."),
        "permissions" => Some("Security ceiling: an array of capability strings the package and its dependencies may use."),
        "keywords" => Some("Search keywords: an array of up to 12 lowercase/digit/hyphen tags, 24 characters each, mirroring npm keywords."),
        "registries" => Some("Scoped registry overrides keyed by scope, e.g. `registries: { \"@acme\": { url = \"...\" } }`."),
        "artifacts" => Some("One bare OCI repo ref for the version's artifact image, e.g. `artifacts: \"ghcr.io/org/sqlite3\"`. No tags, no digests; the digest is asserted out-of-band at publish time."),
        "license" => Some("SPDX license id for the package, e.g. `license: \"MIT\"`. Must match the LICENSE file at the package root."),
        "native" => Some("Final-consumer re-pin of transitive native sources: `native: { zlib: { system: true } }` overrides the author's declaration. Root only; member and dependency tables are ignored."),
        "tasks" => Some("Author-local task runner: `tasks: { check: [\"rnx check\", \"rnx lint\"] }`. Metatasks run other tasks with `strategy: serial | parallel` and `on-failure: stop | continue`. Never ships, never runs on consumers."),
        "bindgen" => Some("Bindgen driver: shared `headers`, `prefix`/`skip` sets, and per-output titles consumed by `rnx task bindgen`. Author-local; only the generated files ship."),
        _ if section.starts_with("registries.") => Some("Scoped registry override for `@scope/*`: same fields as `[registry]` plus optional `ca_cert`."),
        _ => None,
    }
}

pub fn manifest_field_doc(section: &str, key: &str) -> Option<&'static str> {
    match (section, key) {
        ("project", "name") => Some("Package name, used for imports and packaging."),
        ("project", "version") => Some("Package version as `major.minor.patch`."),
        ("project", "description") => Some("One-line package summary shown by `rnx doc`."),
        ("project", "engine") => Some("Minimum toolchain requirement, e.g. `>=0.4.0`."),
        ("entries", "main") => Some("Main entry file, defaults to `src/main.rnx`."),
        ("entries", "lib") => Some("Library entry file for `import \"pkg\"` consumers."),
        ("entries", "docs") => Some("Guides folder collected at pack time, at most one nesting level."),
        ("entries", "bins") => Some("Named tool entry files, one shim per entry."),
        ("registry", "url") => Some("Default registry URL for unscoped packages."),
        ("registry", "token_env") => Some("Env var holding the default registry auth token."),
        ("registry", "ca_cert") => Some("Custom root CA for the registry (enterprise proxy/VPN)."),
        ("workspace", "members") => Some("Member package paths, strings or `\"dir/*\"` globs."),
        (_, "path") => Some("Local path dependency; must contain its own `Project.config`."),
        (_, "git") => Some("Git URL dependency; requires one of `rev`, `tag`, or `branch`."),
        (_, "rev") => Some("Pinned tag or commit for a git dependency."),
        (_, "tag") => Some("Git tag for a git dependency (stored as the pinned ref)."),
        (_, "branch") => Some("Git branch for a git dependency (moving target)."),
        ("native", "system") => Some("Consumer re-pin: `true` takes the host library, `false` takes the bundled OCI blob."),
        ("native", "version") => Some("Consumer re-pin of the version constraint: host library version for `system: true`, declaring-package version for bundled sources."),
        ("native", "from") => Some("Consumer re-pin of the OCI image title for a bundled native dependency."),
        (_, "version") => Some("Semver requirement, e.g. `\"^3.45.0\"` or `{ version = \"1.5.5\", url = \"...\" }`."),
        (_, "url") => Some("Direct tarball URL, paired with `version` and optional `checksum`."),
        (_, "checksum") => Some("Expected `sha256:...` digest of a tarball URL dependency."),
        (_, "native") => Some("System native library dependency; links `-l<lib>` and records `native:<lib>` in the lockfile."),
        (_, "system") => Some("Whether a native dependency comes from the host system (`true`) or a vendored path."),
        (_, "from") => Some("OCI image title for a bundled native dependency; selects the blob by title from the package's own `artifacts` image and implies `system: false`."),
        (_, "link") => Some("Linkage preference for the dependency: `static`, `dynamic`, or `auto` (default `auto`; `dynamic` needs a native dependency)."),
        (_, "license") => Some("SPDX license id for the package, e.g. `MIT` or `Apache-2.0`. Must match the LICENSE file."),
        ("tasks", _) => Some("Author-local task: a step array, `{ steps, matrix }`, or a `{ tasks, strategy, on-failure }` metatask."),
        ("bindgen", "headers") => Some("C headers the driver feeds the bindgen generator."),
        ("bindgen", "outputs") => Some("Output directories mapping to `{ title, target }`; titles become OCI titles."),
        ("dependencies", _) => Some("Semver, path, git, tarball, or native dependency (see `rnx doc` manifest reference)."),
        _ => None,
    }
}

pub fn validate_manifest_text(text: &str, root: Option<&Path>) -> Vec<ManifestIssue> {
    let mut out = Vec::new();
    let module = match Parser::parse_module(text) {
        Ok(m) => m,
        Err(d) => {
            out.push(ManifestIssue {
                line: d.span.map(|s| span_line(text, s.start)).unwrap_or(0),
                error: true,
                code: d.code,
                message: d.message,
            });
            return out;
        }
    };
    let target = ConfigTarget::host();
    let value = match eval_module(&module, target) {
        Ok(v) => v,
        Err(d) => {
            out.push(ManifestIssue {
                line: d.span.map(|s| span_line(text, s.start)).unwrap_or(0),
                error: true,
                code: d.code,
                message: d.message,
            });
            return out;
        }
    };
    let top = match &value {
        ConfigValue::Object(_) => &value,
        _ => {
            out.push(ManifestIssue {
                line: 0,
                error: true,
                code: Code::E108,
                message: "configuration `export default` must be an object".to_string(),
            });
            return out;
        }
    };
    for (key, _) in object_fields(top) {
        if !TOP_LEVEL_KEYS.contains(&key.as_str()) {
            if key == "entry" {
                out.push(ManifestIssue {
                    line: field_line(text, &module, &[], &key),
                    error: true,
                    code: Code::E108,
                    message: "`entry` was removed, use `entries.main`".to_string(),
                });
            } else if key == "edition" {
                out.push(ManifestIssue {
                    line: field_line(text, &module, &[], &key),
                    error: true,
                    code: Code::E108,
                    message: "`edition` was removed, engine range only".to_string(),
                });
            } else {
                out.push(ManifestIssue {
                    line: field_line(text, &module, &[], &key),
                    error: false,
                    code: Code::W201,
                    message: format!("unrecognized field `{key}`, it is ignored"),
                });
            }
        }
    }
    match obj_get(top, "project") {
        None => out.push(ManifestIssue {
            line: 0,
            error: true,
            code: Code::E108,
            message: "missing `project` object".to_string(),
        }),
        Some(p) => {
            let obj = match p {
                ConfigValue::Object(_) => p,
                _ => {
                    out.push(ManifestIssue {
                        line: 0,
                        error: true,
                        code: Code::E108,
                        message: "field `project` must be an object".to_string(),
                    });
                    return out;
                }
            };
            for (key, _) in object_fields(obj) {
                if !PROJECT_KEYS.contains(&key.as_str()) {
                    if key == "entry" {
                        out.push(ManifestIssue {
                            line: field_line(text, &module, &["project"], &key),
                            error: true,
                            code: Code::E108,
                            message: "`entry` was removed, use `entries.main`".to_string(),
                        });
                    } else if key == "edition" {
                        out.push(ManifestIssue {
                            line: field_line(text, &module, &["project"], &key),
                            error: true,
                            code: Code::E108,
                            message: "`edition` was removed, engine range only".to_string(),
                        });
                    } else {
                        out.push(ManifestIssue {
                            line: field_line(text, &module, &["project"], &key),
                            error: false,
                            code: Code::W201,
                            message: format!("unrecognized field `project.{key}`, it is ignored"),
                        });
                    }
                }
            }
            match obj_get(obj, "name") {
                Some(ConfigValue::String(s)) if is_package_name(s) => {}
                Some(ConfigValue::String(s)) => out.push(ManifestIssue {
                    line: 0,
                    error: true,
                    code: Code::E108,
                    message: format!(
                        "field `project.name` must be a package name (`name` or `@scope/name`, lowercase letters, digits, hyphens), got `{s}`"
                    ),
                }),
                _ => out.push(ManifestIssue {
                    line: 0,
                    error: true,
                    code: Code::E108,
                    message: "field `project.name` must be a non-empty string".to_string(),
                }),
            }
            match obj_get(obj, "version") {
                Some(ConfigValue::String(s)) if is_version_shape(s) => {}
                Some(ConfigValue::String(s)) => out.push(ManifestIssue {
                    line: 0,
                    error: false,
                    code: Code::W201,
                    message: format!("version `{s}` should be `major.minor.patch`"),
                }),
                _ => out.push(ManifestIssue {
                    line: 0,
                    error: true,
                    code: Code::E108,
                    message: "field `project.version` must be a string".to_string(),
                }),
            }
            if let Some(root) = root {
                let entries_val = obj_get(top, "entries");
                let main = match entries_val.and_then(|e| obj_get(e, "main")) {
                    Some(ConfigValue::String(s)) => s.clone(),
                    _ => DEFAULT_ENTRY.to_string(),
                };
                if !root.join(&main).is_file() {
                    let lib_ok = matches!(
                        entries_val.and_then(|e| obj_get(e, "lib")),
                        Some(ConfigValue::String(l)) if root.join(l).is_file()
                    );
                    if lib_ok {
                        out.push(ManifestIssue {
                            line: 0,
                            error: false,
                            code: Code::W201,
                            message: format!("entries.main `{main}` does not exist"),
                        });
                    } else {
                        out.push(ManifestIssue {
                            line: 0,
                            error: true,
                            code: Code::E108,
                            message: format!("entries.main `{main}` does not exist"),
                        });
                    }
                }
                if let Some(ConfigValue::Object(items)) =
                    entries_val.and_then(|e| obj_get(e, "bins"))
                {
                    let mut names: Vec<&str> =
                        items.iter().map(|(k, _)| k.as_str()).collect();
                    names.sort();
                    for name in names {
                        let target = items
                            .iter()
                            .find(|(k, _)| k == name)
                            .map(|(_, v)| v);
                        if let Some(ConfigValue::String(p)) = target
                            && !root.join(p).is_file()
                        {
                            out.push(ManifestIssue {
                                line: 0,
                                error: true,
                                code: Code::E108,
                                message: format!(
                                    "entries.bins.{name} target `{p}` does not exist"
                                ),
                            });
                        }
                    }
                }
                if let Some(ConfigValue::String(d)) =
                    entries_val.and_then(|e| obj_get(e, "docs"))
                {
                    let dir = root.join(d);
                    if !dir.is_dir() {
                        out.push(ManifestIssue {
                            line: 0,
                            error: true,
                            code: Code::E108,
                            message: format!("entries.docs folder `{d}` does not exist"),
                        });
                    } else if let Some(deep) = doc_too_deep(&dir, &dir) {
                        out.push(ManifestIssue {
                            line: 0,
                            error: true,
                            code: Code::E108,
                            message: format!(
                                "entries.docs allows at most one nesting level (`{deep}` is too deep)"
                            ),
                        });
                    }
                }
            }
        }
    }
    if let Some(e) = obj_get(top, "entries") {
        match e {
            ConfigValue::Object(_) => {
                for (key, _) in object_fields(e) {
                    if !ENTRIES_KEYS.contains(&key.as_str()) {
                        out.push(ManifestIssue {
                            line: 0,
                            error: false,
                            code: Code::W201,
                            message: format!("unrecognized field `entries.{key}`, it is ignored"),
                        });
                    }
                }
                for key in ["main", "lib", "docs"] {
                    if let Some(v) = obj_get(e, key)
                        && !matches!(v, ConfigValue::String(_))
                    {
                        out.push(ManifestIssue {
                            line: 0,
                            error: true,
                            code: Code::E108,
                            message: format!("field `entries.{key}` must be a string"),
                        });
                    }
                }
                match obj_get(e, "bins") {
                    Some(ConfigValue::Object(items)) => {
                        for (k, v) in items {
                            if !matches!(v, ConfigValue::String(_)) {
                                out.push(ManifestIssue {
                                    line: 0,
                                    error: true,
                                    code: Code::E108,
                                    message: format!("field `entries.bins.{k}` must be a string"),
                                });
                            }
                        }
                    }
                    Some(_) => out.push(ManifestIssue {
                        line: 0,
                        error: true,
                        code: Code::E108,
                        message: "field `entries.bins` must be an object".to_string(),
                    }),
                    None => {}
                }
            }
            _ => out.push(ManifestIssue {
                line: 0,
                error: true,
                code: Code::E108,
                message: "field `entries` must be an object".to_string(),
            }),
        }
    }
    if let Some(ConfigValue::Object(deps)) = obj_get(top, "dependencies") {
        for (key, val) in deps {
            match parse_dependency(key, val) {
                Ok(DependencySpec::Path { path, .. }) => {
                    if let Some(root) = root {
                        let dir = root.join(&path);
                        if !dir.join(MANIFEST_FILE).is_file() {
                            out.push(ManifestIssue {
                                line: 0,
                                error: false,
                                code: Code::W201,
                                message: format!(
                                    "dependency `{key}`: `{}` has no Project.config",
                                    dir.display()
                                ),
                            });
                        }
                    }
                }
                Ok(DependencySpec::Git { .. }) => {}
                Ok(DependencySpec::Semver { .. }) => {}
                Ok(DependencySpec::Url { .. }) => {}
                Ok(DependencySpec::Native { .. }) => {}
                Err(msg) => out.push(ManifestIssue {
                    line: 0,
                    error: true,
                    code: Code::E108,
                    message: msg,
                }),
            }
            if let ConfigValue::Object(fields) = val {
                for (sub, _) in fields {
                    if !DEP_KEYS.contains(&sub.as_str()) {
                        out.push(ManifestIssue {
                            line: 0,
                            error: false,
                            code: Code::W201,
                            message: format!("unrecognized field `{key}.{sub}`, it is ignored"),
                        });
                    }
                }
            }
        }
    } else if obj_get(top, "dependencies").is_some() {
        out.push(ManifestIssue {
            line: 0,
            error: true,
            code: Code::E108,
            message: "field `dependencies` must be an object".to_string(),
        });
    }
    if let Some(kw) = obj_get(top, "keywords") {
        match kw {
            ConfigValue::Array(items) => {
                for item in items {
                    match item {
                        ConfigValue::String(s) => {
                            if let Err(msg) = check_keyword(s) {
                                out.push(ManifestIssue {
                                    line: 0,
                                    error: true,
                                    code: Code::E108,
                                    message: format!("field `keywords`: {msg}"),
                                });
                            }
                        }
                        _ => out.push(ManifestIssue {
                            line: 0,
                            error: true,
                            code: Code::E108,
                            message: "field `keywords` must be strings".to_string(),
                        }),
                    }
                }
                if items.len() > MAX_KEYWORDS {
                    out.push(ManifestIssue {
                        line: 0,
                        error: true,
                        code: Code::E108,
                        message: format!(
                            "field `keywords` holds {} entries, at most {MAX_KEYWORDS} allowed",
                            items.len()
                        ),
                    });
                }
            }
            _ => out.push(ManifestIssue {
                line: 0,
                error: true,
                code: Code::E108,
                message: "field `keywords` must be an array".to_string(),
            }),
        }
    }
    if let Some(w) = obj_get(top, "workspace") {
        let obj = match w {
            ConfigValue::Object(_) => w,
            _ => {
                out.push(ManifestIssue {
                    line: 0,
                    error: true,
                    code: Code::E108,
                    message: "field `workspace` must be an object".to_string(),
                });
                return out;
            }
        };
        for (key, _) in object_fields(obj) {
            if !WORKSPACE_KEYS.contains(&key.as_str()) {
                out.push(ManifestIssue {
                    line: 0,
                    error: false,
                    code: Code::W201,
                    message: format!("unrecognized field `workspace.{key}`, it is ignored"),
                });
            }
        }
        if let (Some(root), Some(ConfigValue::Array(members))) = (root, obj_get(obj, "members")) {
            for member in members {
                if let ConfigValue::String(pattern) = member {
                    let base = pattern.strip_suffix("/*").unwrap_or(pattern);
                    if !root.join(base).is_dir() {
                        out.push(ManifestIssue {
                            line: 0,
                            error: false,
                            code: Code::W201,
                            message: format!("workspace member `{pattern}` not found"),
                        });
                    }
                }
            }
        }
    }
    if let Some(reg) = obj_get(top, "registry") {
        if let Err(msg) = parse_registry_table(reg) {
            out.push(ManifestIssue { line: 0, error: true, code: Code::E108, message: format!("registry: {msg}") });
        }
        if let ConfigValue::Object(fields) = reg {
            for (key, _) in fields {
                if !REGISTRY_KEYS.contains(&key.as_str()) {
                    out.push(ManifestIssue {
                        line: 0,
                        error: false,
                        code: Code::W201,
                        message: format!("unrecognized field `registry.{key}`, it is ignored"),
                    });
                }
            }
        }
    }
    if let Some(ConfigValue::Object(entries)) = obj_get(top, "registries") {
        for (scope, table) in entries {
            if let Err(msg) = parse_registry_table(table) {
                out.push(ManifestIssue { line: 0, error: true, code: Code::E108, message: format!("registries.{scope}: {msg}") });
            }
            if let ConfigValue::Object(fields) = table {
                for (key, _) in fields {
                    if !REGISTRY_KEYS.contains(&key.as_str()) {
                        out.push(ManifestIssue {
                            line: 0,
                            error: false,
                            code: Code::W201,
                            message: format!("unrecognized field `registries.{scope}.{key}`, it is ignored"),
                        });
                    }
                }
            }
        }
    } else if obj_get(top, "registries").is_some() {
        out.push(ManifestIssue {
            line: 0,
            error: true,
            code: Code::E108,
            message: "field `registries` must be an object".to_string(),
        });
    }
    if let Some(a) = obj_get(top, "artifacts")
        && let Err(msg) = parse_artifact_decl(a)
    {
        out.push(ManifestIssue { line: 0, error: true, code: Code::E108, message: msg });
    }
    if let Some(l) = obj_get(top, "license") {
        match l {
            ConfigValue::String(s) if crate::license::is_known_spdx(s) => {}
            ConfigValue::String(s) => out.push(ManifestIssue {
                line: 0,
                error: true,
                code: Code::E108,
                message: format!(
                    "field `license` `{s}` is not a known SPDX id (expected one of {})",
                    crate::license::KNOWN_SPDX.join(", ")
                ),
            }),
            _ => out.push(ManifestIssue {
                line: 0,
                error: true,
                code: Code::E108,
                message: "field `license` must be a string".to_string(),
            }),
        }
    }
    if let Some(n) = obj_get(top, "native") {
        match n {
            ConfigValue::Object(entries) => {
                for (k, v) in entries {
                    if let Err(msg) = parse_native_pin(k, v) {
                        out.push(ManifestIssue { line: 0, error: true, code: Code::E108, message: msg });
                    }
                }
            }
            _ => out.push(ManifestIssue {
                line: 0,
                error: true,
                code: Code::E108,
                message: "field `native` must be an object".to_string(),
            }),
        }
    }
    if let Some(t) = obj_get(top, "tasks") {
        match t {
            ConfigValue::Object(entries) => {
                for (k, v) in entries {
                    if k == "bindgen" {
                        out.push(ManifestIssue {
                            line: 0,
                            error: true,
                            code: Code::E108,
                            message: "task `bindgen` is reserved for the bindgen driver (declare `bindgen: { ... }` instead)".to_string(),
                        });
                    } else if let Err(msg) = parse_task_def(k, v) {
                        out.push(ManifestIssue { line: 0, error: true, code: Code::E108, message: msg });
                    }
                }
            }
            _ => out.push(ManifestIssue {
                line: 0,
                error: true,
                code: Code::E108,
                message: "field `tasks` must be an object".to_string(),
            }),
        }
    }
    if let Some(b) = obj_get(top, "bindgen")
        && let Err(msg) = parse_bindgen_driver(b)
    {
        out.push(ManifestIssue { line: 0, error: true, code: Code::E108, message: msg });
    }
    if obj_get(top, "artifacts").is_none()
        && let Some(ConfigValue::Object(deps)) = obj_get(top, "dependencies")
    {
        for (key, val) in deps {
            if let Ok(DependencySpec::Native { from: Some(title), .. }) = parse_dependency(key, val) {
                out.push(ManifestIssue {
                    line: 0,
                    error: true,
                    code: Code::E108,
                    message: format!("dependency `{key}`: `from: \"{title}\"` names a title in the package's own `artifacts` image (declare `artifacts: \"host/org/repo\"` first)"),
                });
            }
        }
    }
    out
}

fn doc_too_deep(dir: &Path, base: &Path) -> Option<String> {
    let mut paths: Vec<PathBuf> = std::fs::read_dir(dir)
        .ok()?
        .flatten()
        .map(|e| e.path())
        .collect();
    paths.sort();
    for path in paths {
        if path.is_dir() {
            if let Some(hit) = doc_too_deep(&path, base) {
                return Some(hit);
            }
        } else if path.is_file() {
            if let Ok(rel) = path.strip_prefix(base)
                && rel.components().count() > 2
            {
                return Some(rel.to_string_lossy().replace('\\', "/"));
            }
        }
    }
    None
}

fn object_fields(v: &ConfigValue) -> &[(String, ConfigValue)] {
    match v {
        ConfigValue::Object(fields) => fields,
        _ => &[],
    }
}

fn field_line(
    text: &str,
    module: &crate::ast::Module,
    path: &[&str],
    key: &str,
) -> usize {
    for decl in &module.decls {
        if let crate::ast::Decl::ExportDefault(d) = &decl.node {
            if let Some(line) = find_key_line(&d.expr, path, key) {
                return span_line(text, line);
            }
        }
    }
    0
}

fn find_key_line(e: &crate::ast::Spanned<crate::ast::Expr>, path: &[&str], key: &str) -> Option<u32> {
    use crate::ast::{Expr, MapEntry, RecordEntry};
    let entries: Vec<(&str, &crate::ast::Spanned<crate::ast::Expr>)> = match &e.node {
        Expr::Record(fields) => fields
            .iter()
            .filter_map(|f| match f {
                RecordEntry::Field(k, v) => Some((k.as_str(), v)),
                RecordEntry::Spread(_) => None,
            })
            .collect(),
        Expr::MapLiteral(entries) => entries
            .iter()
            .filter_map(|f| match f {
                MapEntry::Field(k, v) => Some((k.as_str(), v)),
                MapEntry::Spread(_) => None,
            })
            .collect(),
        _ => return None,
    };
    for (k, v) in entries {
        if path.is_empty() {
            if k == key {
                return Some(v.span.start);
            }
        } else if k == path[0] {
            if path.len() == 1 {
                return find_key_line(v, &[], key);
            }
            return find_key_line(v, &path[1..], key);
        }
    }
    None
}
pub fn format_manifest_text(text: &str) -> Result<String, String> {
    crate::fmt::format_source(text).map_err(|e| e.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::project::eval::{ConfigTarget, eval_module};

    fn target() -> ConfigTarget {
        ConfigTarget {
            os: "linux".to_string(),
            arch: "x86_64".to_string(),
            env: "gnu".to_string(),
        }
    }

    fn parse(text: &str) -> ProjectConfig {
        parse_config(text).unwrap_or_else(|e| panic!("parse failed: {e}\n---\n{text}"))
    }

    #[test]
    fn package_names_accept_optional_scope() {
        for ok in ["colony", "http-lite", "@std/fs", "@rovelstars/argparse"] {
            assert!(is_package_name(ok), "{ok}");
        }
        for bad in ["", "@std/", "@/fs", "@Std/fs", "@std/Fs", "has space", "@a/b/c"] {
            assert!(!is_package_name(bad), "{bad}");
        }
    }

    #[test]
    fn parses_project_with_comments_and_inline_objects() {
        let c = parse(
            "// sample manifest\n\
             export default {\n\
                 project: {\n\
                     name: \"demo\", // core fields\n\
                     version: \"0.1.0\"\n\
                 },\n\
                 entries: {\n\
                     main: \"src/app.rnx\" // trailing comment\n\
                 },\n\
                 dependencies: {\n\
                     physics_2d: { path: \"../physics_2d\" },\n\
                     helper: \"libs/helper\"\n\
                 }\n\
             }\n",
        );
        assert_eq!(c.name, "demo");
        assert_eq!(c.version, "0.1.0");
        assert_eq!(c.entries.main, "src/app.rnx");
        assert_eq!(
            c.main_path(Path::new("/root")),
            Path::new("/root/src/app.rnx")
        );
        assert_eq!(
            c.dependencies["physics_2d"],
            DependencySpec::Path {
                path: PathBuf::from("../physics_2d"),
                link: crate::license::LinkPreference::Auto,
            }
        );
        assert_eq!(
            c.dependencies["helper"],
            DependencySpec::Path {
                path: PathBuf::from("libs/helper"),
                link: crate::license::LinkPreference::Auto,
            }
        );
    }

    #[test]
    fn entry_defaults_and_unknown_keys_are_ignored() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 extra_thing: 1\n\
             }\n",
        );
        assert_eq!(c.entries.main, DEFAULT_ENTRY);
        assert_eq!(c.entries.lib, None);
        assert_eq!(c.entries.docs, None);
        assert!(c.entries.bins.is_empty());
        assert!(c.dependencies.is_empty());
        assert_eq!(c.permissions, None);
        assert!(c.keywords.is_empty());
    }

    #[test]
    fn parses_entries_lib_docs_and_bins() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 entries: {\n\
                     main: \"src/main.rnx\",\n\
                     lib: \"src/lib.rnx\",\n\
                     docs: \"docs/\",\n\
                     bins: { tool: \"src/bin/tool.rnx\" }\n\
                 }\n\
             }\n",
        );
        assert_eq!(c.entries.main, "src/main.rnx");
        assert_eq!(c.entries.lib.as_deref(), Some("src/lib.rnx"));
        assert_eq!(c.entries.docs.as_deref(), Some("docs/"));
        assert_eq!(
            c.entries.bins.get("tool").map(String::as_str),
            Some("src/bin/tool.rnx")
        );
        let root = Path::new("/root");
        assert_eq!(c.main_path(root), Path::new("/root/src/main.rnx"));
        assert_eq!(c.lib_path(root), Some(Path::new("/root/src/lib.rnx").to_path_buf()));
        assert_eq!(
            c.bin_path(root, "tool"),
            Some(Path::new("/root/src/bin/tool.rnx").to_path_buf())
        );
        assert_eq!(c.bin_path(root, "missing"), None);
    }

    #[test]
    fn entry_and_edition_are_hard_errors() {
        let e = parse_config(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\", entry: \"src/main.rnx\" }\n\
             }\n",
        )
        .unwrap_err();
        assert!(e.contains("`entry` was removed"), "{e}");
        let e = parse_config(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\", edition: \"2026\" }\n\
             }\n",
        )
        .unwrap_err();
        assert!(e.contains("`edition` was removed"), "{e}");
        for text in [
            "export default {\n    project: { name: \"a\", version: \"0.1.0\", entry: \"src/main.rnx\" }\n}\n",
            "export default {\n    project: { name: \"a\", version: \"0.1.0\", edition: \"2026\" }\n}\n",
        ] {
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108 && i.message.contains("was removed")),
                "{issues:?}"
            );
        }
    }

    #[test]
    fn sandbox_key_falls_back_to_ignored_unknown_key() {
        let text = "export default {\n    project: { name: \"a\", version: \"0.1.0\" },\n    sandbox: \"strict\"\n}\n";
        let issues = validate_manifest_text(text, None);
        assert!(
            issues.iter().any(|i| !i.error
                && i.code == Code::W201
                && i.message.contains("`sandbox`")),
            "{issues:?}"
        );
    }

    #[test]
    fn parses_git_semver_and_native_deps() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 dependencies: {\n\
                     algo: { git: \"https://github.com/example/algo\", branch: \"main\" },\n\
                     json: \"^1.2.0\",\n\
                     zlib: { native: \"z\", system: true },\n\
                     vendored: { native: \"gui\", path: \"./native\" }\n\
                 }\n\
             }\n",
        );
        assert_eq!(
            c.dependencies["algo"],
            DependencySpec::Git {
                git: "https://github.com/example/algo".to_string(),
                rev: "main".to_string(),
                link: crate::license::LinkPreference::Auto,
            }
        );
        assert_eq!(
            c.dependencies["json"],
            DependencySpec::Semver { version: "^1.2.0".to_string(), link: crate::license::LinkPreference::Auto }
        );
        assert_eq!(
            c.dependencies["zlib"],
            DependencySpec::Native { lib: "z".to_string(), system: true, path: None, version: None, from: None, link: crate::license::LinkPreference::Auto }
        );
        assert_eq!(
            c.dependencies["vendored"],
            DependencySpec::Native {
                lib: "gui".to_string(),
                system: false,
                path: Some(PathBuf::from("./native")),
                version: None,
                from: None,
                link: crate::license::LinkPreference::Auto,
            }
        );
    }

    #[test]
    fn rejects_native_mixed_with_other_sources() {
        let e = parse_config(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 dependencies: { zlib: { native: \"z\", git: \"https://example.com/z\" } }\n\
             }\n",
        )
        .unwrap_err();
        assert!(e.contains("must not mix"), "{e}");
    }

    #[test]
    fn native_version_and_from_parse_and_round_trip() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 artifacts: \"ghcr.io/org/demo\",\n\
                 dependencies: {\n\
                     zlib: { native: \"z\", system: true, version: \">=1.2.0\" },\n\
                     codec: { native: \"codec\", from: \"libcodec.so\" }\n\
                 }\n\
             }\n",
        );
        assert_eq!(
            c.dependencies["zlib"],
            DependencySpec::Native {
                lib: "z".to_string(),
                system: true,
                path: None,
                version: Some(">=1.2.0".to_string()),
                from: None,
                link: crate::license::LinkPreference::Auto,
            }
        );
        assert_eq!(
            c.dependencies["codec"],
            DependencySpec::Native {
                lib: "codec".to_string(),
                system: false,
                path: None,
                version: None,
                from: Some("libcodec.so".to_string()),
                link: crate::license::LinkPreference::Auto,
            }
        );
        let m = parse_manifest(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 artifacts: \"ghcr.io/org/demo\",\n\
                 dependencies: { codec: { native: \"codec\", from: \"libcodec.so\" } }\n\
             }\n",
        )
        .unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(out.contains("from:"), "{out}");
        let again = parse_manifest(&out).unwrap().0.unwrap();
        assert_eq!(again.dependencies, m.0.unwrap().dependencies);
    }

    #[test]
    fn native_rejects_bad_source_combinations() {
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    artifacts: \"ghcr.io/org/demo\",\n    dependencies: { z: { native: \"z\", system: true, from: \"libz.so\" } }\n}\n",
                "cannot combine",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { z: { native: \"z\", system: true, path: \"./z\" } }\n}\n",
                "cannot combine",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { z: { native: \"z\", path: \"./z\", version: \"^1.0.0\" } }\n}\n",
                "cannot combine",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { z: { native: \"z\", path: \"./z\", from: \"libz.so\" } }\n}\n",
                "cannot combine",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { z: { native: \"z\", system: true, version: \"nonsense!!\" } }\n}\n",
                "not a valid range",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { z: { native: \"z\", from: \"libz.so\" } }\n}\n",
                "own `artifacts` image",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }

    #[test]
    fn native_pins_parse_and_reject_bad_shapes() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 dependencies: { zlib: { native: \"z\", system: true } },\n\
                 native: { zlib: { system: true, version: \">=1.2.0\" } }\n\
             }\n",
        );
        assert_eq!(
            c.native_pins["zlib"],
            NativePin { system: true, version: Some(">=1.2.0".to_string()), from: None }
        );
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    native: { zlib: { version: \"^1.0.0\" } }\n}\n",
                "missing required field `system`",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    native: { zlib: { system: \"yes\" } }\n}\n",
                "must be a boolean",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    native: { zlib: { system: true, from: \"libz.so\" } }\n}\n",
                "cannot combine",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    native: { zlib: \"system\" }\n}\n",
                "must be an object",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    native: [\"zlib\"]\n}\n",
                "must be an object",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }

    #[test]
    fn tasks_parse_steps_groups_and_reject_bad_shapes() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 tasks: {\n\
                     check: [\"rnx check\", { run: \"rnx lint\" }],\n\
                     matrix: { steps: [{ run: \"rnx build --target x\" }], matrix: [\"x86_64-unknown-linux-gnu\"] },\n\
                     all: { tasks: [\"check\", \"matrix\"], strategy: \"parallel\", \"on-failure\": \"continue\" }\n\
                 }\n\
             }\n",
        );
        assert_eq!(
            c.tasks["check"],
            TaskDef::Steps {
                steps: vec![
                    TaskStep::Run("rnx check".to_string()),
                    TaskStep::Run("rnx lint".to_string()),
                ],
                matrix: Vec::new(),
            }
        );
        assert_eq!(
            c.tasks["matrix"],
            TaskDef::Steps {
                steps: vec![TaskStep::Run("rnx build --target x".to_string())],
                matrix: vec!["x86_64-unknown-linux-gnu".to_string()],
            }
        );
        assert_eq!(
            c.tasks["all"],
            TaskDef::Group {
                tasks: vec!["check".to_string(), "matrix".to_string()],
                strategy: TaskStrategy::Parallel,
                on_failure: TaskOnFailure::Continue,
            }
        );
        let m = parse_manifest(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 tasks: { check: [\"rnx check\"], all: { tasks: [\"check\"] } }\n\
             }\n",
        )
        .unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(out.contains("tasks:"), "{out}");
        let again = parse_manifest(&out).unwrap().0.unwrap();
        assert_eq!(again.tasks, m.0.unwrap().tasks);
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { bindgen: [\"x\"] }\n}\n",
                "reserved",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { bad: { steps: [\"ok\"], tasks: [\"ok\"] } }\n}\n",
                "exactly one of",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { bad: { tasks: [\"x\"], strategy: \"sideways\" } }\n}\n",
                "must be `serial` or `parallel`",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { bad: { tasks: [] } }\n}\n",
                "must not be empty",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: { bad: { steps: [\"ok\"], matrix: [\"nope\"] } }\n}\n",
                "must be arch-vendor-os-env triples",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    tasks: [\"x\"]\n}\n",
                "must be an object",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }

    #[test]
    fn bindgen_driver_parses_and_round_trips() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 bindgen: {\n\
                     headers: [\"include/foo.h\"],\n\
                     include: [\"include\"],\n\
                     prefix: [\"foo_\"],\n\
                     skip: [\"foo_debug\"],\n\
                     native_lib: \"foo\",\n\
                     upstream: \"foo-src\",\n\
                     outputs: { \"src/gen\": { title: \"foo.rnx\", target: \"x86_64-unknown-linux-gnu\" } }\n\
                 }\n\
             }\n",
        );
        let driver = c.bindgen.unwrap();
        assert_eq!(driver.headers, vec!["include/foo.h".to_string()]);
        assert_eq!(driver.native_lib.as_deref(), Some("foo"));
        assert_eq!(
            driver.outputs["src/gen"],
            BindgenOutputDecl { title: "foo.rnx".to_string(), target: Some("x86_64-unknown-linux-gnu".to_string()) }
        );
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: { outputs: { \"x\": { title: \"a.rnx\" } } }\n}\n",
                "missing required field `headers`",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: { headers: [], outputs: { \"x\": { title: \"a.rnx\" } } }\n}\n",
                "at least one header",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: { headers: [\"a.h\"], outputs: {} }\n}\n",
                "at least one output",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    bindgen: { headers: [\"a.h\"], outputs: { \"x\": { title: \"../evil.rnx\" } } }\n}\n",
                "must be a plain filename",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }

    #[test]
    fn evaluates_spread_and_conditionals() {
        let module = Parser::parse_module(
            "const commonDeps = { json: \"1.2.0\" }\n\
             export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 dependencies: {\n\
                     ...commonDeps,\n\
                     zlib: target.os == \"windows\" ? { native: \"zlibstatic\", system: true } : { native: \"z\", system: true },\n\
                     gui: switch (target.arch) { case \"aarch64\": { native: \"gui_arm\" } default: { native: \"gui_x86\" } }\n\
                 },\n\
                 permissions: [{ perm: \"fs:read:/data\", reason: \"fixtures\" }, \"net:http:example.com\"]\n\
             }\n",
        )
        .unwrap();
        let v = eval_module(&module, target()).unwrap();
        let c = parse_manifest(
            "const commonDeps = { json: \"1.2.0\" }\n\
             export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 dependencies: {\n\
                     ...commonDeps,\n\
                     zlib: target.os == \"windows\" ? { native: \"zlibstatic\", system: true } : { native: \"z\", system: true },\n\
                     gui: switch (target.arch) { case \"aarch64\": { native: \"gui_arm\" } default: { native: \"gui_x86\" } }\n\
                 },\n\
                 permissions: [{ perm: \"fs:read:/data\", reason: \"fixtures\" }, \"net:http:example.com\"]\n\
             }\n",
        )
        .unwrap()
        .0
        .unwrap();
        assert_eq!(v.get("project").and_then(|p| p.get("name")), Some(&eval::ConfigValue::String("demo".to_string())));
        assert_eq!(
            c.dependencies["json"],
            DependencySpec::Semver { version: "1.2.0".to_string(), link: crate::license::LinkPreference::Auto }
        );
        assert_eq!(
            c.dependencies["zlib"],
            DependencySpec::Native { lib: "z".to_string(), system: true, path: None, version: None, from: None, link: crate::license::LinkPreference::Auto }
        );
        assert_eq!(
            c.dependencies["gui"],
            DependencySpec::Native { lib: "gui_x86".to_string(), system: false, path: None, version: None, from: None, link: crate::license::LinkPreference::Auto }
        );
        assert_eq!(c.permissions, Some(vec![PermissionDecl { perm: "fs:read:/data".to_string(), reason: Some("fixtures".to_string()) }, PermissionDecl { perm: "net:http:example.com".to_string(), reason: None }]));
    }

    #[test]
    fn manifest_round_trips_through_serializer() {
        let text = "export default {\n\
             project: { name: \"demo\", version: \"0.1.0\" },\n\
             entries: { main: \"src/app.rnx\", bins: { tool: \"src/bin/tool.rnx\" } },\n\
             dependencies: { helper: \"libs/helper\", zlib: { native: \"z\", system: true } },\n\
             permissions: [{ perm: \"fs:read:/data\", reason: \"seed data\" }, \"term:write\"]\n\
         }\n";
        let m = parse_manifest(text).unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(!out.contains("entry:"), "{out}");
        assert!(!out.contains("edition"), "{out}");
        let again = parse_manifest(&out).unwrap();
        let (first, second) = (again.0.unwrap(), m.0.unwrap());
        assert_eq!(first.entries, second.entries);
        assert_eq!(first.dependencies, second.dependencies);
        assert_eq!(first.permissions, second.permissions);
        assert_eq!(
            first.permissions,
            Some(vec![
                PermissionDecl {
                    perm: "fs:read:/data".to_string(),
                    reason: Some("seed data".to_string())
                },
                PermissionDecl { perm: "term:write".to_string(), reason: None }
            ])
        );
    }

    #[test]
    fn permissions_accept_strings_tables_and_mixed() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 permissions: [\"term:write\", { perm: \"fs:read:/data\", reason: \"seed\" }]\n\
             }\n",
        );
        assert_eq!(
            c.permissions,
            Some(vec![
                PermissionDecl { perm: "term:write".to_string(), reason: None },
                PermissionDecl {
                    perm: "fs:read:/data".to_string(),
                    reason: Some("seed".to_string())
                }
            ])
        );
        let bare = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 permissions: { allowed: [\"fs:delegated\"] }\n\
             }\n",
        );
        assert_eq!(
            bare.permissions,
            Some(vec![PermissionDecl { perm: "fs:delegated".to_string(), reason: None }])
        );
    }

    #[test]
    fn permissions_reject_bad_perm_and_non_string_reason() {
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    permissions: [\"bogus:cap\"]\n}\n",
                "invalid permission",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    permissions: [{ perm: \"fs:read\", reason: \"x\" }]\n}\n",
                "invalid permission",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    permissions: [{ perm: \"term:write\", reason: 7 }]\n}\n",
                "must be a string",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    permissions: [{ reason: \"x\" }]\n}\n",
                "missing required field `perm`",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    permissions: [42]\n}\n",
                "must be strings or",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    permissions: { allowed: [{ perm: \"term:write\" }] }\n}\n",
                "must be strings",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
        }
    }

    #[test]
    fn keywords_parse_and_round_trip() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 keywords: [\"http\", \"cli-2\", \"json5\"]\n\
             }\n",
        );
        assert_eq!(c.keywords, vec!["http".to_string(), "cli-2".to_string(), "json5".to_string()]);
        let m = parse_manifest(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 keywords: [\"http\", \"cli-2\"]\n\
             }\n",
        )
        .unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(out.contains("keywords:"), "{out}");
        let again = parse_manifest(&out).unwrap().0.unwrap();
        assert_eq!(again.keywords, vec!["http".to_string(), "cli-2".to_string()]);
    }

    #[test]
    fn keywords_default_to_empty() {
        let c = parse("export default {\n    project: { name: \"demo\", version: \"0.1.0\" }\n}\n");
        assert!(c.keywords.is_empty());
    }

    #[test]
    fn keywords_reject_bad_shapes() {
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    keywords: [\"ok\", \"Bad\"]\n}\n",
                "lowercase",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    keywords: [\"has space\"]\n}\n",
                "lowercase",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    keywords: [\"under_score\"]\n}\n",
                "lowercase",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    keywords: [\"\"]\n}\n",
                "must not be empty",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    keywords: [\"aaaaaaaaaaaaaaaaaaaaaaaaa\"]\n}\n",
                "exceeds 24",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    keywords: [\"ok\", 7]\n}\n",
                "must be strings",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    keywords: \"http\"\n}\n",
                "must be an array",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }

    #[test]
    fn keywords_reject_too_many() {
        let words: Vec<String> = (0..13).map(|i| format!("kw-{i}")).collect();
        let list = words.iter().map(|w| format!("\"{w}\"")).collect::<Vec<_>>().join(", ");
        let text = format!(
            "export default {{\n    project: {{ name: \"demo\", version: \"0.1.0\" }},\n    keywords: [{list}]\n}}\n"
        );
        let e = parse_config(&text).unwrap_err();
        assert!(e.contains("at most 12"), "{e}");
        let issues = validate_manifest_text(&text, None);
        assert!(
            issues.iter().any(|i| i.error && i.code == Code::E108 && i.message.contains("at most 12")),
            "{issues:?}"
        );
    }

    #[test]
    fn artifacts_accept_single_bare_repo_string() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 artifacts: \"ghcr.io/org/sqlite3\"\n\
             }\n",
        );
        assert_eq!(c.artifacts.as_deref(), Some("ghcr.io/org/sqlite3"));
        let m = parse_manifest(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 artifacts: \"ghcr.io/org/sqlite3\"\n\
             }\n",
        )
        .unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(out.contains("artifacts:"), "{out}");
        let again = parse_manifest(&out).unwrap().0.unwrap();
        assert_eq!(again.artifacts.as_deref(), Some("ghcr.io/org/sqlite3"));
    }

    #[test]
    fn artifacts_default_to_none() {
        let c = parse("export default {\n    project: { name: \"demo\", version: \"0.1.0\" }\n}\n");
        assert_eq!(c.artifacts, None);
    }

    #[test]
    fn artifacts_reject_digest_bearing_and_keyed_tables() {
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    artifacts: \"ghcr.io/org/sqlite3@sha256:abc\"\n}\n",
                "bare OCI repo ref",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    artifacts: \"ghcr.io/org/sqlite3:0.3.0\"\n}\n",
                "bare OCI repo ref",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    artifacts: { native: { ref: \"ghcr.io/org/codec:0.3.0\", digest: \"sha256:abc\" } }\n}\n",
                "keyed `artifacts` tables are rejected",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    artifacts: 7\n}\n",
                "bare OCI repo ref",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }

    #[test]
    fn link_defaults_to_auto_and_round_trips() {
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 dependencies: { zlib: { native: \"z\", system: true } }\n\
             }\n",
        );
        assert_eq!(
            c.dependencies["zlib"],
            DependencySpec::Native {
                lib: "z".to_string(),
                system: true,
                path: None,
                version: None,
                from: None,
                link: crate::license::LinkPreference::Auto,
            }
        );
        let c = parse(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 artifacts: \"ghcr.io/org/demo\",\n\
                 dependencies: { codec: { native: \"codec\", from: \"libcodec.so\", link: \"static\" } }\n\
             }\n",
        );
        assert_eq!(
            c.dependencies["codec"],
            DependencySpec::Native {
                lib: "codec".to_string(),
                system: false,
                path: None,
                version: None,
                from: Some("libcodec.so".to_string()),
                link: crate::license::LinkPreference::Static,
            }
        );
        let m = parse_manifest(
            "export default {\n\
                 project: { name: \"demo\", version: \"0.1.0\" },\n\
                 artifacts: \"ghcr.io/org/demo\",\n\
                 dependencies: { codec: { native: \"codec\", from: \"libcodec.so\", link: \"static\" } }\n\
             }\n",
        )
        .unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(out.contains("link:"), "{out}");
        let again = parse_manifest(&out).unwrap().0.unwrap();
        assert_eq!(again.dependencies, m.0.unwrap().dependencies);
    }

    #[test]
    fn link_rejects_unknown_values_and_dynamic_on_source_deps() {
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    artifacts: \"ghcr.io/org/demo\",\n    dependencies: { z: { native: \"z\", from: \"libz.so\", link: \"shared\" } }\n}\n",
                "invalid `link` value",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { z: { native: \"z\", system: true, link: 7 } }\n}\n",
                "`link` must be a string",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { dep: { path: \"../dep\", link: \"dynamic\" } }\n}\n",
                "needs a native dependency",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    dependencies: { dep: { version: \"^1.0.0\", link: \"dynamic\" } }\n}\n",
                "needs a native dependency",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }

    #[test]
    fn license_accepts_known_spdx_and_rejects_unknown() {
        let c = parse(
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    license: \"Apache-2.0\"\n}\n",
        );
        assert_eq!(c.license.as_deref(), Some("Apache-2.0"));
        let m = parse_manifest(
            "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    license: \"MIT\"\n}\n",
        )
        .unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(out.contains("license:"), "{out}");
        let again = parse_manifest(&out).unwrap().0.unwrap();
        assert_eq!(again.license.as_deref(), Some("MIT"));
        for (text, needle) in [
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    license: \"GPL-2.0\"\n}\n",
                "not a known SPDX",
            ),
            (
                "export default {\n    project: { name: \"demo\", version: \"0.1.0\" },\n    license: 7\n}\n",
                "must be a string",
            ),
        ] {
            let e = parse_config(text).unwrap_err();
            assert!(e.contains(needle), "{e} for {text}");
            let issues = validate_manifest_text(text, None);
            assert!(
                issues.iter().any(|i| i.error && i.code == Code::E108),
                "{issues:?} for {text}"
            );
        }
    }
}
