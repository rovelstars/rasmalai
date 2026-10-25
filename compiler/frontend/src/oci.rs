use crate::checksum::Sha256;
use crate::fetch::{parse_json, JsonValue};
use diagnostics::{Code, Diagnostic};
use std::collections::{BTreeMap, BTreeSet};
use std::path::{Path, PathBuf};

pub const OCI_INDEX_CAP: usize = 1024 * 1024;
pub const OCI_MANIFEST_CAP: usize = 1024 * 1024;
pub const OCI_BLOB_CAP: usize = 256 * 1024 * 1024;
const OCI_FETCH_WIDTH: usize = 8;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciDescriptor {
    pub digest: String,
    pub role: String,
    pub title: String,
    pub kind: String,
    pub target: String,
    pub generator: String,
    pub upstream: String,
    pub os: Option<String>,
    pub architecture: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OciIndex {
    pub bare: String,
    pub tag: String,
    pub digest: String,
    pub descriptors: Vec<OciDescriptor>,
}

pub fn is_artifact_digest(value: &str) -> bool {
    value.len() == 71
        && value.starts_with("sha256:")
        && value.as_bytes()[7..]
            .iter()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(b))
}

pub fn oci_scheme(registry: &str) -> &'static str {
    let host = registry.split(':').next().unwrap_or(registry);
    if host == "localhost" || host == "127.0.0.1" || host == "::1" {
        "http"
    } else {
        "https"
    }
}

pub fn split_bare(bare: &str) -> Result<(String, String), Diagnostic> {
    crate::project::validate_artifact_ref(bare).map_err(|e| Diagnostic::new(Code::E108, e))?;
    let slash = bare.find('/').unwrap_or(bare.len());
    Ok((bare[..slash].to_string(), bare[slash + 1..].to_string()))
}

fn manifest_url(bare: &str, reference: &str) -> Result<(String, String), Diagnostic> {
    let (registry, repo) = split_bare(bare)?;
    let scheme = oci_scheme(&registry);
    Ok((
        format!("{scheme}://{registry}/v2/{repo}/manifests/{reference}"),
        repo,
    ))
}

fn blob_url(bare: &str, digest: &str) -> Result<String, Diagnostic> {
    let (registry, repo) = split_bare(bare)?;
    let scheme = oci_scheme(&registry);
    Ok(format!("{scheme}://{registry}/v2/{repo}/blobs/{digest}"))
}

#[cfg(not(target_arch = "wasm32"))]
fn http_get(url: &str, accept: &str, cap: usize, what: &str) -> Result<Vec<u8>, Diagnostic> {
    let client = reqwest::blocking::Client::builder()
        .timeout(std::time::Duration::from_secs(30))
        .user_agent("rnx")
        .build()
        .map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot start the artifact client: {e}"))
        })?;
    let response = client
        .get(url)
        .header("Accept", accept)
        .send()
        .map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("artifact origin unreachable ({what}: {e})"),
            )
        })?;
    let status = response.status().as_u16();
    if !(200..300).contains(&status) {
        return Err(Diagnostic::new(
            Code::E108,
            format!("artifact origin unreachable ({what} answered HTTP {status})"),
        ));
    }
    use std::io::Read;
    let mut bytes = Vec::new();
    response
        .take((cap + 1) as u64)
        .read_to_end(&mut bytes)
        .map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("artifact origin unreachable ({what} unreadable: {e})"),
            )
        })?;
    if bytes.len() > cap {
        return Err(Diagnostic::new(
            Code::E108,
            format!("artifact {what} exceeds its size cap"),
        ));
    }
    Ok(bytes)
}

#[cfg(target_arch = "wasm32")]
fn http_get(_url: &str, _accept: &str, _cap: usize, what: &str) -> Result<Vec<u8>, Diagnostic> {
    Err(Diagnostic::new(
        Code::E108,
        format!("artifact fetch is not available on this target ({what})"),
    ))
}

pub fn fetch_index(bare: &str) -> Result<(Vec<u8>, String), Diagnostic> {
    let (url, _) = manifest_url(bare, "latest")?;
    let bytes = http_get(
        &url,
        "application/vnd.oci.image.index.v1+json, application/vnd.docker.distribution.manifest.list.v2+json",
        OCI_INDEX_CAP,
        "index",
    )?;
    let digest = format!("sha256:{}", Sha256::hexdigest(&bytes));
    Ok((bytes, digest))
}

fn role_allowed(kind: &str, role: &str) -> bool {
    match kind {
        "binary" => role == "runtime" || role == "wasm",
        "bindings" => role == "bindgen",
        "media" => role == "media",
        _ => false,
    }
}

pub(crate) fn valid_name(value: &str) -> bool {
    !value.is_empty()
        && value
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'_' || b == b'-')
}

pub fn valid_target(value: &str) -> bool {
    let parts: Vec<&str> = value.split('-').collect();
    (parts.len() == 3 || parts.len() == 4)
        && parts
            .iter()
            .all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_'))
}

pub(crate) fn valid_generator(value: &str) -> bool {
    let Some((name, version)) = value.split_once(' ') else {
        return false;
    };
    if !valid_name(name) {
        return false;
    }
    let core = version.split(['-', '+']).next().unwrap_or_default();
    let nums: Vec<&str> = core.split('.').collect();
    nums.len() == 3
        && nums
            .iter()
            .all(|n| !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()))
}

pub(crate) fn valid_upstream(value: &str) -> bool {
    match value.split_once('@') {
        Some((name, digest)) => valid_name(name) && is_artifact_digest(digest),
        None => false,
    }
}

