use crate::checksum::Sha256;
use crate::rustdep::bridge::{self, BridgeFiles, BridgeInputs, BridgeProfile};
use crate::rustdep::graph::BRIDGE_CRATE_NAME;
use crate::rustdep::toolchain::{ToolchainInfo, find_tool};
use diagnostics::{Code, Diagnostic};
use std::path::{Path, PathBuf};
use std::process::Command;

pub const NATIVE_LIB_NAME: &str = "rnx_rust_bridge";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeArtifact {
    pub static_path: PathBuf,
    pub shared_path: PathBuf,
    pub sha256: String,
    pub key: String,
}

pub fn build_bridge(
    root: &Path,
    key: &str,
    inputs: &BridgeInputs,
    toolchain: &ToolchainInfo,
) -> Result<BridgeArtifact, Diagnostic> {
    let files = bridge::generate_bridge(root, key, inputs)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot generate Rust bridge: {e}")))?;
    let items = crate::rustdep::rustdoc::generate_rustdoc_json(root, key, inputs, toolchain)?;
    let monomorphs: std::collections::BTreeMap<String, Vec<Vec<String>>> = inputs
        .requests
        .iter()
        .map(|request| (request.name.clone(), request.monomorphs.clone()))
        .collect();
    let exports = crate::rustdep::rustdoc::c_exports(&items, &monomorphs);
    let mut keep_symbols: Vec<String> = Vec::new();
    for (module, items) in &items {
        for export in crate::rustdep::wrapper::exports_from_items(
            module,
            items,
            monomorphs.get(module).map(Vec::as_slice).unwrap_or(&[]),
        ) {
            keep_symbols.push(export.symbol);
            keep_symbols.push(export.rust_call.split("::<").next().unwrap_or("").to_string());
        }
    }
    if !exports.is_empty() {
        let source = std::fs::read_to_string(&files.source_path)
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read bridge source: {e}")))?;
        std::fs::write(&files.source_path, format!("{source}\n{exports}"))
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write bridge source: {e}")))?;
    }
    crate::rustdep::rustdoc::generate_wrappers(root, key, &items, &monomorphs)?;
    let release = inputs.profile == BridgeProfile::Release;
    let mut command = Command::new(&toolchain.cargo_path);
    command
        .arg("build")
        .arg("--manifest-path")
        .arg(&files.manifest_path)
        .arg("--target")
        .arg(&toolchain.host);
    if release {
        command.arg("--release");
    }
    if inputs.lto {
        command.env("RUSTFLAGS", "-Clinker-plugin-lto");
    }
    let output = command
        .output()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot run cargo build: {e}")))?;
    if !output.status.success() {
        let err = String::from_utf8_lossy(&output.stderr).trim().to_string();
        return Err(Diagnostic::new(
            Code::E108,
            format!("cargo could not build the Rust bridge: {}", if err.is_empty() { "non-zero exit status" } else { &err }),
        ));
    }
    let profile_dir = if release { "release" } else { "debug" };
    let target_dir = files.dir.join("target").join(&toolchain.host).join(profile_dir);
    let static_path = target_dir.join(format!("lib{BRIDGE_CRATE_NAME}.a"));
    let shared_path = target_dir.join(format!("lib{BRIDGE_CRATE_NAME}.so"));
    for path in [&static_path, &shared_path] {
        if !path.is_file() {
            return Err(Diagnostic::new(
                Code::E108,
                format!("cargo built no bridge artifact at {}", path.display()),
            ));
        }
    }
    let static_path = if inputs.lto {
        static_path
    } else {
        internalize_rust_symbols(&static_path, &files.dir, &keep_symbols)?
    };
    let linked_bytes = std::fs::read(&static_path)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read bridge artifact: {e}")))?;
    Ok(BridgeArtifact {
        static_path,
        shared_path,
        sha256: format!("sha256:{}", Sha256::hexdigest(&linked_bytes)),
        key: key.to_string(),
    })
}

pub fn fingerprint(artifact: &BridgeArtifact, files: &BridgeFiles) -> String {
    format!("{}:{}:{}", files.manifest_path.display(), artifact.key, artifact.sha256)
}

fn internalize_rust_symbols(
    static_path: &Path,
    dir: &Path,
    keep_symbols: &[String],
) -> Result<PathBuf, Diagnostic> {
    let nm = find_tool("llvm-nm")
        .or_else(|_| find_tool("nm"))
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot internalize Rust bridge symbols: {e}")))?;
    let objcopy = find_tool("objcopy")
        .or_else(|_| find_tool("llvm-objcopy"))
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot internalize Rust bridge symbols: {e}")))?;
    let listing = Command::new(&nm)
        .arg("-g")
        .arg("--defined-only")
        .arg("--format=posix")
        .arg(static_path)
        .output()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot list bridge symbols: {e}")))?;
    if !listing.status.success() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("cannot list bridge symbols: {}", String::from_utf8_lossy(&listing.stderr).trim()),
        ));
    }
    let mut keep: Vec<String> = Vec::new();
    for line in String::from_utf8_lossy(&listing.stdout).lines() {
        if line.trim_end().ends_with(':') {
            continue;
        }
        let mut fields = line.split_whitespace();
        let Some(name) = fields.next() else {
            continue;
        };
        if !name.starts_with("rnx_") && !keep_symbols.iter().any(|symbol| symbol == name) {
            continue;
        }
        if fields.next().is_some_and(|kind| kind.len() == 1 && kind.chars().all(|c| c.is_ascii_uppercase()))
            && !keep.contains(&name.to_string())
        {
            keep.push(name.to_string());
        }
    }
    let keep_path = dir.join("keep-symbols.txt");
    std::fs::write(&keep_path, keep.join("\n") + "\n")
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot write bridge symbol list: {e}")))?;
    let filtered = dir.join("librnx_rust_bridge_internal.a");
    let output = Command::new(&objcopy)
        .arg(format!("--keep-global-symbols={}", keep_path.display()))
        .arg(static_path)
        .arg(&filtered)
        .output()
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot internalize bridge symbols: {e}")))?;
    if !output.status.success() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("cannot internalize bridge symbols: {}", String::from_utf8_lossy(&output.stderr).trim()),
        ));
    }
    Ok(filtered)
}
