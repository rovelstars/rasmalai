use crate::checksum::Sha256;
use crate::config::ConfigValue;
use crate::deplock::rnx_string;
use crate::project::{DependencySpec, ProjectConfig, RustSource};
use crate::rustdep::bridge::{self, BridgeInputs, BridgeProfile};
use crate::rustdep::toolchain::{self, ToolchainInfo};
use diagnostics::{Code, Diagnostic};
use std::path::Path;
use std::process::Command;

pub const BRIDGE_ABI_VERSION: u32 = 1;
pub const RUST_GRAPH_VERSION: u32 = 1;
pub const BRIDGE_CRATE_NAME: &str = "rnx_rust_bridge";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustGraphRequest {
    pub name: String,
    pub cargo_name: String,
    pub source: RustSource,
    pub version: Option<String>,
    pub features: Vec<String>,
    pub default_features: bool,
    pub monomorphs: Vec<Vec<String>>,
}

impl RustGraphRequest {
    pub fn descriptor(&self) -> String {
        let mut s = format!("{}@{}", self.cargo_name, self.source.describe());
        if let Some(version) = &self.version {
            s.push_str(&format!(":{version}"));
        }
        s.push_str(&format!(":{}", self.features.join(",")));
        s.push_str(if self.default_features { ":default" } else { ":no-default" });
        for types in &self.monomorphs {
            s.push_str(&format!(":{}", types.join(",")));
        }
        s
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustPackage {
    pub name: String,
    pub version: String,
    pub source: String,
    pub checksum: Option<String>,
    pub features: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RustGraph {
    pub version: u32,
    pub packages: Vec<RustPackage>,
    pub cargo_lock_digest: String,
    pub rustc_version: String,
    pub llvm_version: String,
    pub bridge_abi: u32,
    pub artifact_digest: Option<String>,
}

impl Default for RustGraph {
    fn default() -> Self {
        RustGraph {
            version: RUST_GRAPH_VERSION,
            packages: Vec::new(),
            cargo_lock_digest: String::new(),
            rustc_version: String::new(),
            llvm_version: String::new(),
            bridge_abi: BRIDGE_ABI_VERSION,
            artifact_digest: None,
        }
    }
}

pub fn requests_from_configs(configs: &[&ProjectConfig]) -> Vec<RustGraphRequest> {
    let mut out: Vec<RustGraphRequest> = configs
        .iter()
        .flat_map(|config| requests_from_config(config))
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out.dedup();
    out
}

pub fn requests_from_config(config: &ProjectConfig) -> Vec<RustGraphRequest> {
    let mut out: Vec<RustGraphRequest> = config
        .dependencies
        .iter()
        .filter_map(|(name, spec)| match spec {
            DependencySpec::Rust { package, source, version, features, default_features, monomorphs, .. } => {
                Some(RustGraphRequest {
                    name: name.clone(),
                    cargo_name: package.clone().unwrap_or_else(|| name.clone()),
                    source: source.clone(),
                    version: version.clone(),
                    features: features.clone(),
                    default_features: *default_features,
                    monomorphs: monomorphs.clone(),
                })
            }
            _ => None,
        })
        .collect();
    out.sort_by(|a, b| a.name.cmp(&b.name));
    out
}

fn cargo_args(requests: &[RustGraphRequest], toolchain: &ToolchainInfo) -> BridgeInputs {
    BridgeInputs {
        requests: requests.to_vec(),
        rustc_version: toolchain.rustc.clone(),
        llvm_version: toolchain.llvm.clone(),
        host: toolchain.host.clone(),
        profile: BridgeProfile::Dev,
        allocator_forwarding: true,
        lto: false,
    }
}

pub fn resolve_rust_graph(
    root: &Path,
    config: &ProjectConfig,
    toolchain: &ToolchainInfo,
) -> Result<Option<RustGraph>, Diagnostic> {
    resolve_rust_graph_with_requests(root, requests_from_config(config), toolchain)
}

pub fn resolve_rust_graph_with_requests(
    root: &Path,
    requests: Vec<RustGraphRequest>,
    toolchain: &ToolchainInfo,
) -> Result<Option<RustGraph>, Diagnostic> {
    if requests.is_empty() {
        return Ok(None);
    }
    let inputs = cargo_args(&requests, toolchain);
    let key = bridge::bridge_cache_key(&inputs);
    let files = bridge::generate_bridge(root, &key, &inputs).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot generate Rust bridge: {e}"))
    })?;
    let output = Command::new(&toolchain.cargo_path)
        .arg("metadata")
        .arg("--format-version")
        .arg("1")
        .arg("--manifest-path")
        .arg(&files.manifest_path)
        .output()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot run cargo metadata: {e}")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Diagnostic::new(
            Code::E108,
            format!("cargo could not resolve rust dependencies: {}", if err.is_empty() { "non-zero exit status" } else { &err }),
        )
        .with_hint("check the `rust:` declarations in Project.config"));
    }
    let meta: serde_json::Value = serde_json::from_slice(&output.stdout)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot parse cargo metadata: {e}")))?;
    let mut msrv: Option<String> = None;
    let mut packages: Vec<RustPackage> = Vec::new();
    let mut features_by_id: std::collections::BTreeMap<String, Vec<String>> = std::collections::BTreeMap::new();
    if let Some(nodes) = meta.get("resolve").and_then(|r| r.get("nodes")).and_then(|n| n.as_array()) {
        for node in nodes {
            let Some(id) = node.get("id").and_then(|v| v.as_str()) else {
                continue;
            };
            let mut features: Vec<String> = node
                .get("features")
                .and_then(|v| v.as_array())
                .into_iter()
                .flatten()
                .filter_map(|v| v.as_str().map(str::to_string))
                .collect();
            features.sort();
            features.dedup();
            features_by_id.insert(id.to_string(), features);
        }
    }
    if let Some(items) = meta.get("packages").and_then(|v| v.as_array()) {
        for item in items {
            let name = item.get("name").and_then(|v| v.as_str()).unwrap_or("");
            if name == BRIDGE_CRATE_NAME {
                continue;
            }
            let version = item.get("version").and_then(|v| v.as_str()).unwrap_or("");
            if version.is_empty() {
                continue;
            }
            if let Some(rust_version) = item.get("rust_version").and_then(|v| v.as_str()) {
                let better = msrv.as_deref().is_none_or(|old| {
                    toolchain::version_key(rust_version) > toolchain::version_key(old)
                });
                if better {
                    msrv = Some(rust_version.to_string());
                }
            }
            let source = item.get("source").and_then(|v| v.as_str()).unwrap_or("path").to_string();
            let checksum = item.get("checksum").and_then(|v| v.as_str()).map(str::to_string);
            let id = item.get("id").and_then(|v| v.as_str()).unwrap_or("");
            let features = features_by_id.get(id).cloned().unwrap_or_default();
            packages.push(RustPackage { name: name.to_string(), version: version.to_string(), source, checksum, features });
        }
    }
    if packages.is_empty() {
        return Err(Diagnostic::new(
            Code::E108,
            "cargo metadata resolved no packages for the declared `rust:` dependencies",
        ));
    }
    toolchain::check_msrv(msrv.as_deref(), &toolchain.rustc)
        .map_err(|e| Diagnostic::new(Code::E108, e))?;
    let doc_items = crate::rustdep::rustdoc::generate_rustdoc_json(root, &key, &inputs, toolchain)?;
    let monomorphs: std::collections::BTreeMap<String, Vec<Vec<String>>> = requests
        .iter()
        .map(|request| (request.name.clone(), request.monomorphs.clone()))
        .collect();
    crate::rustdep::rustdoc::generate_wrappers(root, &key, &doc_items, &monomorphs)?;
    packages.sort_by(|a, b| a.name.cmp(&b.name).then(a.version.cmp(&b.version)));
    let lock_path = files.dir.join("Cargo.lock");
    let lock_bytes = std::fs::read(&lock_path).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot read generated Cargo.lock: {e}"))
    })?;
    Ok(Some(RustGraph {
        version: RUST_GRAPH_VERSION,
        packages,
        cargo_lock_digest: format!("sha256:{}", Sha256::hexdigest(&lock_bytes)),
        rustc_version: toolchain.rustc.clone(),
        llvm_version: toolchain.llvm.clone(),
        bridge_abi: BRIDGE_ABI_VERSION,
        artifact_digest: None,
    }))
}