pub fn valid_title(value: &str) -> bool {
    !value.is_empty()
        && value != "."
        && value != ".."
        && !value.contains('/')
        && !value.contains('\\')
}

fn check_descriptor(bare: &str, item: &JsonValue) -> Result<OciDescriptor, Diagnostic> {
    let bad = |msg: String| Diagnostic::new(Code::E108, msg);
    let obj = match item {
        JsonValue::Object(_) => item,
        _ => {
            return Err(bad(format!(
                "E108: artifact descriptor must be an object (ref `{bare}`)"
            )))
        }
    };
    let digest = obj
        .get("digest")
        .and_then(|d| d.as_str())
        .unwrap_or_default();
    if !is_artifact_digest(digest) {
        return Err(bad(format!(
            "E108: artifact descriptor carries a bad digest `{}` (ref `{bare}`; expected `sha256:<64 hex chars>`)",
            digest.chars().take(24).collect::<String>()
        )));
    }
    let ann = match obj.get("annotations") {
        Some(JsonValue::Object(_)) => obj.get("annotations").unwrap(),
        _ => {
            return Err(bad(format!(
                "E108: artifact descriptor {digest} carries no annotations (ref `{bare}`)"
            )));
        }
    };
    let str_field = |key: &str| ann.get(key).and_then(|v| v.as_str()).unwrap_or_default();
    let kind = str_field("run.rasmalai.kind");
    let role = str_field("run.rasmalai.role");
    let target = str_field("run.rasmalai.target");
    let generator = str_field("run.rasmalai.generator");
    let upstream = str_field("run.rasmalai.upstream");
    let title = str_field("org.opencontainers.image.title");
    if kind != "binary" && kind != "bindings" && kind != "media" {
        return Err(bad(format!(
            "E108: artifact descriptor {digest} carries a bad run.rasmalai.kind `{}` (ref `{bare}`; expected `binary`, `bindings` or `media`)",
            if kind.is_empty() { "missing" } else { kind }
        )));
    }
    if !role_allowed(kind, role) {
        let admitted = match kind {
            "binary" => "runtime, wasm",
            "bindings" => "bindgen",
            _ => "media",
        };
        return Err(bad(format!(
            "E108: artifact descriptor {digest} carries a bad run.rasmalai.role `{}` (ref `{bare}`; kind `{kind}` admits roles {admitted})",
            if role.is_empty() { "missing" } else { role }
        )));
    }
    if !valid_target(target) {
        return Err(bad(format!(
            "E108: artifact descriptor {digest} carries a bad run.rasmalai.target `{}` (ref `{bare}`; expected an arch-vendor-os-env triple)",
            if target.is_empty() { "missing" } else { target }
        )));
    }
    if !valid_generator(generator) {
        return Err(bad(format!(
            "E108: artifact descriptor {digest} carries a bad run.rasmalai.generator `{}` (ref `{bare}`; expected `<name> <semver>`)",
            if generator.is_empty() { "missing" } else { generator }
        )));
    }
    if !valid_upstream(upstream) {
        return Err(bad(format!(
            "E108: artifact descriptor {digest} carries a bad run.rasmalai.upstream `{}` (ref `{bare}`; expected `<name>@sha256:<hex>`)",
            if upstream.is_empty() { "missing" } else { upstream }
        )));
    }
    if !valid_title(title) {
        return Err(bad(format!(
            "E108: artifact descriptor {digest} carries a bad org.opencontainers.image.title `{}` (ref `{bare}`; expected a plain filename)",
            if title.is_empty() { "missing" } else { title }
        )));
    }
    let (os, architecture) = match obj.get("platform") {
        None => (None, None),
        Some(JsonValue::Object(_)) => {
            let plat = obj.get("platform").unwrap();
            let os = plat.get("os").and_then(|v| v.as_str()).unwrap_or_default();
            let architecture = plat
                .get("architecture")
                .and_then(|v| v.as_str())
                .unwrap_or_default();
            if os.is_empty() || architecture.is_empty() {
                return Err(bad(format!(
                    "E108: artifact descriptor {digest} carries a bad platform (ref `{bare}`; expected `os` and `architecture`)"
                )));
            }
            (Some(os.to_string()), Some(architecture.to_string()))
        }
        Some(_) => {
            return Err(bad(format!(
                "E108: artifact descriptor {digest} carries a bad platform (ref `{bare}`; expected `os` and `architecture`)"
            )));
        }
    };
    Ok(OciDescriptor {
        digest: digest.to_string(),
        role: role.to_string(),
        title: title.to_string(),
        kind: kind.to_string(),
        target: target.to_string(),
        generator: generator.to_string(),
        upstream: upstream.to_string(),
        os,
        architecture,
    })
}

pub fn parse_index(
    bare: &str,
    tag: &str,
    bytes: &[u8],
    asserted: &str,
) -> Result<OciIndex, Diagnostic> {
    let served = format!("sha256:{}", Sha256::hexdigest(bytes));
    if served != asserted {
        return Err(Diagnostic::new(
            Code::E108,
            format!("E108: artifact digest mismatch (asserted {asserted}, origin serves {served} for `{bare}`)"),
        ));
    }
    let doc = parse_json(bytes).map_err(|_| {
        Diagnostic::new(
            Code::E108,
            format!("E108: artifact index is not valid JSON (ref `{bare}`)"),
        )
    })?;
    let manifests = match doc.get("manifests").and_then(|m| m.as_array()) {
        Some(items) if !items.is_empty() => items,
        _ => {
            return Err(Diagnostic::new(
                Code::E108,
                format!("E108: artifact index carries no manifests (ref `{bare}`)"),
            ));
        }
    };
    let mut descriptors = Vec::new();
    for item in manifests {
        descriptors.push(check_descriptor(bare, item)?);
    }
    Ok(OciIndex {
        bare: bare.to_string(),
        tag: tag.to_string(),
        digest: served,
        descriptors,
    })
}

