use crate::target::{Capability, CpuSpec, LinkMap, RuntimeSpec, Target, Tier};
use diagnostics::{Code, Diagnostic};
use serde_json::Value;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::path::{Path, PathBuf};

pub const MANIFEST_FILE: &str = "sysroot.json";
pub const PACK_VERSION: u64 = 2;
pub const INTEGRITY_ALGORITHM: &str = "sha256";
const DIGEST_PREFIX: &str = "sha256:";
const DIGEST_DOMAIN: &[u8] = b"rnx-target-pack\x00v2\x00";

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackError {
    pub dir: PathBuf,
    pub path: Option<String>,
    pub message: String,
}

impl PackError {
    pub fn new(dir: &Path, message: impl Into<String>) -> Self {
        PackError {
            dir: dir.to_path_buf(),
            path: None,
            message: message.into(),
        }
    }

    pub fn at(dir: &Path, path: &str, message: impl Into<String>) -> Self {
        PackError {
            dir: dir.to_path_buf(),
            path: Some(path.to_string()),
            message: message.into(),
        }
    }

    pub fn code(&self) -> Code {
        Code::E124
    }

    pub fn diagnostic(&self) -> Diagnostic {
        Diagnostic::new(Code::E124, self.to_string())
    }

    pub fn with_hint(self, hint: impl Into<String>) -> Diagnostic {
        self.diagnostic().with_hint(hint.into())
    }
}

impl fmt::Display for PackError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut message = format!("target pack `{}`", self.dir.display());
        if let Some(path) = &self.path {
            message.push_str(&format!(": `{path}`"));
        }
        message.push_str(": ");
        message.push_str(&self.message);
        f.write_str(&message)
    }
}

impl std::error::Error for PackError {}

pub fn is_digest(value: &str) -> bool {
    value.len() == 71
        && value
            .strip_prefix(DIGEST_PREFIX)
            .map(|hex| {
                hex.bytes()
                    .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
            })
            .unwrap_or(false)
}

pub fn validate_rel_path(value: &str) -> Result<(), String> {
    if value.is_empty() {
        return Err("path is empty".to_string());
    }
    if value.contains('\\') {
        return Err(format!(
            "path `{value}` uses a backslash; pack paths are POSIX"
        ));
    }
    if value.starts_with('/') {
        return Err(format!("path `{value}` is absolute"));
    }
    for segment in value.split('/') {
        if segment.is_empty() {
            return Err(format!("path `{value}` has an empty segment"));
        }
        if segment == "." || segment == ".." {
            return Err(format!("path `{value}` is not normalized"));
        }
        if segment.contains(':') {
            return Err(format!("path `{value}` carries a drive prefix"));
        }
        if segment.contains('\0') {
            return Err(format!("path `{value}` carries a NUL byte"));
        }
    }
    Ok(())
}

pub fn file_sha256(path: &Path) -> Result<String, PackError> {
    let bytes = std::fs::read(path).map_err(|e| {
        PackError::new(
            path.parent().unwrap_or(path),
            format!("cannot read `{}`: {e}", path.display()),
        )
    })?;
    Ok(format!("{DIGEST_PREFIX}{}", hex(&Sha256::digest(&bytes))))
}