pub fn verify_rust_graph(config: &ProjectConfig, lock: Option<&RustGraph>) -> Result<(), Diagnostic> {
    let requests = requests_from_config(config);
    if requests.is_empty() {
        return Ok(());
    }
    let Some(graph) = lock else {
        return Err(Diagnostic::new(
            Code::E108,
            "Project.deplock has no Rust graph for the declared `rust:` dependencies, run 'rnx lock'",
        ));
    };
    if graph.version != RUST_GRAPH_VERSION || graph.bridge_abi != BRIDGE_ABI_VERSION {
        return Err(Diagnostic::new(
            Code::E108,
            format!("unsupported Rust graph version {} / ABI {}, run 'rnx lock'", graph.version, graph.bridge_abi),
        ));
    }
    let mut by_name: std::collections::BTreeMap<&str, &RustPackage> = std::collections::BTreeMap::new();
    for package in &graph.packages {
        by_name.entry(&package.name).or_insert(package);
    }
    for request in &requests {
        let Some(package) = by_name.get(request.cargo_name.as_str()) else {
            return Err(Diagnostic::new(
                Code::E108,
                format!("rust dependency `{}` is missing from Project.deplock, run 'rnx lock'", request.name),
            ));
        };
        if let Some(want) = &request.version
            && !package.version_matches(want)
        {
            return Err(Diagnostic::new(
                Code::E108,
                format!("rust dependency `{}` version changed (Project.deplock has {}, project requests {want}), run 'rnx lock'", request.name, package.version),
            ));
        }
        for feature in &request.features {
            if !package.features.iter().any(|f| f == feature) {
                return Err(Diagnostic::new(
                    Code::E108,
                    format!("rust dependency `{}` lost feature `{feature}` in Project.deplock, run 'rnx lock'", request.name),
                ));
            }
        }
    }
    Ok(())
}