fn canonical_os(os: &str) -> &str {
    match os {
        "macos" | "darwin" => "darwin",
        "linux" => "linux",
        "windows" => "windows",
        other => other,
    }
}

fn canonical_arch(arch: &str) -> &str {
    match arch {
        "x86_64" => "amd64",
        "aarch64" => "arm64",
        "wasm32" => "wasm32",
        other => other,
    }
}

pub fn descriptor_matches(os: &str, arch: &str, descriptor: &OciDescriptor) -> bool {
    match (&descriptor.os, &descriptor.architecture) {
        (None, None) => true,
        (Some(o), Some(a)) => canonical_os(os) == o.as_str() && canonical_arch(arch) == a.as_str(),
        _ => false,
    }
}

pub fn select_for_host<'a>(index: &'a OciIndex, os: &str, arch: &str) -> Vec<&'a OciDescriptor> {
    let mut out: Vec<&OciDescriptor> = index
        .descriptors
        .iter()
        .filter(|d| descriptor_matches(os, arch, d))
        .collect();
    out.sort_by(|a, b| a.digest.cmp(&b.digest));
    out
}

fn fetch_manifest(bare: &str, digest: &str) -> Result<JsonValue, Diagnostic> {
    let (url, _) = manifest_url(bare, digest)?;
    let bytes = http_get(
        &url,
        "application/vnd.oci.image.manifest.v1+json",
        OCI_MANIFEST_CAP,
        "manifest",
    )?;
    let served = format!("sha256:{}", Sha256::hexdigest(&bytes));
    if served != digest {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "E108: artifact manifest mismatch (descriptor {digest}, origin serves {served})"
            ),
        ));
    }
    parse_json(&bytes).map_err(|_| {
        Diagnostic::new(
            Code::E108,
            format!("E108: artifact manifest is not valid JSON (digest {digest})"),
        )
    })
}

fn manifest_layer_digests(manifest: &JsonValue) -> Result<Vec<String>, Diagnostic> {
    let layers = match manifest.get("layers").and_then(|l| l.as_array()) {
        Some(layers) if !layers.is_empty() => layers,
        _ => {
            return Err(Diagnostic::new(
                Code::E108,
                "E108: artifact manifest carries no layers".to_string(),
            ));
        }
    };
    let mut out = Vec::new();
    for layer in layers {
        match layer.get("digest").and_then(|d| d.as_str()) {
            Some(digest) if is_artifact_digest(digest) => out.push(digest.to_string()),
            _ => {
                return Err(Diagnostic::new(
                    Code::E108,
                    "E108: artifact manifest layer carries a bad digest".to_string(),
                ));
            }
        }
    }
    Ok(out)
}

pub fn fetch_blob(bare: &str, digest: &str) -> Result<Vec<u8>, Diagnostic> {
    let url = blob_url(bare, digest)?;
    let bytes = http_get(&url, "application/octet-stream", OCI_BLOB_CAP, "blob")?;
    let served = format!("sha256:{}", Sha256::hexdigest(&bytes));
    if served != digest {
        return Err(Diagnostic::new(
            Code::E108,
            format!("E108: artifact blob mismatch (expected {digest}, origin serves {served})"),
        ));
    }
    Ok(bytes)
}

#[derive(Debug, Clone)]
pub struct AssembleInput {
    pub role: String,
    pub title: String,
    pub kind: String,
    pub target: String,
    pub platform: Option<(String, String)>,
    pub generator: String,
    pub upstream: String,
    pub bytes: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct Assembly {
    pub index_bytes: Vec<u8>,
    pub digest: String,
    pub manifests: Vec<(String, Vec<u8>)>,
    pub blobs: Vec<(String, Vec<u8>)>,
}

pub fn platform_for_triple(triple: &str) -> Option<(String, String)> {
    let parts: Vec<&str> = triple.split('-').collect();
    if parts.len() < 3 || parts.len() > 4 || parts.iter().any(|p| p.is_empty()) {
        return None;
    }
    let arch = match parts[0] {
        "x86_64" => "amd64".to_string(),
        "aarch64" => "arm64".to_string(),
        "wasm32" => "wasm32".to_string(),
        other if valid_name(other) => other.to_string(),
        _ => return None,
    };
    let os = match parts[2] {
        "macos" | "darwin" => "darwin".to_string(),
        "linux" => "linux".to_string(),
        "windows" => "windows".to_string(),
        other if valid_name(other) => other.to_string(),
        _ => return None,
    };
    Some((os, arch))
}

fn json_escape(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            _ => out.push(c),
        }
    }
    out
}

