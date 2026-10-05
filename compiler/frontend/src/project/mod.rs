pub mod eval;

use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "Project.config";
pub const DEFAULT_ENTRY: &str = "src/main.rnx";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DependencySpec {
    Semver { version: String },
    Path { path: PathBuf },
    Git { git: String, rev: String },
    Url { version: String, url: String, checksum: Option<String> },
    Native { lib: String, system: bool, path: Option<PathBuf> },
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
    pub permissions: Option<Vec<String>>,
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

fn eval_config_text(text: &str) -> Result<ConfigValue, String> {
    let module = Parser::parse_module(text).map_err(|e| e.message.clone())?;
    eval_module(&module, ConfigTarget::host()).map_err(|e| e.message.clone())
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

fn parse_dependency(name: &str, val: &ConfigValue) -> Result<DependencySpec, String> {
    match val {
        ConfigValue::String(s) => {
            if s.starts_with('.') || s.contains('/') {
                return Ok(DependencySpec::Path {
                    path: PathBuf::from(s),
                });
            }
            if s.is_empty() {
                return Err(format!("dependency `{name}`: version must not be empty"));
            }
            Ok(DependencySpec::Semver { version: s.clone() })
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
                if has_git || has_url || has_version {
                    return Err(format!("dependency `{name}`: `native` must not mix with `git` or `version`/`url`"));
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
                return Ok(DependencySpec::Native { lib, system, path });
            }
            let kinds = [has_path, has_git, has_url || (has_version && !has_git && !has_path)].iter().filter(|&&b| b).count();
            if kinds > 1 {
                return Err(format!("dependency `{name}`: specify only one of `path`, `git`, `url`, or `version`"));
            }
            if has_path {
                if has_git || has_url || has_version {
                    return Err(format!("dependency `{name}`: specify either `path`, `git`, or `version`/`url`, not a mix"));
                }
                return match obj_get(val, "path") {
                    Some(ConfigValue::String(s)) => Ok(DependencySpec::Path {
                        path: PathBuf::from(s),
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
                return Ok(DependencySpec::Git { git, rev });
            }
            if has_url {
                let url = match obj_get(val, "url") {
                    Some(ConfigValue::String(s)) => s.clone(),
                    _ => return Err(format!("dependency `{name}`: `url` must be a string")),
                };
                let version = get_str("version").unwrap_or_default();
                let checksum = get_str("checksum");
                return Ok(DependencySpec::Url { version, url, checksum });
            }
            if has_version {
                return match obj_get(val, "version") {
                    Some(ConfigValue::String(s)) => Ok(DependencySpec::Semver { version: s.clone() }),
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

fn parse_manifest(text: &str) -> Result<(Option<ProjectConfig>, Option<WorkspaceConfig>), String> {
    let root = eval_config_text(text)?;
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
                        match item {
                            ConfigValue::String(s) => allowed.push(s.clone()),
                            _ => return Err("field `permissions` must be strings".to_string()),
                        }
                    }
                    Some(allowed)
                }
                Some(p) => match obj_get(p, "allowed") {
                    Some(ConfigValue::Array(items)) => {
                        let mut allowed = Vec::new();
                        for item in items {
                            match item {
                                ConfigValue::String(s) => allowed.push(s.clone()),
                                _ => {
                                    return Err(
                                        "field `permissions.allowed` must be strings".to_string()
                                    );
                                }
                            }
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
const DEP_KEYS: &[&str] = &["path", "git", "rev", "tag", "branch", "version", "url", "checksum", "native", "system"];
const TOP_LEVEL_KEYS: &[&str] = &["project", "entries", "dependencies", "permissions", "registry", "registries", "workspace"];

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
            for (i, c) in allowed.iter().enumerate() {
                if i > 0 {
                    out.push_str(", ");
                }
                out.push_str(&crate::deplock::rnx_string(c));
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
        DependencySpec::Semver { version } => crate::deplock::rnx_string(version),
        DependencySpec::Path { path } => {
            format!("{{ path: {} }}", crate::deplock::rnx_string(&path.to_string_lossy()))
        }
        DependencySpec::Git { git, rev } => {
            format!(
                "{{ git: {}, rev: {} }}",
                crate::deplock::rnx_string(git),
                crate::deplock::rnx_string(rev)
            )
        }
        DependencySpec::Url { version, url, checksum } => {
            let mut s = String::from("{ ");
            if !version.is_empty() {
                s.push_str(&format!("version: {}, ", crate::deplock::rnx_string(version)));
            }
            s.push_str(&format!("url: {}", crate::deplock::rnx_string(url)));
            if let Some(c) = checksum {
                s.push_str(&format!(", checksum: {}", crate::deplock::rnx_string(c)));
            }
            s.push_str(" }");
            s
        }
        DependencySpec::Native { lib, system, path } => {
            let mut s = format!("{{ native: {}", crate::deplock::rnx_string(lib));
            if *system {
                s.push_str(", system: true");
            }
            if let Some(p) = path {
                s.push_str(&format!(", path: {}", crate::deplock::rnx_string(&p.to_string_lossy())));
            }
            s.push_str(" }");
            s
        }
    }
}

pub fn manifest_sections() -> &'static [&'static str] {
    &["project", "entries", "registry", "dependencies", "workspace", "permissions", "registries"]
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
        "dependencies" => Some("Semver (`\"^1.2.0\"`), path (`\"libs/x\"` or `{ path = \"...\" }`), git (`{ git = \"<url>\", rev/tag/branch = \"...\" }`), tarball (`{ version = \"...\", url = \"...\", checksum = \"...\" }`), or native (`{ native = \"z\", system = true }`)."),
        "workspace" => Some("Monorepo members: `members = [\"alpha\", \"beta\"]` or globs like `[\"crates/*\"]`."),
        "permissions" => Some("Security ceiling: an array of capability strings the package and its dependencies may use."),
        "registries" => Some("Scoped registry overrides keyed by scope, e.g. `registries: { \"@acme\": { url = \"...\" } }`."),
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
        (_, "version") => Some("Semver requirement, e.g. `\"^3.45.0\"` or `{ version = \"1.5.5\", url = \"...\" }`."),
        (_, "url") => Some("Direct tarball URL, paired with `version` and optional `checksum`."),
        (_, "checksum") => Some("Expected `sha256:...` digest of a tarball URL dependency."),
        (_, "native") => Some("System native library dependency; links `-l<lib>` and records `native:<lib>` in the lockfile."),
        (_, "system") => Some("Whether a native dependency comes from the host system (`true`) or a vendored path."),
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
                Ok(DependencySpec::Path { path }) => {
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
                path: PathBuf::from("../physics_2d")
            }
        );
        assert_eq!(
            c.dependencies["helper"],
            DependencySpec::Path {
                path: PathBuf::from("libs/helper")
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
                rev: "main".to_string()
            }
        );
        assert_eq!(
            c.dependencies["json"],
            DependencySpec::Semver { version: "^1.2.0".to_string() }
        );
        assert_eq!(
            c.dependencies["zlib"],
            DependencySpec::Native { lib: "z".to_string(), system: true, path: None }
        );
        assert_eq!(
            c.dependencies["vendored"],
            DependencySpec::Native {
                lib: "gui".to_string(),
                system: false,
                path: Some(PathBuf::from("./native"))
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
                 permissions: [\"native:zlib\", \"native:gui\"]\n\
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
                 permissions: [\"native:zlib\", \"native:gui\"]\n\
             }\n",
        )
        .unwrap()
        .0
        .unwrap();
        assert_eq!(v.get("project").and_then(|p| p.get("name")), Some(&eval::ConfigValue::String("demo".to_string())));
        assert_eq!(
            c.dependencies["json"],
            DependencySpec::Semver { version: "1.2.0".to_string() }
        );
        assert_eq!(
            c.dependencies["zlib"],
            DependencySpec::Native { lib: "z".to_string(), system: true, path: None }
        );
        assert_eq!(
            c.dependencies["gui"],
            DependencySpec::Native { lib: "gui_x86".to_string(), system: false, path: None }
        );
        assert_eq!(c.permissions, Some(vec!["native:zlib".to_string(), "native:gui".to_string()]));
    }

    #[test]
    fn manifest_round_trips_through_serializer() {
        let text = "export default {\n\
             project: { name: \"demo\", version: \"0.1.0\" },\n\
             entries: { main: \"src/app.rnx\", bins: { tool: \"src/bin/tool.rnx\" } },\n\
             dependencies: { helper: \"libs/helper\", zlib: { native: \"z\", system: true } },\n\
             permissions: [\"native:zlib\"]\n\
         }\n";
        let m = parse_manifest(text).unwrap();
        let out = manifest_to_rnx(&Manifest { project: m.0.clone(), workspace: m.1.clone() });
        assert!(!out.contains("entry:"), "{out}");
        assert!(!out.contains("edition"), "{out}");
        let again = parse_manifest(&out).unwrap();
        let (first, second) = (again.0.unwrap(), m.0.unwrap());
        assert_eq!(first.entries, second.entries);
        assert_eq!(first.dependencies, second.dependencies);
    }
}