fn hex(bytes: &[u8]) -> String {
    let mut out = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        out.push(char::from_digit((b >> 4) as u32, 16).unwrap_or('0'));
        out.push(char::from_digit((b & 0xf) as u32, 16).unwrap_or('0'));
    }
    out
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Integrity {
    pub algorithm: String,
    pub digest: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PackManifest {
    pub version: u64,
    pub target: String,
    pub arch: String,
    pub os: String,
    pub env: String,
    pub object: String,
    pub libc: String,
    pub pointer_width: u8,
    pub endian: String,
    pub tier: u8,
    pub capabilities: Vec<String>,
    pub cpu: CpuSpec,
    pub loader: Option<String>,
    pub kernel_min: Option<String>,
    pub glibc_baseline: Option<String>,
    pub prelude: String,
    pub link: LinkMap,
    pub runtime: RuntimeSpec,
    pub integrity: Integrity,
}

#[derive(Clone, Debug)]
pub struct Pack {
    pub root: PathBuf,
    pub manifest: PackManifest,
    pub target: Target,
}

impl Pack {
    pub fn load(dir: &Path) -> Result<Pack, PackError> {
        let manifest = read_manifest(dir)?;
        let target = validate_manifest(dir, &manifest)?;
        let computed = digest(dir)?;
        if computed != manifest.integrity.digest {
            return Err(PackError::new(
                dir,
                format!(
                    "integrity digest mismatch: manifest records {}, files hash to {computed}",
                    manifest.integrity.digest
                ),
            ));
        }
        Ok(Pack {
            root: dir.to_path_buf(),
            manifest,
            target,
        })
    }

    pub fn digest(dir: &Path) -> Result<String, PackError> {
        digest(dir)
    }

    pub fn target(&self) -> &Target {
        &self.target
    }

    pub fn path(&self, rel: &str) -> PathBuf {
        self.root.join(rel)
    }

    pub fn resolve_file(&self, name: &str) -> Result<PathBuf, PackError> {
        resolve_link_path(
            &self.root,
            &self.target.triple,
            &self.manifest.link.search,
            name,
        )
    }
}

pub fn read_manifest(dir: &Path) -> Result<PackManifest, PackError> {
    let manifest_path = dir.join(MANIFEST_FILE);
    let body = std::fs::read(&manifest_path).map_err(|e| {
        if e.kind() == std::io::ErrorKind::NotFound {
            PackError::new(dir, format!("no {MANIFEST_FILE}"))
        } else {
            PackError::at(dir, MANIFEST_FILE, format!("cannot read: {e}"))
        }
    })?;
    let value: Value = serde_json::from_slice(&body)
        .map_err(|e| PackError::at(dir, MANIFEST_FILE, format!("invalid JSON: {e}")))?;
    parse_manifest_value(dir, &value)
}

fn parse_manifest_value(dir: &Path, value: &Value) -> Result<PackManifest, PackError> {
    if !value.is_object() {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            "manifest must be a JSON object",
        ));
    }
    let version = match value.get("version").and_then(Value::as_u64) {
        Some(v) => v,
        None => {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                "missing numeric `version`",
            ));
        }
    };
    if version != PACK_VERSION {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("unsupported target pack manifest version {version}; expected {PACK_VERSION}"),
        ));
    }
    let target = require_str(dir, value, "target")?;
    let arch = require_str(dir, value, "arch")?;
    let os = require_str(dir, value, "os")?;
    let env = require_str(dir, value, "env")?;
    let object = require_str(dir, value, "object")?;
    let libc = require_str(dir, value, "libc")?;
    let endian = require_str(dir, value, "endian")?;
    let pointer_width = value
        .get("pointer_width")
        .and_then(Value::as_u64)
        .filter(|v| *v == 32 || *v == 64)
        .ok_or_else(|| {
            PackError::at(dir, MANIFEST_FILE, "field `pointer_width` must be 32 or 64")
        })? as u8;
    let tier = value
        .get("tier")
        .and_then(Value::as_u64)
        .and_then(|v| u8::try_from(v).ok())
        .filter(|v| Tier::from_number(*v).is_some())
        .ok_or_else(|| PackError::at(dir, MANIFEST_FILE, "field `tier` must be 1, 2, or 3"))?;
    let capabilities = string_list(dir, value, "capabilities", true)?;
    let cpu = match value.get("cpu") {
        Some(cpu @ Value::Object(_)) => CpuSpec {
            model: cpu
                .get("model")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
            features: cpu
                .get("features")
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_string(),
        },
        _ => {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                "field `cpu` must be an object",
            ));
        }
    };
    let loader = optional_str(dir, value, "loader")?;
    let kernel_min = optional_str(dir, value, "kernel_min")?;
    let glibc_baseline = optional_str(dir, value, "glibc_baseline")?;
    let prelude = require_str(dir, value, "prelude")?;
    let link = parse_link(dir, value.get("link"))?;
    let runtime = parse_runtime(dir, value.get("runtime"))?;
    let integrity = parse_integrity(dir, value.get("integrity"))?;
    Ok(PackManifest {
        version,
        target,
        arch,
        os,
        env,
        object,
        libc,
        pointer_width,
        endian,
        tier,
        capabilities,
        cpu,
        loader,
        kernel_min,
        glibc_baseline,
        prelude,
        link,
        runtime,
        integrity,
    })
}