pub fn assemble_index(inputs: &[AssembleInput]) -> Result<Assembly, Diagnostic> {
    if inputs.is_empty() {
        return Err(Diagnostic::new(
            Code::E108,
            "cannot assemble an artifact image with no files (stage per-target blobs under `.rnx-cache/publish/` or declare `bindgen.outputs` first)",
        ));
    }
    let bad = |msg: String| Diagnostic::new(Code::E108, msg);
    let mut layers: Vec<(String, AssembleInput)> = Vec::with_capacity(inputs.len());
    for input in inputs {
        if !role_allowed(&input.kind, &input.role) {
            return Err(bad(format!(
                "cannot assemble `{}`: kind `{}` admits roles {}, not `{}`",
                input.title,
                input.kind,
                match input.kind.as_str() {
                    "binary" => "runtime, wasm",
                    "bindings" => "bindgen",
                    _ => "media",
                },
                input.role,
            )));
        }
        if !valid_target(&input.target) {
            return Err(bad(format!(
                "cannot assemble `{}`: bad target `{}` (expected an arch-vendor-os-env triple)",
                input.title, input.target,
            )));
        }
        if !valid_generator(&input.generator) {
            return Err(bad(format!(
                "cannot assemble `{}`: bad generator `{}` (expected `<name> <semver>`)",
                input.title, input.generator,
            )));
        }
        if !valid_upstream(&input.upstream) {
            return Err(bad(format!(
                "cannot assemble `{}`: bad upstream `{}` (expected `<name>@sha256:<hex>`)",
                input.title, input.upstream,
            )));
        }
        if !valid_title(&input.title) {
            return Err(bad(format!(
                "cannot assemble `{}`: bad title (expected a plain filename)",
                input.title,
            )));
        }
        if let Some((os, arch)) = &input.platform
            && (os.is_empty() || arch.is_empty())
        {
            return Err(bad(format!(
                "cannot assemble `{}`: bad platform (expected `os` and `architecture`)",
                input.title,
            )));
        }
        let hex = Sha256::hexdigest(&input.bytes);
        layers.push((format!("sha256:{hex}"), input.clone()));
    }
    layers.sort_by(|a, b| a.0.cmp(&b.0));
    let config_bytes = b"{}".to_vec();
    let config_digest = format!("sha256:{}", Sha256::hexdigest(&config_bytes));
    let mut blob_map: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    blob_map.insert(config_digest.clone(), config_bytes);
    let mut descriptors: Vec<String> = Vec::with_capacity(layers.len());
    let mut manifests: Vec<(String, Vec<u8>)> = Vec::with_capacity(layers.len());
    for (blob_digest, input) in &layers {
        blob_map.insert(blob_digest.clone(), input.bytes.clone());
        let manifest = format!(
            "{{\"schemaVersion\": 2, \"mediaType\": \"application/vnd.oci.image.manifest.v1+json\", \"config\": {{\"mediaType\": \"application/vnd.oci.image.config.v1+json\", \"digest\": \"{config_digest}\", \"size\": 2}}, \"layers\": [{{\"mediaType\": \"application/vnd.oci.image.layer.v1.tar\", \"digest\": \"{blob_digest}\", \"size\": {}}}]}}",
            input.bytes.len()
        );
        let manifest_digest = format!("sha256:{}", Sha256::hexdigest(manifest.as_bytes()));
        let platform = match &input.platform {
            Some((os, arch)) => format!(
                ", \"platform\": {{\"os\": \"{}\", \"architecture\": \"{}\"}}",
                json_escape(os),
                json_escape(arch)
            ),
            None => String::new(),
        };
        let mut ann: BTreeMap<&str, &str> = BTreeMap::new();
        ann.insert("org.opencontainers.image.title", &input.title);
        ann.insert("run.rasmalai.generator", &input.generator);
        ann.insert("run.rasmalai.kind", &input.kind);
        ann.insert("run.rasmalai.role", &input.role);
        ann.insert("run.rasmalai.target", &input.target);
        ann.insert("run.rasmalai.upstream", &input.upstream);
        let ann_json: Vec<String> = ann
            .iter()
            .map(|(k, v)| format!("\"{}\": \"{}\"", json_escape(k), json_escape(v)))
            .collect();
        descriptors.push(format!(
            "{{\"mediaType\": \"application/vnd.oci.image.manifest.v1+json\", \"digest\": \"{manifest_digest}\", \"size\": {}{}, \"annotations\": {{{}}}}}",
            manifest.as_bytes().len(),
            platform,
            ann_json.join(", ")
        ));
        manifests.push((manifest_digest, manifest.into_bytes()));
    }
    descriptors.sort();
    let index = format!(
        "{{\"schemaVersion\": 2, \"manifests\": [{}]}}",
        descriptors.join(", ")
    );
    let index_bytes = index.into_bytes();
    let digest = format!("sha256:{}", Sha256::hexdigest(&index_bytes));
    let blobs: Vec<(String, Vec<u8>)> = blob_map.into_iter().collect();
    Ok(Assembly { index_bytes, digest, manifests, blobs })
}

pub fn assembly_blob_hexes(assembly: &Assembly) -> BTreeSet<String> {
    let mut hexes = BTreeSet::new();
    for (digest, _) in assembly.blobs.iter().chain(assembly.manifests.iter()) {
        if let Some(hex) = digest.strip_prefix("sha256:") {
            hexes.insert(hex.to_string());
        }
    }
    hexes
}

fn artifact_file_path(
    pkg_dir: &Path,
    role: &str,
    title: &str,
    index: usize,
    layers: usize,
) -> Option<PathBuf> {
    linked_artifact_path(pkg_dir, role, title, index, layers)
}

pub fn project_artifact_cache(pkg_dir: &Path) -> PathBuf {
    pkg_dir.join(".rnx-cache").join("artifacts")
}

pub fn project_blob_path(pkg_dir: &Path, hex: &str) -> PathBuf {
    project_artifact_cache(pkg_dir).join("blobs").join(hex)
}