impl RustPackage {
    fn version_matches(&self, request: &str) -> bool {
        let request = request.trim();
        let (prefix, rest) = match request.chars().next() {
            Some('^' | '~') => (&request[..1], &request[1..]),
            Some('=') => ("", &request[1..]),
            _ if request.chars().all(|c| c.is_ascii_digit() || c == '.') => ("^", request),
            _ => ("", request),
        };
        let (core, pre) = match rest.trim().split_once('-') {
            Some((core, pre)) => (core, format!("-{pre}")),
            None => (rest.trim(), String::new()),
        };
        if !core.chars().all(|c| c.is_ascii_digit() || c == '.') || core.is_empty() {
            return crate::fetch::engine_satisfied(request, &self.version);
        }
        let mut parts: Vec<&str> = core.split('.').filter(|p| !p.is_empty()).collect();
        while parts.len() < 3 {
            parts.push("0");
        }
        let range = format!("{prefix}{}{pre}", parts.join("."));
        crate::fetch::engine_satisfied(&range, &self.version)
    }
}

pub fn rust_graph_to_rnx(graph: &RustGraph) -> String {
    let mut out = String::from("    rust: {\n");
    out.push_str(&format!("        version: {},\n", graph.version));
    out.push_str("        packages: [");
    if graph.packages.is_empty() {
        out.push_str("],\n");
    } else {
        out.push('\n');
        for (i, package) in graph.packages.iter().enumerate() {
            out.push_str("            {\n");
            out.push_str(&format!("                name: {},\n", rnx_string(&package.name)));
            out.push_str(&format!("                version: {},\n", rnx_string(&package.version)));
            out.push_str(&format!("                source: {},\n", rnx_string(&package.source)));
            if let Some(checksum) = &package.checksum {
                out.push_str(&format!("                checksum: {},\n", rnx_string(checksum)));
            }
            let features: Vec<String> = package.features.iter().map(|f| rnx_string(f)).collect();
            out.push_str(&format!("                features: [{}]\n", features.join(", ")));
            out.push_str("            }");
            if i + 1 < graph.packages.len() {
                out.push(',');
            }
            out.push('\n');
        }
        out.push_str("        ],\n");
    }
    out.push_str(&format!("        cargoLockDigest: {},\n", rnx_string(&graph.cargo_lock_digest)));
    out.push_str(&format!("        rustcVersion: {},\n", rnx_string(&graph.rustc_version)));
    out.push_str(&format!("        llvmVersion: {},\n", rnx_string(&graph.llvm_version)));
    out.push_str(&format!("        bridgeAbi: {},\n", graph.bridge_abi));
    if let Some(digest) = &graph.artifact_digest {
        out.push_str(&format!("        artifactDigest: {},\n", rnx_string(digest)));
    }
    out.push_str("    },\n");
    out
}