fn require_str(dir: &Path, value: &Value, key: &str) -> Result<String, PackError> {
    match value.get(key).and_then(Value::as_str) {
        Some(s) if !s.is_empty() => Ok(s.to_string()),
        Some(_) => Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("field `{key}` must not be empty"),
        )),
        None => Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("missing string field `{key}`"),
        )),
    }
}

fn optional_str(dir: &Path, value: &Value, key: &str) -> Result<Option<String>, PackError> {
    match value.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(s)) => Ok(Some(s.clone())),
        Some(_) => Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("field `{key}` must be a string or null"),
        )),
    }
}

fn string_list(
    dir: &Path,
    value: &Value,
    key: &str,
    required: bool,
) -> Result<Vec<String>, PackError> {
    match value.get(key) {
        Some(Value::Array(items)) => {
            let mut out = Vec::with_capacity(items.len());
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => {
                        return Err(PackError::at(
                            dir,
                            MANIFEST_FILE,
                            format!("field `{key}` must hold strings"),
                        ));
                    }
                }
            }
            Ok(out)
        }
        Some(_) => Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("field `{key}` must be an array"),
        )),
        None if required => Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("missing array field `{key}`"),
        )),
        None => Ok(Vec::new()),
    }
}

fn parse_link(dir: &Path, value: Option<&Value>) -> Result<LinkMap, PackError> {
    let value = match value {
        Some(value @ Value::Object(_)) => value,
        _ => {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                "field `link` must be an object",
            ));
        }
    };
    let mut link = LinkMap::default();
    link.search = string_list(dir, value, "search", true)?;
    link.crt = path_groups(dir, value, "crt")?;
    link.crt_end = path_groups(dir, value, "crt_end")?;
    link.libs = match value.get("libs") {
        None => BTreeMap::new(),
        Some(Value::Object(map)) => {
            let mut out = BTreeMap::new();
            for (key, item) in map {
                let path = match item {
                    Value::Null => None,
                    Value::String(s) => Some(s.clone()),
                    _ => {
                        return Err(PackError::at(
                            dir,
                            MANIFEST_FILE,
                            format!("link libs entry `{key}` must be a string or null"),
                        ));
                    }
                };
                out.insert(key.clone(), path);
            }
            out
        }
        Some(_) => {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                "field `link.libs` must be an object",
            ));
        }
    };
    link.builtins = optional_str(dir, value, "builtins")?;
    link.runtime = optional_str(dir, value, "runtime")?;
    Ok(link)
}

fn path_groups(
    dir: &Path,
    value: &Value,
    key: &str,
) -> Result<BTreeMap<String, Vec<String>>, PackError> {
    match value.get(key) {
        None => Ok(BTreeMap::new()),
        Some(Value::Object(map)) => {
            let mut out = BTreeMap::new();
            for (mode, items) in map {
                let list = match items {
                    Value::Array(items) => items,
                    _ => {
                        return Err(PackError::at(
                            dir,
                            MANIFEST_FILE,
                            format!("link `{key}.{mode}` must be an array"),
                        ));
                    }
                };
                let mut paths = Vec::with_capacity(list.len());
                for item in list {
                    match item.as_str() {
                        Some(s) => paths.push(s.to_string()),
                        None => {
                            return Err(PackError::at(
                                dir,
                                MANIFEST_FILE,
                                format!("link `{key}.{mode}` must hold strings"),
                            ));
                        }
                    }
                }
                out.insert(mode.clone(), paths);
            }
            Ok(out)
        }
        Some(_) => Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("link `{key}` must be an object"),
        )),
    }
}