pub fn global_blob_path(hex: &str) -> PathBuf {
    crate::cache::global_cache_dir().join("artifacts").join(hex)
}

pub fn linked_artifact_path(
    pkg_dir: &Path,
    role: &str,
    title: &str,
    index: usize,
    layers: usize,
) -> Option<PathBuf> {
    if role.contains('/') || role.contains('\\') || role.is_empty() {
        return None;
    }
    let file = if layers == 1 {
        title.to_string()
    } else {
        format!("{title}.{index}")
    };
    let path = project_artifact_cache(pkg_dir).join(role).join(&file);
    if path
        .components()
        .any(|c| matches!(c, std::path::Component::ParentDir))
    {
        return None;
    }
    Some(path)
}

fn place_blob(pkg_dir: &Path, hex: &str, bytes: &[u8]) {
    for path in [project_blob_path(pkg_dir, hex), global_blob_path(hex)] {
        if path.is_file() {
            continue;
        }
        if let Some(parent) = path.parent()
            && std::fs::create_dir_all(parent).is_ok()
        {
            let _ = std::fs::write(&path, bytes);
        }
    }
}

fn cached_blob(pkg_dir: &Path, digest: &str) -> Option<Vec<u8>> {
    let hex = digest.strip_prefix("sha256:")?;
    let project = project_blob_path(pkg_dir, hex);
    if let Ok(bytes) = std::fs::read(&project) {
        eprintln!("notice: using cached artifact blob {digest} (fetch skipped)");
        return Some(bytes);
    }
    let global = global_blob_path(hex);
    if let Ok(bytes) = std::fs::read(&global) {
        eprintln!("notice: using cached artifact blob {digest} (fetch skipped)");
        if let Some(parent) = project.parent()
            && std::fs::create_dir_all(parent).is_ok()
        {
            let _ = std::fs::write(&project, &bytes);
        }
        return Some(bytes);
    }
    None
}

pub fn ensure_artifacts(
    pkg_dir: &Path,
    bare: &str,
    asserted: &str,
    os: &str,
    arch: &str,
) -> Result<usize, Diagnostic> {
    let (bytes, _) = fetch_index(bare)?;
    let index = parse_index(bare, "latest", &bytes, asserted)?;
    let selected = select_for_host(&index, os, arch);
    let manifests: Vec<(OciDescriptor, Vec<String>)> = selected
        .into_iter()
        .map(|d| {
            let manifest = fetch_manifest(bare, &d.digest)?;
            let layers = manifest_layer_digests(&manifest)?;
            Ok((d.clone(), layers))
        })
        .collect::<Result<Vec<_>, Diagnostic>>()?;
    let mut jobs: Vec<(OciDescriptor, String, usize, usize)> = Vec::new();
    for (descriptor, layers) in &manifests {
        for (i, digest) in layers.iter().enumerate() {
            jobs.push((descriptor.clone(), digest.clone(), i, layers.len()));
        }
    }
    let mut blobs: BTreeMap<String, Vec<u8>> = BTreeMap::new();
    let mut missing: Vec<usize> = Vec::new();
    for (i, (_, digest, _, _)) in jobs.iter().enumerate() {
        match cached_blob(pkg_dir, digest) {
            Some(bytes) => {
                blobs.insert(digest.clone(), bytes);
            }
            None => missing.push(i),
        }
    }
    let width = OCI_FETCH_WIDTH.min(missing.len().max(1));
    let next = std::sync::atomic::AtomicUsize::new(0);
    let slots: Vec<std::sync::Mutex<Option<Result<Vec<u8>, Diagnostic>>>> = (0..missing.len())
        .map(|_| std::sync::Mutex::new(None))
        .collect();
    std::thread::scope(|s| {
        for _ in 0..width {
            s.spawn(|| loop {
                let k = next.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
                let Some(mi) = missing.get(k) else {
                    break;
                };
                let (_, digest, _, _) = &jobs[*mi];
                let got = fetch_blob(bare, digest);
                let mut slot = slots[k].lock().unwrap_or_else(|e| e.into_inner());
                *slot = Some(got);
            });
        }
    });
    for (k, mi) in missing.iter().enumerate() {
        let (_, digest, _, _) = &jobs[*mi];
        let mut slot = slots[k].lock().unwrap_or_else(|e| e.into_inner());
        match slot.take() {
            Some(Ok(bytes)) => {
                blobs.insert(digest.clone(), bytes);
            }
            Some(Err(e)) => return Err(e),
            None => {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("artifact blob {digest} was not fetched"),
                ));
            }
        }
    }
    for (digest, bytes) in &blobs {
        if let Some(hex) = digest.strip_prefix("sha256:")
            && !project_blob_path(pkg_dir, hex).is_file()
        {
            place_blob(pkg_dir, hex, bytes);
        }
    }
    let mut written = 0;
    for (descriptor, digest, i, total) in &jobs {
        let Some(bytes) = blobs.get(digest) else {
            return Err(Diagnostic::new(
                Code::E108,
                format!("artifact blob {digest} was not fetched"),
            ));
        };
        let Some(path) =
            artifact_file_path(pkg_dir, &descriptor.role, &descriptor.title, *i, *total)
        else {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "artifact descriptor {} names an unsafe path",
                    descriptor.digest
                ),
            ));
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent).map_err(|e| {
                Diagnostic::new(
                    Code::E108,
                    format!("cannot create `{}`: {e}", parent.display()),
                )
            })?;
        }
        let blob = digest.strip_prefix("sha256:").map(|hex| project_blob_path(pkg_dir, hex));
        let mut linked = false;
        if let Some(source) = blob.as_ref().filter(|b| b.is_file()) {
            let _ = std::fs::remove_file(&path);
            linked = std::fs::hard_link(source, &path).is_ok();
        }
        if !linked {
            std::fs::write(&path, bytes).map_err(|e| {
                Diagnostic::new(
                    Code::E108,
                    format!("cannot write `{}`: {e}", path.display()),
                )
            })?;
        }
        written += 1;
    }
    Ok(written)
}

