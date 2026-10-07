use crate::checksum::Sha256;
use crate::project::{self, ProjectConfig, RegistryConfig};
use diagnostics::{Code, Diagnostic};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const REGISTRY_SPEC: u32 = 1;
pub const DEFAULT_REGISTRY_DOMAIN: &str = "rasmalai.rovelstars.com";
const REQUEST_TIMEOUT_SECS: u64 = 30;
const MAX_DOWNLOAD_BYTES: usize = 8 * 1024 * 1024;
const MAX_CHUNK_BYTES: usize = 1024 * 1024;
const CHUNK_FETCH_CONCURRENCY: usize = 8;

pub fn cache_anchor(proj_root: &Path) -> PathBuf {
    project::find_workspace_root_strict(proj_root).unwrap_or_else(|| proj_root.to_path_buf())
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
    let _ = std::fs::remove_dir_all(&dir);
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
        let _ = std::fs::remove_dir_all(&dir);
        git_cmd(&["clone", git_url, &dir.to_string_lossy()], cache_dir)?;
        git_cmd(&["checkout", rev], &dir).map_err(|e| {
            let _ = std::fs::remove_dir_all(&dir);
            Diagnostic::new(
                Code::E108,
                format!("cannot reach revision `{rev}` of `{git_url}`: {}", e.message),
            )
        })?;
    }
    if !dir.join(project::MANIFEST_FILE).is_file() {
        let _ = std::fs::remove_dir_all(&dir);
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
    let vendor = anchor.join("vendor").join(name);
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
            let crate::project::DependencySpec::Git { git, rev } = spec else {
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

pub fn vendor_all(scope_root: &Path) -> Result<Vec<String>, Diagnostic> {
    let fetched = fetch_all_git_deps(scope_root)?;
    let mut names: Vec<String> = Vec::new();
    for (name, url, rev) in &fetched {
        let dest = scope_root.join("vendor").join(name);
        let _ = std::fs::remove_dir_all(&dest);
        let src = resolve_git_dep(scope_root, name, url, rev)?;
        copy_tree(&src, &dest)?;
        names.push(name.clone());
    }
    names.sort();
    Ok(names)
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
    let trimmed = raw.trim().trim_end_matches('/');
    if trimmed.is_empty() {
        return format!("https://{DEFAULT_REGISTRY_DOMAIN}");
    }
    if trimmed.contains("://") {
        return trimmed.to_string();
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

fn engine_satisfied(range: &str, current: &str) -> bool {
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

pub trait RegistryTransport: Send + Sync {
    fn get(&self, url: &str) -> Result<RegistryReply, Diagnostic>;
    fn post_json(&self, url: &str, body: &str) -> Result<RegistryReply, Diagnostic>;
}

static TRANSPORT: std::sync::Mutex<Option<std::sync::Arc<dyn RegistryTransport>>> =
    std::sync::Mutex::new(None);

pub fn set_registry_transport(transport: std::sync::Arc<dyn RegistryTransport>) {
    let mut slot = TRANSPORT.lock().unwrap_or_else(|e| e.into_inner());
    *slot = Some(transport);
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

fn read_http_response(stream: &mut std::net::TcpStream) -> Result<RegistryReply, Diagnostic> {
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
    let status = lines
        .next()
        .unwrap_or_default()
        .split_whitespace()
        .nth(1)
        .and_then(|s| s.parse::<u16>().ok())
        .ok_or_else(|| Diagnostic::new(Code::E108, "registry returned a bad status line"))?;
    let mut spec_header = None;
    let mut content_length = None;
    let mut chunked = false;
    for line in lines {
        if let Some((name, value)) = line.split_once(':') {
            match name.trim().to_ascii_lowercase().as_str() {
                "rnx-registry-spec" => spec_header = Some(value.trim().to_string()),
                "content-length" => content_length = value.trim().parse::<usize>().ok(),
                "transfer-encoding" => {
                    chunked = value.to_ascii_lowercase().contains("chunked");
                }
                _ => {}
            }
        }
    }
    if chunked {
        return Ok(RegistryReply {
            status,
            spec_header,
            body: read_chunked(stream)?,
        });
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
    Ok(RegistryReply { status, spec_header, body })
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
}

fn std_request(
    method: &str,
    url: &str,
    body: Option<&[u8]>,
) -> Result<RegistryReply, Diagnostic> {
    use std::io::Write;
    use std::net::ToSocketAddrs;
    let (host, port, path) = split_http_url(url)?;
    let addr = format!("{host}:{port}");
    let target = addr
        .to_socket_addrs()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot reach registry `{addr}`: {e}")))?
        .next()
        .ok_or_else(|| Diagnostic::new(Code::E108, format!("cannot reach registry `{addr}`")))?;
    let mut stream = std::net::TcpStream::connect_timeout(
        &target,
        std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS),
    )
    .map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot reach registry `{addr}`: {e}"))
    })?;
    stream
        .set_read_timeout(Some(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS)))
        .map_err(|e| Diagnostic::new(Code::E108, format!("registry request failed: {e}")))?;
    stream
        .set_write_timeout(Some(std::time::Duration::from_secs(REQUEST_TIMEOUT_SECS)))
        .map_err(|e| Diagnostic::new(Code::E108, format!("registry request failed: {e}")))?;
    let mut head = format!("{method} {path} HTTP/1.1\r\nHost: {host}\r\nConnection: close\r\n");
    if method == "POST" {
        head.push_str("Content-Type: application/json\r\nAccept: application/json\r\n");
    } else {
        head.push_str("Accept: application/json\r\n");
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
    read_http_response(&mut stream)
}

fn retryable_status(status: u16) -> bool {
    matches!(status, 429 | 502 | 503 | 504)
}

fn http_get(url: &str, what: &str) -> Result<RegistryReply, Diagnostic> {
    let mut attempt = 0u32;
    loop {
        match transport().get(url) {
            Ok(reply) => {
                if retryable_status(reply.status) && attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Ok(reply);
            }
            Err(e) => {
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
    let mut attempt = 0u32;
    loop {
        match transport().post_json(url, body) {
            Ok(reply) => {
                if retryable_status(reply.status) && attempt < 3 {
                    std::thread::sleep(std::time::Duration::from_millis(resolve_retry_delay_ms(attempt)));
                    attempt += 1;
                    continue;
                }
                return Ok(reply);
            }
            Err(e) => {
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

fn resolve_request_body(
    requirements: &BTreeMap<String, String>,
    have: &[HaveEntry],
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
    out.push_str("]}");
    out
}

pub fn resolve_retry_delay_ms(attempt: u32) -> u64 {
    150 * 2u64.saturating_pow(attempt.min(4))
}

pub fn resolve_requirements(
    base: &str,
    requirements: &BTreeMap<String, String>,
    have: &[HaveEntry],
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
    let body = resolve_request_body(requirements, have);
    let url = format!("{}/api/resolve", base.trim_end_matches('/'));
    let reply = http_post_json(&url, &body, "registry resolve")?;
    if reply.status == 200 {
        return parse_resolve(&reply.body);
    }
    let value = parse_json(&reply.body).unwrap_or(JsonValue::Null);
    let message = json_string(&value, "message");
    let message = if message.is_empty() {
        format!("registry resolve failed with HTTP {}", reply.status)
    } else {
        message
    };
    Err(Diagnostic::new(Code::E108, message))
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
    Ok(ResolveNode {
        full: req("full")?,
        version: req("version")?,
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
    })
}

fn host_of(base: &str) -> String {
    let without_scheme = match base.split_once("://") {
        Some((_, rest)) => rest,
        None => base,
    };
    let host = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host = host.split('@').last().unwrap_or(host);
    sanitize_segment(host)
}

fn sanitize_segment(s: &str) -> String {
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

fn sanitize_full(full: &str) -> String {
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

pub fn cached_package_dir(base: &str, full: &str, version: &str) -> PathBuf {
    registry_cache_root()
        .join(host_of(base))
        .join(sanitize_full(full))
        .join(sanitize_segment(version))
}

fn integrity_path(dir: &Path) -> PathBuf {
    dir.join(".rnx-integrity")
}

pub fn cached_integrity(dir: &Path) -> Option<String> {
    let text = std::fs::read_to_string(integrity_path(dir)).ok()?;
    let mut lines = text.lines();
    lines.next()?;
    lines.next()?;
    lines.next().map(|s| s.trim().to_string())
}

pub fn scan_cache_have() -> Vec<HaveEntry> {
    let mut out = Vec::new();
    let root = registry_cache_root();
    let mut stack = vec![root];
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

fn check_engine(node: &ResolveNode) -> Result<(), Diagnostic> {
    let range = node.engine_range.trim();
    if range.is_empty() {
        return Ok(());
    }
    let current = env!("CARGO_PKG_VERSION");
    if engine_satisfied(range, current) {
        return Ok(());
    }
    Err(Diagnostic::new(
        Code::E108,
        format!("`{}@{}` needs engine {range}, have {current}", node.full, node.version),
    ))
}

fn warn_if_yanked(node: &ResolveNode) {
    if node.yanked {
        eprintln!("warning: package `{}@{}` is yanked and should be avoided", node.full, node.version);
    }
}

struct ChunkEntry {
    name: String,
    size: u64,
    dir: bool,
    chunks: Vec<String>,
}

fn fetch_chunks_manifest(url: &str, full: &str, version: &str) -> Result<Vec<ChunkEntry>, Diagnostic> {
    let reply = http_get(url, &format!("chunk manifest of `{full}@{version}`"))?;
    if reply.status != 200 {
        return Err(chunk_fetch_error(reply.status, &reply.body, full, version));
    }
    let value = parse_json(&reply.body).map_err(|_| {
        Diagnostic::new(Code::E108, format!("chunk manifest of `{full}@{version}` is malformed"))
    })?;
    let items = value
        .get("entries")
        .and_then(|v| v.as_array())
        .ok_or_else(|| {
            Diagnostic::new(Code::E108, format!("chunk manifest of `{full}@{version}` is malformed"))
        })?;
    let mut out = Vec::with_capacity(items.len());
    for item in items {
        let obj = match item {
            JsonValue::Object(fields) => fields,
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("chunk manifest of `{full}@{version}` is malformed"),
                ));
            }
        };
        let str_field = |key: &str| {
            obj.iter()
                .find(|(k, _)| k == key)
                .and_then(|(_, v)| v.as_str())
                .map(|s| s.to_string())
                .ok_or_else(|| {
                    Diagnostic::new(Code::E108, format!("chunk manifest of `{full}@{version}` is malformed"))
                })
        };
        let size = obj
            .iter()
            .find(|(k, _)| k == "size")
            .and_then(|(_, v)| v.as_u64())
            .ok_or_else(|| {
                Diagnostic::new(Code::E108, format!("chunk manifest of `{full}@{version}` is malformed"))
            })?;
        let dir = obj
            .iter()
            .find(|(k, _)| k == "dir")
            .and_then(|(_, v)| v.as_bool())
            .unwrap_or(false);
        let hashes = match obj.iter().find(|(k, _)| k == "chunks").map(|(_, v)| v) {
            Some(JsonValue::Array(hashes)) => {
                let mut out = Vec::with_capacity(hashes.len());
                for h in hashes {
                    out.push(h.as_str().ok_or_else(|| {
                        Diagnostic::new(Code::E108, format!("chunk manifest of `{full}@{version}` is malformed"))
                    })?.to_string());
                }
                out
            }
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("chunk manifest of `{full}@{version}` is malformed"),
                ));
            }
        };
        out.push(ChunkEntry { name: str_field("name")?, size, dir, chunks: hashes });
    }
    Ok(out)
}

fn fetch_chunk(url: &str, hash: &str, full: &str, version: &str) -> Result<Vec<u8>, Diagnostic> {
    if hash.len() != 64 || !hash.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err(Diagnostic::new(Code::E108, format!("bad chunk hash `{hash}`")));
    }
    let reply = http_get(url, &format!("chunk {hash} of `{full}@{version}`"))?;
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

fn fetch_chunks_parallel(
    chunk_base: &str,
    hashes: &[String],
    full: &str,
    version: &str,
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
                let url = format!("{chunk_base}/chunk/{hash}");
                let got = fetch_chunk(&url, hash, full, version);
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
        404 => Diagnostic::new(
            Code::E108,
            format!("package `{full}@{version}` does not exist"),
        ),
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

fn rebuild_tar(entries: &[ChunkEntry], by_hash: &BTreeMap<String, Vec<u8>>) -> Result<Vec<u8>, Diagnostic> {
    let bad = || Diagnostic::new(Code::E108, "chunk manifest failed to reassemble".to_string());
    let fallback = entries.len() == 1 && entries[0].name.is_empty() && !entries[0].dir;
    let mut out: Vec<u8> = Vec::new();
    for e in entries {
        if e.name.is_empty() && !e.dir && !fallback {
            return Err(bad());
        }
        if e.dir {
            if !e.chunks.is_empty() {
                return Err(bad());
            }
        }
        if fallback {
            for h in &e.chunks {
                out.extend_from_slice(by_hash.get(h).ok_or_else(bad)?);
            }
            continue;
        }
        if e.dir {
            // header-only entry below
        }
        let name = e.name.as_bytes();
        if name.len() > 100 {
            return Err(bad());
        }
        let mut head = vec![0u8; 512];
        head[..name.len()].copy_from_slice(name);
        write_tar_octal(&mut head, 100, 8, if e.dir { 0o755 } else { 0o644 });
        write_tar_octal(&mut head, 108, 8, 0);
        write_tar_octal(&mut head, 116, 8, 0);
        write_tar_octal(&mut head, 124, 12, e.size);
        write_tar_octal(&mut head, 136, 12, 0);
        for b in head.iter_mut().skip(148).take(8) {
            *b = b' ';
        }
        head[156] = if e.dir { b'5' } else { b'0' };
        head[257..263].copy_from_slice(b"ustar\0");
        head[263..265].copy_from_slice(b"00");
        head[265..268].copy_from_slice(b"rnx");
        head[297..300].copy_from_slice(b"rnx");
        let sum: u64 = head.iter().map(|b| *b as u64).sum();
        let sum_text = format!("{sum:06o}");
        head[148..154].copy_from_slice(&sum_text.as_bytes()[..6]);
        head[154] = 0;
        head[155] = b' ';
        out.extend_from_slice(&head);
        if e.dir {
            continue;
        }
        let mut done = 0u64;
        for h in &e.chunks {
            let bytes = by_hash.get(h).ok_or_else(bad)?;
            out.extend_from_slice(bytes);
            done += bytes.len() as u64;
        }
        if done != e.size {
            return Err(bad());
        }
        let pad = (512 - (e.size % 512)) % 512;
        out.extend(std::iter::repeat(0).take(pad as usize));
    }
    if !fallback {
        out.extend(std::iter::repeat(0).take(1024));
    }
    Ok(out)
}

fn tar_octal(bytes: &[u8]) -> Result<u64, Diagnostic> {
    let mut text = Vec::new();
    for b in bytes {
        if *b == 0 || *b == b' ' {
            break;
        }
        if !b.is_ascii_digit() || *b == b'8' || *b == b'9' {
            return Err(Diagnostic::new(Code::E108, "bad tar number".to_string()));
        }
        text.push(*b);
    }
    if text.is_empty() {
        return Ok(0);
    }
    let s = String::from_utf8(text)
        .map_err(|_| Diagnostic::new(Code::E108, "bad tar number".to_string()))?;
    u64::from_str_radix(&s, 8)
        .map_err(|_| Diagnostic::new(Code::E108, "bad tar number".to_string()))
}

fn tar_name(bytes: &[u8]) -> Result<String, Diagnostic> {
    let end = bytes.iter().position(|b| *b == 0).unwrap_or(bytes.len());
    std::str::from_utf8(&bytes[..end])
        .map(|s| s.to_string())
        .map_err(|_| Diagnostic::new(Code::E108, "bad tar name".to_string()))
}

fn tar_safe_rel(name: &str) -> Result<String, Diagnostic> {
    let trimmed = name.strip_suffix('/').unwrap_or(name);
    if trimmed.is_empty() || trimmed.starts_with('/') {
        return Err(Diagnostic::new(Code::E108, format!("unsafe tar path `{name}`")));
    }
    for part in trimmed.split('/') {
        if part.is_empty() || part == "." || part == ".." {
            return Err(Diagnostic::new(Code::E108, format!("unsafe tar path `{name}`")));
        }
    }
    Ok(trimmed.to_string())
}

struct TarFile {
    path: String,
    data: Vec<u8>,
}

fn parse_download_tar(data: &[u8]) -> Result<(Vec<String>, Vec<TarFile>), Diagnostic> {
    let mut dirs = Vec::new();
    let mut files = Vec::new();
    let mut pos = 0usize;
    while pos + 512 <= data.len() {
        let head = &data[pos..pos + 512];
        if head.iter().all(|b| *b == 0) {
            break;
        }
        if head.len() < 262 || &head[257..262] != b"ustar" {
            return Err(Diagnostic::new(Code::E108, "bad tar magic".to_string()));
        }
        let stored = tar_octal(&head[148..156])?;
        let mut sum = 0u64;
        for (i, b) in head.iter().enumerate() {
            sum += if (148..156).contains(&i) { b' ' as u64 } else { *b as u64 };
        }
        if sum != stored {
            return Err(Diagnostic::new(Code::E108, "tar checksum mismatch".to_string()));
        }
        let mut name = tar_name(&head[0..100])?;
        let prefix = tar_name(&head[345..394])?;
        if !prefix.is_empty() {
            name = format!("{prefix}/{name}");
        }
        let size = tar_octal(&head[124..136])?;
        let flag = head[156];
        if flag == b'L' || flag == b'K' {
            return Err(Diagnostic::new(Code::E108, "long tar names unsupported".to_string()));
        }
        let dir = flag == b'5';
        if !dir && flag != b'0' && flag != 0 {
            return Err(Diagnostic::new(Code::E108, "unsupported tar entry".to_string()));
        }
        let path = tar_safe_rel(&name)?;
        let blocks = size.div_ceil(512) as usize;
        if pos + 512 + blocks * 512 > data.len() {
            return Err(Diagnostic::new(Code::E108, "truncated tar archive".to_string()));
        }
        if dir {
            dirs.push(path);
        } else {
            files.push(TarFile {
                path,
                data: data[pos + 512..pos + 512 + size as usize].to_vec(),
            });
        }
        pos += 512 + blocks * 512;
    }
    Ok((dirs, files))
}

fn extract_download(bytes: &[u8], full: &str, dest: &Path) -> Result<(), Diagnostic> {
    let tar = if bytes.len() >= 2 && bytes[0] == 0x1F && bytes[1] == 0x8B {
        crate::gzip::decompress_gzip(bytes)?
    } else {
        bytes.to_vec()
    };
    let (dirs, mut files) = parse_download_tar(&tar)?;
    let mut ordered = dirs;
    ordered.sort();
    for dir in &ordered {
        std::fs::create_dir_all(dest.join(dir)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dest.display()))
        })?;
    }
    let mut parents: Vec<&str> = files
        .iter()
        .filter_map(|f| f.path.rsplit_once('/').map(|(p, _)| p))
        .collect();
    parents.sort();
    parents.dedup();
    for parent in parents {
        std::fs::create_dir_all(dest.join(parent)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dest.display()))
        })?;
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    for file in &files {
        std::fs::write(dest.join(&file.path), &file.data).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot write `{}`: {e}", dest.join(&file.path).display()),
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
        return format!("{}/api/packages", request_base.trim_end_matches('/'));
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

fn ensure_node(base: &str, api_base: &str, node: &ResolveNode) -> Result<PathBuf, Diagnostic> {
    let dir = cached_package_dir(base, &node.full, &node.version);
    if dir.join(crate::project::MANIFEST_FILE).is_file()
        && cached_integrity(&dir).as_deref() == Some(node.integrity.as_str())
    {
        check_engine(node)?;
        warn_if_yanked(node);
        return canonical_cached(&dir);
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
    let entries = fetch_chunks_manifest(&manifest_url, &node.full, &node.version)?;
    let mut uniq: BTreeSet<String> = BTreeSet::new();
    for entry in &entries {
        for hash in &entry.chunks {
            uniq.insert(hash.clone());
        }
    }
    let ordered: Vec<String> = uniq.into_iter().collect();
    let by_hash = fetch_chunks_parallel(&chunk_base, &ordered, &node.full, &node.version)?;
    let bytes = rebuild_tar(&entries, &by_hash)?;
    if bytes.len() > MAX_DOWNLOAD_BYTES {
        return Err(Diagnostic::new(
            Code::E108,
            format!("download of `{}@{}` exceeds size limits", node.full, node.version),
        ));
    }
    let digest = Sha256::hexdigest(&bytes);
    if !digest.eq_ignore_ascii_case(&node.integrity) {
        let _ = std::fs::remove_dir_all(&dir);
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "checksum mismatch for package `{}@{}`: download does not match registry integrity",
                node.full, node.version
            ),
        ));
    }
    let _ = std::fs::remove_dir_all(&dir);
    if let Some(parent) = dir.parent() {
        std::fs::create_dir_all(parent).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot create `{}`: {e}", parent.display()))
        })?;
    }
    extract_download(&bytes, &node.full, &dir)?;
    std::fs::write(integrity_path(&dir), format!("{}\n{}\n{}\n", node.full, node.version, node.integrity))
        .map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot write `{}`: {e}", dir.display()))
        })?;
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
        discover(base)?;
        let response = resolve_requirements(base, reqs, have)?;
        let api_base = api_base_for(&response.base, base);
        let mut nodes: BTreeMap<(String, String), &ResolveNode> = BTreeMap::new();
        for level in &response.levels {
            for node in level {
                nodes.insert((node.full.clone(), node.version.clone()), node);
            }
        }
        for ((full, _), node) in &nodes {
            let dir = ensure_node(base, &api_base, node)?;
            out.insert(
                full.clone(),
                FetchedPackage { name: full.clone(), version: node.version.clone(), dir },
            );
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

pub fn resolve_registry_package(
    pkg: &str,
    range: &str,
    default_registry: Option<&RegistryConfig>,
    overrides: &BTreeMap<String, RegistryConfig>,
    pinned: Option<String>,
    have: &[HaveEntry],
) -> Result<(PathBuf, ProjectConfig, String), Diagnostic> {
    if let Some(pin) = pinned.as_deref() {
        // Pinned versions skip the network when the cache already holds
        // them: the integrity file is written only after a verified
        // download, so its presence means completeness. This keeps
        // locked builds (and repeat installs) entirely off the wire.
        let base = registry_base_for(pkg, default_registry, overrides);
        let dir = cached_package_dir(&base, pkg, pin);
        if dir.join(crate::project::MANIFEST_FILE).is_file() && cached_integrity(&dir).is_some() {
            if let Ok(canonical) = std::fs::canonicalize(&dir) {
                if let Ok(Some(cfg)) = ProjectConfig::load_from_dir(&canonical) {
                    return Ok((canonical, cfg, pin.to_string()));
                }
            }
        }
    }
    let requirement = pinned.unwrap_or_else(|| range.to_string());
    let mut reqs = BTreeMap::new();
    reqs.insert(pkg.to_string(), requirement);
    let fetched = ensure_registry_requirements(&reqs, default_registry, overrides, have)?;
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
            if let crate::project::DependencySpec::Semver { version } = spec {
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
        pub body: Vec<u8>,
    }

    pub struct MockConfig {
        pub version_spec: u32,
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
    static TAG_COUNTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

    fn env_lock() -> &'static Mutex<()> {
        ENV_LOCK.get_or_init(|| Mutex::new(()))
    }

    pub struct EnvGuard {
        lock: Option<std::sync::MutexGuard<'static, ()>>,
        prev_cache: Option<String>,
        prev_proxies: Vec<(String, Option<String>)>,
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            unsafe {
                match self.prev_cache.clone() {
                    Some(v) => std::env::set_var("RNX_CACHE_HOME", v),
                    None => std::env::remove_var("RNX_CACHE_HOME"),
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

    pub fn isolate_cache(tag: &str) -> (EnvGuard, PathBuf) {
        let lock = env_lock().lock().unwrap_or_else(|e| e.into_inner());
        let n = TAG_COUNTER.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        let dir = std::env::temp_dir().join(format!(
            "rnx-reg-{tag}-{n}-{}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let prev_cache = std::env::var("RNX_CACHE_HOME").ok();
        unsafe {
            std::env::set_var("RNX_CACHE_HOME", &dir);
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
        (EnvGuard { lock: Some(lock), prev_cache, prev_proxies }, dir)
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

    fn read_request(stream: &mut std::net::TcpStream) -> Option<(String, String, Vec<u8>)> {
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
        for line in lines {
            if let Some(rest) = line.strip_prefix("Content-Length:") {
                length = rest.trim().parse::<usize>().unwrap_or(0);
            }
            if let Some(rest) = line.strip_prefix("content-length:") {
                length = rest.trim().parse::<usize>().unwrap_or(0);
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
        Some((method, path, body))
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
                    let Some((method, path, body)) = read_request(&mut stream) else {
                        continue;
                    };
                    seen.lock()
                        .unwrap_or_else(|e| e.into_inner())
                        .push(LoggedRequest { method: method.clone(), path: path.clone(), body });
                    let spec = config.version_spec.to_string();
                    let with_spec: &[(&str, &str)] = &[("rnx-registry-spec", Box::leak(spec.into_boxed_str()) as &str)];
                    if method == "GET" && path == "/api/version" {
                        let body = format!(
                            "{{\"spec\": {}, \"capabilities\": [\"tombstones\"], \"registry\": \"test\"}}",
                            config.version_spec
                        );
                        respond(&mut stream, 200, "application/json", body.as_bytes(), with_spec);
                    } else if method == "POST" && path == "/api/resolve" {
                        respond(
                            &mut stream,
                            config.resolve_status,
                            "application/json",
                            config.resolve_body.as_bytes(),
                            with_spec,
                        );
                    } else if method == "GET" && path.ends_with("/chunks") {
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
                    } else if method == "GET" && path.contains("/chunk/") {
                        let hash = path.rsplit('/').next().unwrap_or_default().to_string();
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
                .filter(|(m, p, _)| m == "GET" && p.ends_with(suffix))
                .count()
        }

        pub fn resolve_bodies(&self) -> Vec<JsonValue> {
            self.requests()
                .iter()
                .filter(|(m, p, _)| m == "POST" && p == "/api/resolve")
                .filter_map(|(_, _, b)| parse_json(b).ok())
                .collect()
        }
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
        format!(
            "{{\"resolved\": {{{resolved}}}, \"base\": \"\", \"levels\": [[{}]]}}",
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
            resolve_status: 200,
            resolve_body: resolve_json_static(nodes),
            manifest_status,
            fallback_tarball: tarball,
            manifest_error: manifest_error.to_string(),
            manifest_override: None,
            chunk_override: None,
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
    fn rebuild_tar_matches_server_byte_layout() {
        let entries = vec![ChunkEntry {
            name: "a.txt".to_string(),
            size: 5,
            dir: false,
            chunks: vec!["H".repeat(64)],
        }];
        let mut by_hash = BTreeMap::new();
        by_hash.insert("H".repeat(64), b"hello".to_vec());
        let out = rebuild_tar(&entries, &by_hash).unwrap();
        assert_eq!(out.len(), 2048);
        let hex: String = out[..64].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex,
            "612e7478740000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"
        );
        let hex2: String = out[256..320].iter().map(|b| format!("{b:02x}")).collect();
        assert_eq!(
            hex2,
            "007573746172003030726e780000000000000000000000000000000000000000000000000000000000726e780000000000000000000000000000000000000000"
        );
        assert!(out[1024..].iter().all(|b| *b == 0));
        assert_eq!(&out[512..517], b"hello");
    }

    #[test]
    fn resolve_retry_backoff_is_capped() {
        assert_eq!(resolve_retry_delay_ms(0), 150);
        assert_eq!(resolve_retry_delay_ms(1), 300);
        assert_eq!(resolve_retry_delay_ms(2), 600);
        assert_eq!(resolve_retry_delay_ms(9), 2400);
        assert_eq!(resolve_retry_delay_ms(u32::MAX), 2400);
    }

    #[test]
    fn resolve_503_retries_before_failing() {
        let (_guard, _dir) = isolate_cache("retry503");
        let server = MockRegistry::start(MockConfig {
            version_spec: 1,
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

    #[test]
    fn pinned_cache_hit_skips_network() {
        let (_guard, _dir) = isolate_cache("pinhit");
        let (gz, sha) = fixture_tarball("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &sha, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 1,
            resolve_status: 200,
            resolve_body: resolve_json_static(&[node]),
            manifest_status: 200,
            fallback_tarball: gz,
            manifest_error: String::new(),
            manifest_override: None,
            chunk_override: None,
        });
        let default = registry_cfg(&server.base);
        let empty = BTreeMap::new();
        let (dir, _, ver) =
            resolve_registry_package("@acme/widget", "^1.0.0", Some(&default), &empty, None, &[]).unwrap();
        assert_eq!(ver, "1.2.0");
        assert!(dir.is_dir());
        let served = server.requests().len();
        assert!(served > 0);
        let (dir2, _, ver2) =
            resolve_registry_package("@acme/widget", "^1.0.0", Some(&default), &empty, Some("1.2.0".to_string()), &[]).unwrap();
        assert_eq!(ver2, "1.2.0");
        assert_eq!(dir2, dir);
        assert_eq!(server.requests().len(), served);
    }

    #[test]
    fn api_base_prefers_omitted_and_relative() {
        assert_eq!(
            api_base_for("", "https://r.example.com"),
            "https://r.example.com/api/packages"
        );
        assert_eq!(
            api_base_for("   ", "https://r.example.com/"),
            "https://r.example.com/api/packages"
        );
        assert_eq!(
            api_base_for("/api/v1/custom", "https://r.example.com/api/packages"),
            "https://r.example.com/api/v1/custom"
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
            ("Project.config", b"export default {}\n".to_vec()),
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
        let node = node_json("@acme/widget", "1.2.0", &tar_sha, false);
        let server = MockRegistry::start(MockConfig {
            version_spec: 1,
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

    #[test]
    fn parallel_fetch_reconstructs_many_chunks() {
        let (_guard, _dir) = isolate_cache("parachunks");
        let mut files: Vec<(String, Vec<u8>)> = vec![(
            "Project.config".to_string(),
            b"export default {}\n".to_vec(),
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
        let tar = rebuild_tar(&entries, &by_hash).unwrap();
        let sha = Sha256::hexdigest(&tar);
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
            version_spec: 1,
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
            version_spec: 1,
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
        assert!(err.message.contains("supports 1"), "{}", err.message);
    }

    #[test]
    fn resolve_then_fetch_roundtrip() {
        let (_guard, _dir) = isolate_cache("roundtrip");
        let (gz, sha) = fixture_tarball("@acme/widget", "1.2.0");
        let server = start_ok(1, &[node_json("@acme/widget", "1.2.0", &sha, false)], gz, 200, "");
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
        assert_eq!(have[0].integrity, sha);
        let bodies = server.resolve_bodies();
        assert_eq!(bodies.len(), 1);
        assert_eq!(requirement(&bodies[0], "@acme/widget"), "^1.0.0");
    }

    #[test]
    fn have_skips_redownload() {
        let (_guard, _dir) = isolate_cache("delta");
        let (gz, sha) = fixture_tarball("@acme/widget", "1.2.0");
        let node = node_json("@acme/widget", "1.2.0", &sha, false);
        let server = start_ok(1, &[node], gz, 200, "");
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
            ("@acme/widget".to_string(), "1.2.0".to_string(), sha)
        );
    }

    #[test]
    fn manifest_404_maps_to_does_not_exist() {
        let (_guard, _dir) = isolate_cache("notfound");
        let server = start_ok(
            1,
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
            1,
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
            1,
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
            1,
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
        assert!(err.message.contains("needs engine"), "{}", err.message);
        assert_eq!(server.count_get("chunks"), 0);
    }
}