fn parse_runtime(dir: &Path, value: Option<&Value>) -> Result<RuntimeSpec, PackError> {
    let value = match value {
        Some(value @ Value::Object(_)) => value,
        _ => {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                "field `runtime` must be an object",
            ));
        }
    };
    let abi = value
        .get("abi")
        .and_then(Value::as_u64)
        .and_then(|v| u32::try_from(v).ok())
        .filter(|v| *v > 0)
        .ok_or_else(|| {
            PackError::at(
                dir,
                MANIFEST_FILE,
                "field `runtime.abi` must be a positive integer",
            )
        })?;
    let file = require_str(dir, value, "file")?;
    let sha256 = match value.get("sha256") {
        None | Some(Value::Null) => None,
        Some(Value::String(s)) if is_digest(s) => Some(s.clone()),
        Some(_) => {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                "field `runtime.sha256` must be `sha256:<64 lowercase hex chars>` or null",
            ));
        }
    };
    Ok(RuntimeSpec { abi, file, sha256 })
}

fn parse_integrity(dir: &Path, value: Option<&Value>) -> Result<Integrity, PackError> {
    let value = match value {
        Some(value @ Value::Object(_)) => value,
        _ => {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                "field `integrity` must be an object",
            ));
        }
    };
    let algorithm = require_str(dir, value, "algorithm")?;
    if algorithm != INTEGRITY_ALGORITHM {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!("unsupported integrity algorithm `{algorithm}`"),
        ));
    }
    let digest = require_str(dir, value, "digest")?;
    if !is_digest(&digest) {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            "field `integrity.digest` must be `sha256:<64 lowercase hex chars>`",
        ));
    }
    Ok(Integrity { algorithm, digest })
}

fn validate_manifest(dir: &Path, manifest: &PackManifest) -> Result<Target, PackError> {
    let target = Target::parse(&manifest.target).map_err(|e| PackError::new(dir, e.to_string()))?;
    compare(dir, "arch", &manifest.arch, target.arch.as_str())?;
    compare(dir, "os", &manifest.os, target.os.as_str())?;
    compare(dir, "env", &manifest.env, target.env.as_str())?;
    compare(dir, "object", &manifest.object, target.object.as_str())?;
    compare(dir, "libc", &manifest.libc, target.libc.as_str())?;
    compare(dir, "endian", &manifest.endian, target.endian.as_str())?;
    if manifest.pointer_width != target.pointer_width {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!(
                "pointer_width {} does not match target `{}` ({})",
                manifest.pointer_width, target.triple, target.pointer_width
            ),
        ));
    }
    if manifest.tier != tier_number(target.tier) {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!(
                "tier {} does not match target `{}` (tier {})",
                manifest.tier,
                target.triple,
                target.tier.as_str()
            ),
        ));
    }
    let mut declared: BTreeSet<Capability> = BTreeSet::new();
    for name in &manifest.capabilities {
        let capability = name
            .parse::<Capability>()
            .map_err(|e| PackError::at(dir, MANIFEST_FILE, e))?;
        if !declared.insert(capability) {
            return Err(PackError::at(
                dir,
                MANIFEST_FILE,
                format!("capability `{name}` is listed twice"),
            ));
        }
    }
    if declared != target.capabilities {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!(
                "capabilities [{}] do not match target `{}` ([{}])",
                manifest.capabilities.join(","),
                target.triple,
                target
                    .capabilities
                    .iter()
                    .map(|c| c.as_str())
                    .collect::<Vec<_>>()
                    .join(",")
            ),
        ));
    }
    if manifest.runtime.file != target.runtime.file {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!(
                "runtime file `{}` does not match target `{}` (`{}`)",
                manifest.runtime.file, target.triple, target.runtime.file
            ),
        ));
    }
    if let Some(linked) = manifest.link.runtime.as_deref()
        && linked != manifest.runtime.file
    {
        return Err(PackError::at(
            dir,
            MANIFEST_FILE,
            format!(
                "link runtime `{linked}` does not match runtime file `{}`",
                manifest.runtime.file
            ),
        ));
    }
    validate_tree(dir, manifest)?;
    Ok(target)
}

fn tier_number(tier: Tier) -> u8 {
    match tier {
        Tier::One => 1,
        Tier::Two => 2,
        Tier::Three => 3,
    }
}

fn compare(dir: &Path, field: &str, declared: &str, expected: &str) -> Result<(), PackError> {
    if declared == expected {
        return Ok(());
    }
    Err(PackError::at(
        dir,
        MANIFEST_FILE,
        format!("`{field}` is `{declared}` but the target descriptor says `{expected}`"),
    ))
}