#[cfg(test)]
pub(crate) mod testkit {
    use super::*;
    use std::io::{Read, Write};
    use std::net::TcpListener;
    use std::sync::{Arc, Mutex};

    pub struct OciOrigin {
        pub bare: String,
        pub log: Arc<Mutex<Vec<String>>>,
    }

    pub fn start_origin(index: Vec<u8>, files: BTreeMap<String, Vec<u8>>) -> OciOrigin {
        let listener = TcpListener::bind("127.0.0.1:0").unwrap();
        let addr = listener.local_addr().unwrap();
        let bare = format!("{addr}/org/sqlite3");
        let files = Arc::new(files);
        let index = Arc::new(index);
        let log: Arc<Mutex<Vec<String>>> = Arc::new(Mutex::new(Vec::new()));
        let seen = log.clone();
        std::thread::spawn(move || {
            for stream in listener.incoming() {
                let Ok(mut stream) = stream else {
                    continue;
                };
                let mut head = Vec::new();
                let mut byte = [0u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    match stream.read(&mut byte) {
                        Ok(0) => break,
                        Ok(_) => head.push(byte[0]),
                        Err(_) => break,
                    }
                    if head.len() > 65536 {
                        break;
                    }
                }
                let text = String::from_utf8_lossy(&head).into_owned();
                let path = text
                    .lines()
                    .next()
                    .unwrap_or_default()
                    .split_whitespace()
                    .nth(1)
                    .unwrap_or_default()
                    .to_string();
                seen.lock()
                    .unwrap_or_else(|e| e.into_inner())
                    .push(path.clone());
                let body: Option<Vec<u8>> = if path.ends_with("/manifests/latest") {
                    Some(index.to_vec())
                } else if path.contains("/manifests/") {
                    let digest = path.rsplit('/').next().unwrap_or_default().to_string();
                    files.get(&digest).cloned()
                } else if path.contains("/blobs/") {
                    let digest = path.rsplit('/').next().unwrap_or_default().to_string();
                    files.get(&digest).cloned()
                } else {
                    None
                };
                match body {
                    Some(bytes) => {
                        let head = format!(
                            "HTTP/1.1 200 OK\r\nContent-Type: application/octet-stream\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                            bytes.len()
                        );
                        let _ = stream.write_all(head.as_bytes());
                        let _ = stream.write_all(&bytes);
                    }
                    None => {
                        let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                    }
                }
                let _ = stream.flush();
            }
        });
        OciOrigin { bare, log }
    }
}

#[cfg(test)]
mod tests {
    use super::testkit::start_origin;
    use super::*;
    use std::collections::BTreeMap;

    const BLOB: &[u8] = b"fake native bytes";

    fn annotations(role: &str, kind: &str, title: &str) -> String {
        format!(
            "\"run.rasmalai.kind\": \"{kind}\", \"run.rasmalai.role\": \"{role}\", \
             \"run.rasmalai.target\": \"x86_64-unknown-linux-gnu\", \
             \"run.rasmalai.generator\": \"rnx-bindgen 0.1.0\", \
             \"run.rasmalai.upstream\": \"sqlite@sha256:{}\", \
             \"org.opencontainers.image.title\": \"{title}\"",
            "cd".repeat(32)
        )
    }

    fn descriptor_json(
        digest: &str,
        role: &str,
        kind: &str,
        title: &str,
        platform: Option<(&str, &str)>,
    ) -> String {
        let ann = annotations(role, kind, title);
        if let Some((os, arch)) = platform {
            format!(
                "{{\"digest\": \"{digest}\", \"platform\": {{\"os\": \"{os}\", \"architecture\": \"{arch}\"}}, \"annotations\": {{{ann}}}}}"
            )
        } else {
            format!("{{\"digest\": \"{digest}\", \"annotations\": {{{ann}}}}}")
        }
    }

    fn manifest_hex() -> String {
        "aa".repeat(32)
    }

    fn valid_index(manifest_digests: &[String]) -> Vec<u8> {
        let items: Vec<String> = manifest_digests
            .iter()
            .map(|d| {
                descriptor_json(
                    &format!("sha256:{d}"),
                    "runtime",
                    "binary",
                    "libfoo.so",
                    None,
                )
            })
            .collect();
        format!(
            "{{\"schemaVersion\": 2, \"manifests\": [{}]}}",
            items.join(", ")
        )
        .into_bytes()
    }

    #[test]
    fn digest_spelling_is_lowercase_hex_only() {
        assert!(is_artifact_digest(&format!("sha256:{}", "ab".repeat(32))));
        assert!(!is_artifact_digest(&format!("sha256:{}", "AB".repeat(32))));
        assert!(!is_artifact_digest("sha256:xyz"));
        assert!(!is_artifact_digest(""));
    }

    #[test]
    fn scheme_is_plain_http_for_loopback_only() {
        assert_eq!(oci_scheme("127.0.0.1:8080"), "http");
        assert_eq!(oci_scheme("localhost"), "http");
        assert_eq!(oci_scheme("ghcr.io"), "https");
    }