pub fn parse_rust_graph(value: &ConfigValue) -> Result<RustGraph, String> {
    let ConfigValue::Object(_) = value else {
        return Err("field `rust` must be an object".to_string());
    };
    let get_str = |key: &str| -> Result<String, String> {
        match value.get(key) {
            Some(ConfigValue::String(s)) => Ok(s.clone()),
            Some(_) => Err(format!("rust graph field `{key}` must be a string")),
            None => Err(format!("rust graph missing field `{key}`")),
        }
    };
    let version = match value.get("version") {
        Some(ConfigValue::Int(v)) if *v >= 0 => *v as u32,
        Some(_) => return Err("rust graph field `version` must be a non-negative integer".to_string()),
        None => RUST_GRAPH_VERSION,
    };
    let bridge_abi = match value.get("bridgeAbi") {
        Some(ConfigValue::Int(v)) if *v >= 0 => *v as u32,
        Some(_) => return Err("rust graph field `bridgeAbi` must be a non-negative integer".to_string()),
        None => BRIDGE_ABI_VERSION,
    };
    let mut packages = Vec::new();
    match value.get("packages") {
        Some(ConfigValue::Array(items)) => {
            for item in items {
                let ConfigValue::Object(_) = item else {
                    return Err("rust graph package must be an object".to_string());
                };
                let str_field = |key: &str| -> Result<String, String> {
                    match item.get(key) {
                        Some(ConfigValue::String(s)) => Ok(s.clone()),
                        Some(_) => Err(format!("rust graph package field `{key}` must be a string")),
                        None => Err(format!("rust graph package missing field `{key}`")),
                    }
                };
                let features = match item.get("features") {
                    Some(ConfigValue::Array(items)) => items
                        .iter()
                        .map(|v| match v {
                            ConfigValue::String(s) => Ok(s.clone()),
                            _ => Err("rust graph package features must be strings".to_string()),
                        })
                        .collect::<Result<Vec<_>, _>>()?,
                    Some(_) => return Err("rust graph package field `features` must be an array".to_string()),
                    None => Vec::new(),
                };
                let checksum = match item.get("checksum") {
                    Some(ConfigValue::String(s)) => Some(s.clone()),
                    Some(_) => return Err("rust graph package field `checksum` must be a string".to_string()),
                    None => None,
                };
                packages.push(RustPackage {
                    name: str_field("name")?,
                    version: str_field("version")?,
                    source: str_field("source")?,
                    checksum,
                    features,
                });
            }
        }
        Some(_) => return Err("rust graph field `packages` must be an array".to_string()),
        None => {}
    }
    let artifact_digest = match value.get("artifactDigest") {
        Some(ConfigValue::String(s)) => Some(s.clone()),
        Some(_) => return Err("rust graph field `artifactDigest` must be a string".to_string()),
        None => None,
    };
    Ok(RustGraph {
        version,
        packages,
        cargo_lock_digest: get_str("cargoLockDigest")?,
        rustc_version: get_str("rustcVersion")?,
        llvm_version: get_str("llvmVersion")?,
        bridge_abi,
        artifact_digest,
    })
}