fn validate_tree(dir: &Path, manifest: &PackManifest) -> Result<(), PackError> {
    pack_files(dir)?;
    for rel in &manifest.link.search {
        validate_rel_path(rel).map_err(|e| PackError::at(dir, rel, e))?;
        let path = dir.join(rel);
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| PackError::at(dir, rel, format!("search directory missing: {e}")))?;
        if meta.file_type().is_symlink() {
            return Err(PackError::at(
                dir,
                rel,
                "symlinks are not allowed in a target pack",
            ));
        }
        if !meta.is_dir() {
            return Err(PackError::at(dir, rel, "search entry is not a directory"));
        }
    }
    for group in [&manifest.link.crt, &manifest.link.crt_end] {
        for paths in group.values() {
            for rel in paths {
                resolve_link_path(dir, &manifest.target, &manifest.link.search, rel)?;
            }
        }
    }
    for rel in manifest.link.libs.values().flatten() {
        resolve_link_path(dir, &manifest.target, &manifest.link.search, rel)?;
    }
    for rel in manifest
        .link
        .builtins
        .iter()
        .chain(manifest.link.runtime.iter())
    {
        resolve_link_path(dir, &manifest.target, &manifest.link.search, rel)?;
    }
    let runtime = resolve_link_path(dir, &manifest.target, &manifest.link.search, &manifest.runtime.file)?;
    if let Some(want) = manifest.runtime.sha256.as_deref() {
        let got = file_sha256(&runtime)?;
        if got != want {
            return Err(PackError::at(
                dir,
                &manifest.runtime.file,
                format!("runtime sha256 {got} does not match the manifest ({want})"),
            ));
        }
    }
    Ok(())
}

fn resolve_link_path(
    dir: &Path,
    triple: &str,
    search: &[String],
    rel: &str,
) -> Result<PathBuf, PackError> {
    validate_rel_path(rel).map_err(|e| PackError::at(dir, rel, e))?;
    for base in search {
        let path = dir.join(base).join(rel);
        if path.is_file() {
            return Ok(path);
        }
    }
    let target_lib = dir.join("lib").join("rustlib").join(triple).join("lib");
    let packed = target_lib.join(rel);
    if packed.is_file() {
        return Ok(packed);
    }
    let direct = dir.join(rel);
    if direct.is_file() {
        return Ok(direct);
    }
    Err(PackError::at(
        dir,
        rel,
        "listed file is missing from the pack",
    ))
}

pub fn pack_files(dir: &Path) -> Result<Vec<String>, PackError> {
    let mut out = Vec::new();
    walk(dir, dir, &mut out)?;
    out.sort();
    Ok(out)
}

fn walk(root: &Path, dir: &Path, out: &mut Vec<String>) -> Result<(), PackError> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| PackError::new(root, format!("cannot read `{}`: {e}", dir.display())))?;
    let mut paths: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
    paths.sort();
    for path in paths {
        let rel = path
            .strip_prefix(root)
            .map_err(|_| PackError::new(root, format!("`{}` escapes the pack", path.display())))?
            .to_string_lossy()
            .replace('\\', "/");
        validate_rel_path(&rel).map_err(|e| PackError::at(root, &rel, e))?;
        let meta = std::fs::symlink_metadata(&path)
            .map_err(|e| PackError::at(root, &rel, format!("cannot stat: {e}")))?;
        let kind = meta.file_type();
        if kind.is_symlink() {
            return Err(PackError::at(
                root,
                &rel,
                "symlinks are not allowed in a target pack",
            ));
        }
        if kind.is_dir() {
            walk(root, &path, out)?;
            continue;
        }
        if !kind.is_file() {
            return Err(PackError::at(
                root,
                &rel,
                "only regular files and directories are allowed in a target pack",
            ));
        }
        out.push(rel);
    }
    Ok(())
}