    #[test]
    fn parse_rejects_mismatched_digest() {
        let index = valid_index(&[manifest_hex()]);
        let err = parse_index(
            "example.invalid/org/x",
            "latest",
            &index,
            &format!("sha256:{}", "bb".repeat(32)),
        )
        .unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("digest mismatch"), "{}", err.message);
    }

    #[test]
    fn parse_rejects_role_kind_mismatch_and_bad_grammars() {
        let cases = vec![
            descriptor_json(
                &format!("sha256:{}", manifest_hex()),
                "bindgen",
                "binary",
                "libfoo.so",
                None,
            ),
            descriptor_json(
                &format!("sha256:{}", manifest_hex()),
                "runtime",
                "media",
                "libfoo.so",
                None,
            ),
            descriptor_json("not-a-digest", "runtime", "binary", "libfoo.so", None),
            descriptor_json(
                &format!("sha256:{}", manifest_hex()),
                "runtime",
                "binary",
                "../escape.so",
                None,
            ),
        ];
        for descriptor in cases {
            let index =
                format!("{{\"schemaVersion\": 2, \"manifests\": [{descriptor}]}}").into_bytes();
            let digest = format!("sha256:{}", Sha256::hexdigest(&index));
            let err = parse_index("example.invalid/org/x", "latest", &index, &digest).unwrap_err();
            assert_eq!(err.code, diagnostics::Code::E108, "{descriptor}");
            assert!(err.message.contains("E108"), "{}", err.message);
        }
    }

    #[test]
    fn platform_selection_prefers_universal_and_exact_match() {
        let linux_hex = "aa".repeat(32);
        let mac_hex = "bb".repeat(32);
        let items = format!(
            "{}, {}",
            descriptor_json(
                &format!("sha256:{linux_hex}"),
                "runtime",
                "binary",
                "linux.so",
                Some(("linux", "amd64"))
            ),
            descriptor_json(
                &format!("sha256:{mac_hex}"),
                "runtime",
                "binary",
                "mac.so",
                Some(("darwin", "arm64"))
            )
        );
        let index_bytes =
            format!("{{\"schemaVersion\": 2, \"manifests\": [{items}]}}").into_bytes();
        let digest = format!("sha256:{}", Sha256::hexdigest(&index_bytes));
        let index = parse_index("example.invalid/org/x", "latest", &index_bytes, &digest).unwrap();
        let linux: Vec<String> = select_for_host(&index, "linux", "x86_64")
            .iter()
            .map(|d| d.digest.clone())
            .collect();
        assert_eq!(linux, vec![format!("sha256:{linux_hex}")]);
        let mac: Vec<String> = select_for_host(&index, "macos", "aarch64")
            .iter()
            .map(|d| d.digest.clone())
            .collect();
        assert_eq!(mac, vec![format!("sha256:{mac_hex}")]);
        let universal_bytes = valid_index(&[manifest_hex()]);
        let universal_digest = format!("sha256:{}", Sha256::hexdigest(&universal_bytes));
        let universal = parse_index(
            "example.invalid/org/x",
            "latest",
            &universal_bytes,
            &universal_digest,
        )
        .unwrap();
        assert_eq!(select_for_host(&universal, "windows", "x86_64").len(), 1);
    }

    #[test]
    fn ensure_installs_blob_by_title_under_role_dir() {
        // Bytes are unique per run: the global blob cache is shared (and
        // restored across CI jobs), so fixed bytes could hit a warm cache
        // and skip the fetch this test asserts on.
        let blob: Vec<u8> = format!(
            "fake native bytes install {}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        )
        .into_bytes();
        let blob_hash = Sha256::hexdigest(&blob);
        let manifest = format!(
            "{{\"schemaVersion\": 2, \"config\": {{\"digest\": \"sha256:{}\"}}, \"layers\": [{{\"digest\": \"sha256:{blob_hash}\"}}]}}",
            "cc".repeat(32)
        );
        let manifest_hash = Sha256::hexdigest(manifest.as_bytes());
        let index = valid_index(&[manifest_hash.clone()]);
        let digest = format!("sha256:{}", Sha256::hexdigest(&index));
        let mut files = BTreeMap::new();
        files.insert(format!("sha256:{manifest_hash}"), manifest.into_bytes());
        files.insert(format!("sha256:{blob_hash}"), blob.clone());
        let origin = start_origin(index, files);
        let dir = crate::scratch::dir("oci-install");
        let written = ensure_artifacts(&dir, &origin.bare, &digest, "linux", "x86_64").unwrap();
        assert_eq!(written, 1);
        let installed =
            std::fs::read(dir.join(".rnx-cache").join("artifacts").join("runtime").join("libfoo.so")).unwrap();
        assert_eq!(installed, blob);
        let gets: Vec<String> = origin.log.lock().unwrap().clone();
        assert!(
            gets.iter().any(|p| p.ends_with("/manifests/latest")),
            "{gets:?}"
        );
        assert!(
            gets.iter().any(|p| p.contains("/blobs/sha256:")),
            "{gets:?}"
        );
    }

    #[test]
    fn ensure_rejects_corrupt_blob_bytes() {
        let manifest = format!(
            "{{\"schemaVersion\": 2, \"config\": {{\"digest\": \"sha256:{}\"}}, \"layers\": [{{\"digest\": \"sha256:{}\"}}]}}",
            "cc".repeat(32),
            "dd".repeat(32)
        );
        let manifest_hash = Sha256::hexdigest(manifest.as_bytes());
        let index = valid_index(&[manifest_hash.clone()]);
        let digest = format!("sha256:{}", Sha256::hexdigest(&index));
        let mut files = BTreeMap::new();
        files.insert(format!("sha256:{manifest_hash}"), manifest.into_bytes());
        files.insert(format!("sha256:{}", "dd".repeat(32)), b"tampered".to_vec());
        let origin = start_origin(index, files);
        let dir = crate::scratch::dir("oci-corrupt");
        let err = ensure_artifacts(&dir, &origin.bare, &digest, "linux", "x86_64").unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("blob mismatch"), "{}", err.message);
    }

    #[test]
    fn ensure_skips_fetch_for_cached_blobs() {
        let blob_hash = Sha256::hexdigest(BLOB);
        let manifest = format!(
            "{{\"schemaVersion\": 2, \"config\": {{\"digest\": \"sha256:{}\"}}, \"layers\": [{{\"digest\": \"sha256:{blob_hash}\"}}]}}",
            "cc".repeat(32)
        );
        let manifest_hash = Sha256::hexdigest(manifest.as_bytes());
        let index = valid_index(&[manifest_hash.clone()]);
        let digest = format!("sha256:{}", Sha256::hexdigest(&index));
        let mut files = BTreeMap::new();
        files.insert(format!("sha256:{manifest_hash}"), manifest.into_bytes());
        files.insert(format!("sha256:{blob_hash}"), BLOB.to_vec());
        let origin = start_origin(index, files);
        let dir = crate::scratch::dir("oci-shadow");
        let blob_path = project_blob_path(&dir, &blob_hash);
        std::fs::create_dir_all(blob_path.parent().unwrap()).unwrap();
        std::fs::write(&blob_path, BLOB).unwrap();
        let written = ensure_artifacts(&dir, &origin.bare, &digest, "linux", "x86_64").unwrap();
        assert_eq!(written, 1);
        let installed =
            std::fs::read(dir.join(".rnx-cache").join("artifacts").join("runtime").join("libfoo.so")).unwrap();
        assert_eq!(installed, BLOB);
        let gets: Vec<String> = origin.log.lock().unwrap().clone();
        assert!(
            gets.iter().any(|p| p.ends_with("/manifests/latest")),
            "{gets:?}"
        );
        assert!(
            !gets.iter().any(|p| p.contains("/blobs/sha256:")),
            "{gets:?}"
        );
    }

    fn assemble_input(role: &str, title: &str, kind: &str, target: &str) -> AssembleInput {
        AssembleInput {
            role: role.to_string(),
            title: title.to_string(),
            kind: kind.to_string(),
            target: target.to_string(),
            platform: platform_for_triple(target),
            generator: "rnx-bindgen 0.1.0".to_string(),
            upstream: format!("sqlite@sha256:{}", "cd".repeat(32)),
            bytes: format!("contents of {title}").into_bytes(),
        }
    }

    #[test]
    fn assemble_round_trips_through_parse_and_select() {
        let inputs = vec![
            assemble_input("runtime", "libfoo.so", "binary", "x86_64-unknown-linux-gnu"),
            assemble_input("bindgen", "foo.rnx", "bindings", "aarch64-apple-darwin"),
        ];
        let assembly = assemble_index(&inputs).unwrap();
        assert!(is_artifact_digest(&assembly.digest));
        let again = assemble_index(&inputs).unwrap();
        assert_eq!(assembly.index_bytes, again.index_bytes);
        assert_eq!(assembly.digest, again.digest);
        let index = parse_index("example.invalid/org/x", "latest", &assembly.index_bytes, &assembly.digest).unwrap();
        assert_eq!(index.descriptors.len(), 2);
        let linux: Vec<&str> = select_for_host(&index, "linux", "x86_64")
            .iter()
            .map(|d| d.title.as_str())
            .collect();
        assert_eq!(linux, vec!["libfoo.so"]);
        let mac: Vec<&str> = select_for_host(&index, "macos", "aarch64")
            .iter()
            .filter(|d| d.role == "bindgen")
            .map(|d| d.title.as_str())
            .collect();
        assert_eq!(mac, vec!["foo.rnx"]);
        let hexes = assembly_blob_hexes(&assembly);
        for (digest, _) in assembly.blobs.iter().chain(assembly.manifests.iter()) {
            assert!(hexes.contains(digest.strip_prefix("sha256:").unwrap()), "{hexes:?}");
        }
    }

    #[test]
    fn assemble_rejects_empty_and_bad_annotations() {
        let err = assemble_index(&[]).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        let mut bad_role = assemble_input("bindgen", "libfoo.so", "binary", "x86_64-unknown-linux-gnu");
        bad_role.role = "bindgen".to_string();
        let err = assemble_index(&[bad_role]).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        let mut bad_title = assemble_input("runtime", "../evil.so", "binary", "x86_64-unknown-linux-gnu");
        bad_title.title = "../evil.so".to_string();
        let err = assemble_index(&[bad_title]).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
    }

    #[test]
    fn platform_mapping_covers_toolchain_triples() {
        assert_eq!(
            platform_for_triple("x86_64-unknown-linux-gnu"),
            Some(("linux".to_string(), "amd64".to_string()))
        );
        assert_eq!(
            platform_for_triple("aarch64-apple-darwin"),
            Some(("darwin".to_string(), "arm64".to_string()))
        );
        assert_eq!(
            platform_for_triple("x86_64-pc-windows-msvc"),
            Some(("windows".to_string(), "amd64".to_string()))
        );
        assert!(platform_for_triple("nope").is_none());
    }
}