pub fn digest(dir: &Path) -> Result<String, PackError> {
    let manifest = read_manifest(dir)?;
    let canonical = canonical_manifest(dir, &manifest)?;
    let mut hasher = Sha256::new();
    hasher.update(DIGEST_DOMAIN);
    hasher.update((canonical.len() as u64).to_le_bytes());
    hasher.update(&canonical);
    let files: Vec<String> = pack_files(dir)?
        .into_iter()
        .filter(|rel| rel != MANIFEST_FILE)
        .collect();
    hasher.update((files.len() as u64).to_le_bytes());
    for rel in &files {
        let bytes = std::fs::read(dir.join(rel))
            .map_err(|e| PackError::at(dir, rel, format!("cannot read: {e}")))?;
        hasher.update((rel.len() as u64).to_le_bytes());
        hasher.update(rel.as_bytes());
        hasher.update((bytes.len() as u64).to_le_bytes());
        hasher.update(&bytes);
    }
    Ok(format!("{DIGEST_PREFIX}{}", hex(&hasher.finalize())))
}

fn canonical_manifest(dir: &Path, manifest: &PackManifest) -> Result<Vec<u8>, PackError> {
    let mut capabilities: Vec<&str> = manifest.capabilities.iter().map(String::as_str).collect();
    capabilities.sort();
    capabilities.dedup();
    let value = serde_json::json!({
        "version": manifest.version,
        "target": manifest.target,
        "arch": manifest.arch,
        "os": manifest.os,
        "env": manifest.env,
        "object": manifest.object,
        "libc": manifest.libc,
        "pointer_width": manifest.pointer_width,
        "endian": manifest.endian,
        "tier": manifest.tier,
        "capabilities": capabilities,
        "cpu": {"model": manifest.cpu.model, "features": manifest.cpu.features},
        "loader": manifest.loader,
        "kernel_min": manifest.kernel_min,
        "glibc_baseline": manifest.glibc_baseline,
        "prelude": manifest.prelude,
        "link": {
            "search": manifest.link.search,
            "crt": manifest.link.crt,
            "crt_end": manifest.link.crt_end,
            "libs": manifest.link.libs,
            "builtins": manifest.link.builtins,
            "runtime": manifest.link.runtime,
        },
        "runtime": {
            "abi": manifest.runtime.abi,
            "file": manifest.runtime.file,
            "sha256": manifest.runtime.sha256,
        },
    });
    serde_json::to_vec(&value)
        .map_err(|e| PackError::at(dir, MANIFEST_FILE, format!("cannot canonicalize: {e}")))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::target::Endian;
    use std::fs;

    struct Fixture {
        dir: PathBuf,
    }

    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.dir);
        }
    }

    fn scratch(tag: &str) -> Fixture {
        let dir = std::env::temp_dir().join(format!(
            "rnx-pack-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Fixture { dir }
    }

    fn manifest_json(triple: &str) -> String {
        let target = Target::parse(triple).unwrap();
        let capabilities: Vec<String> = target
            .capabilities
            .iter()
            .map(|c| format!("\"{}\"", c.as_str()))
            .collect();
        let lib = format!("lib/rustlib/{triple}/lib");
        format!(
            "{{\n  \"version\": 2,\n  \"target\": \"{triple}\",\n  \"arch\": \"{}\",\n  \"os\": \"{}\",\n  \"env\": \"{}\",\n  \"object\": \"{}\",\n  \"libc\": \"{}\",\n  \"pointer_width\": {},\n  \"endian\": \"{}\",\n  \"tier\": {},\n  \"capabilities\": [{}],\n  \"cpu\": {{\"model\": \"generic\", \"features\": \"\"}},\n  \"loader\": null,\n  \"kernel_min\": null,\n  \"glibc_baseline\": null,\n  \"prelude\": \"@std/prelude\",\n  \"link\": {{\n    \"search\": [\"{lib}\"],\n    \"crt\": {{\"static\": [\"crt1.o\"]}},\n    \"crt_end\": {{\"static\": [\"crtn.o\"]}},\n    \"libs\": {{\"c\": \"libc.a\"}},\n    \"builtins\": null,\n    \"runtime\": \"libruntime_native.a\"\n  }},\n  \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": null}},\n  \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"sha256:{}\"}}\n}}\n",
            target.arch.as_str(),
            target.os.as_str(),
            target.env.as_str(),
            target.object.as_str(),
            target.libc.as_str(),
            target.pointer_width,
            target.endian.as_str(),
            tier_number(target.tier),
            capabilities.join(", "),
            "0".repeat(64),
        )
    }

    fn seal(dir: &Path, triple: &str) {
        let digest = digest(dir).unwrap();
        let body = manifest_json(triple).replace(&"0".repeat(64), &digest[7..]);
        fs::write(dir.join(MANIFEST_FILE), body).unwrap();
    }

    fn write_pack(tag: &str, triple: &str) -> Fixture {
        let fixture = scratch(tag);
        let lib = fixture
            .dir
            .join("lib")
            .join("rustlib")
            .join(triple)
            .join("lib");
        fs::create_dir_all(&lib).unwrap();
        for (name, bytes) in [
            ("crt1.o", b"crt1".as_slice()),
            ("crtn.o", b"crtn"),
            ("libc.a", b"libc"),
            ("libruntime_native.a", b"runtime"),
        ] {
            fs::write(lib.join(name), bytes).unwrap();
        }
        fs::write(fixture.dir.join(MANIFEST_FILE), manifest_json(triple)).unwrap();
        seal(&fixture.dir, triple);
        fixture
    }

    #[test]
    fn pack_loads_and_reports_its_target() {
        let fixture = write_pack("load", "x86_64-unknown-linux-musl");
        let pack = Pack::load(&fixture.dir).unwrap();
        assert_eq!(pack.target().triple, "x86_64-unknown-linux-musl");
        assert_eq!(
            pack.manifest.integrity.digest,
            digest(&fixture.dir).unwrap()
        );
        assert_eq!(
            pack.target(),
            &Target::parse("x86_64-unknown-linux-musl").unwrap()
        );
        assert_eq!(pack.target().endian, Endian::Little);
    }

    #[test]
    fn digest_is_stable_across_runs_and_mtimes() {
        let fixture = write_pack("stable", "aarch64-unknown-linux-musl");
        let first = digest(&fixture.dir).unwrap();
        let second = digest(&fixture.dir).unwrap();
        assert_eq!(first, second);
        let crt = fixture
            .dir
            .join("lib")
            .join("rustlib")
            .join("aarch64-unknown-linux-musl")
            .join("lib")
            .join("crt1.o");
        let later = std::time::SystemTime::now() + std::time::Duration::from_secs(3600);
        if let Ok(file) = fs::File::options().write(true).open(&crt) {
            let _ = file.set_modified(later);
        }
        assert_eq!(digest(&fixture.dir).unwrap(), first);
    }

    #[test]
    fn digest_changes_with_file_bytes() {
        let fixture = write_pack("bytes", "x86_64-unknown-linux-musl");
        let first = digest(&fixture.dir).unwrap();
        let libc = fixture
            .dir
            .join("lib")
            .join("rustlib")
            .join("x86_64-unknown-linux-musl")
            .join("lib")
            .join("libc.a");
        fs::write(&libc, b"tampered").unwrap();
        assert_ne!(digest(&fixture.dir).unwrap(), first);
    }

    #[test]
    fn load_rejects_a_corrupt_file() {
        let fixture = write_pack("corrupt", "x86_64-unknown-linux-musl");
        let libc = fixture
            .dir
            .join("lib")
            .join("rustlib")
            .join("x86_64-unknown-linux-musl")
            .join("lib")
            .join("libc.a");
        fs::write(&libc, b"tampered").unwrap();
        let err = Pack::load(&fixture.dir).unwrap_err();
        assert_eq!(err.code(), Code::E124);
        assert!(
            err.to_string().contains("integrity digest mismatch"),
            "{err}"
        );
        assert!(
            err.to_string()
                .contains(fixture.dir.to_string_lossy().as_ref()),
            "{err}"
        );
    }

    #[test]
    fn manifest_paths_must_be_relative_and_normalized() {
        for (tag, search) in [
            ("absolute", "[\"/usr/lib\"]"),
            ("parent", "[\"lib/../out\"]"),
            ("empty", "[\"lib//rustlib\"]"),
            ("dot", "[\"./lib\"]"),
        ] {
            let fixture = scratch(tag);
            let body = manifest_json("x86_64-unknown-linux-musl").replace(
                "\"search\": [\"lib/rustlib/x86_64-unknown-linux-musl/lib\"]",
                &format!("\"search\": {search}"),
            );
            fs::write(fixture.dir.join(MANIFEST_FILE), body).unwrap();
            let err = Pack::load(&fixture.dir).unwrap_err();
            assert_eq!(err.code(), Code::E124, "{tag}");
            let text = err.to_string();
            assert!(
                text.contains("absolute")
                    || text.contains("normalized")
                    || text.contains("empty segment"),
                "{tag}: {err}"
            );
        }
    }

    #[test]
    fn manifest_rejects_missing_files_and_symlinks() {
        let fixture = write_pack("missing", "x86_64-unknown-linux-musl");
        fs::remove_file(
            fixture
                .dir
                .join("lib")
                .join("rustlib")
                .join("x86_64-unknown-linux-musl")
                .join("lib")
                .join("libc.a"),
        )
        .unwrap();
        let err = Pack::load(&fixture.dir).unwrap_err();
        assert!(err.to_string().contains("libc.a"), "{err}");

        #[cfg(unix)]
        {
            let fixture = write_pack("symlink", "x86_64-unknown-linux-musl");
            let link = fixture.dir.join("lib").join("alias.a");
            std::os::unix::fs::symlink("rustlib", &link).unwrap();
            let err = Pack::load(&fixture.dir).unwrap_err();
            assert!(err.to_string().contains("symlink"), "{err}");
        }
    }

    #[test]
    fn manifest_must_agree_with_the_descriptor() {
        let fixture = write_pack("drift", "x86_64-unknown-linux-musl");
        let body = fs::read_to_string(fixture.dir.join(MANIFEST_FILE))
            .unwrap()
            .replace("\"libc\": \"musl\"", "\"libc\": \"glibc\"");
        fs::write(fixture.dir.join(MANIFEST_FILE), body).unwrap();
        let err = Pack::load(&fixture.dir).unwrap_err();
        assert!(err.to_string().contains("libc"), "{err}");

        let fixture = write_pack("unknown", "x86_64-unknown-linux-musl");
        let body = fs::read_to_string(fixture.dir.join(MANIFEST_FILE))
            .unwrap()
            .replace(
                "\"target\": \"x86_64-unknown-linux-musl\"",
                "\"target\": \"riscv64-unknown-linux-gnu\"",
            );
        fs::write(fixture.dir.join(MANIFEST_FILE), body).unwrap();
        let err = Pack::load(&fixture.dir).unwrap_err();
        assert!(err.to_string().contains("unsupported target"), "{err}");
    }

    #[test]
    fn runtime_hash_is_checked_when_recorded() {
        let fixture = write_pack("runtime", "x86_64-unknown-linux-musl");
        let runtime = fixture
            .dir
            .join("lib")
            .join("rustlib")
            .join("x86_64-unknown-linux-musl")
            .join("lib")
            .join("libruntime_native.a");
        let recorded = file_sha256(&runtime).unwrap();
        let before = fs::read_to_string(fixture.dir.join(MANIFEST_FILE)).unwrap();
        let previous = read_manifest(&fixture.dir).unwrap().integrity.digest;
        let body = before.replace("\"sha256\": null", &format!("\"sha256\": \"{recorded}\""));
        fs::write(fixture.dir.join(MANIFEST_FILE), body).unwrap();
        let digest = digest(&fixture.dir).unwrap();
        let sealed = fs::read_to_string(fixture.dir.join(MANIFEST_FILE))
            .unwrap()
            .replace(&previous, &digest);
        fs::write(fixture.dir.join(MANIFEST_FILE), sealed).unwrap();
        Pack::load(&fixture.dir).unwrap();
        fs::write(&runtime, b"swapped").unwrap();
        let err = Pack::load(&fixture.dir).unwrap_err();
        assert!(err.to_string().contains("runtime sha256"), "{err}");
    }

    #[test]
    fn digest_spelling_is_strict() {
        assert!(is_digest(&format!("sha256:{}", "ab".repeat(32))));
        assert!(!is_digest(&format!("sha256:{}", "AB".repeat(32))));
        assert!(!is_digest("sha256:abc"));
        assert!(!is_digest(""));
    }
}
