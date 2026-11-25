use crate::rustdep::bridge::{self, BridgeInputs};
use crate::rustdep::docs::{self, RustItem};
use crate::rustdep::toolchain::ToolchainInfo;
use crate::rustdep::wrapper::{self, CExport};
use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

pub fn generate_rustdoc_json(
    root: &Path,
    key: &str,
    inputs: &BridgeInputs,
    toolchain: &ToolchainInfo,
) -> Result<BTreeMap<String, Vec<RustItem>>, Diagnostic> {
    let files = bridge::generate_bridge(root, key, inputs)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot generate Rust bridge: {e}")))?;
    let mut all = BTreeMap::new();
    for request in &inputs.requests {
        let output = Command::new(&toolchain.cargo_path)
            .arg("rustdoc")
            .arg("-p")
            .arg(&request.cargo_name)
            .arg("--manifest-path")
            .arg(&files.manifest_path)
            .arg("--target")
            .arg(&toolchain.host)
            .arg("--")
            .arg("-Z")
            .arg("unstable-options")
            .arg("--output-format")
            .arg("json")
            .env("RUSTC_BOOTSTRAP", "1")
            .output()
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot run cargo rustdoc: {e}")))?;
        if !output.status.success() {
            return Err(Diagnostic::new(
                Code::E108,
                format!(
                    "cargo rustdoc failed for `{}`: {}",
                    request.cargo_name,
                    String::from_utf8_lossy(&output.stderr).trim()
                ),
            ));
        }
        let json_path = files
            .dir
            .join("target")
            .join(&toolchain.host)
            .join("doc")
            .join(format!("{}.json", request.cargo_name));
        let text = std::fs::read_to_string(&json_path).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot read rustdoc JSON at {}: {e}", json_path.display()),
            )
        })?;
        all.insert(request.name.clone(), docs::parse_rustdoc_json(&text).map_err(|e| {
            Diagnostic::new(
                Code::E108,
                format!("cannot parse rustdoc JSON for `{}`: {e}", request.name),
            )
        })?);
    }
    Ok(all)
}

pub fn stable_wrappers_dir(root: &Path) -> PathBuf {
    bridge::bridge_dir(root, "wrappers")
}

pub fn wrapper_path(root: &Path, module: &str) -> Option<PathBuf> {
    let stable = stable_wrappers_dir(root).join(format!("{module}.rnx"));
    if stable.is_file() {
        return Some(stable);
    }
    let base = root.join(bridge::CACHE_ROOT);
    let mut candidates: Vec<PathBuf> = std::fs::read_dir(&base)
        .ok()?
        .filter_map(|entry| entry.ok())
        .map(|entry| entry.path().join("wrappers").join(format!("{module}.rnx")))
        .filter(|path| path.is_file())
        .collect();
    candidates.sort();
    candidates.pop()
}

pub fn generate_wrappers(
    root: &Path,
    key: &str,
    items: &BTreeMap<String, Vec<RustItem>>,
    monomorphs: &BTreeMap<String, Vec<Vec<String>>>,
) -> Result<BTreeMap<String, PathBuf>, Diagnostic> {
    let dir = bridge::bridge_dir(root, key).join("wrappers");
    let stable_dir = stable_wrappers_dir(root);
    for target in [&dir, &stable_dir] {
        std::fs::create_dir_all(target)
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot create {}: {e}", target.display())))?;
    }
    let mut paths = BTreeMap::new();
    for (module, items) in items {
        let exports: Vec<CExport> =
            wrapper::exports_from_items(module, items, monomorphs.get(module).map(Vec::as_slice).unwrap_or(&[]));
        let source = wrapper::wrapper_rnx_source(&exports);
        let path = dir.join(format!("{module}.rnx"));
        let stable = stable_dir.join(format!("{module}.rnx"));
        for target in [&path, &stable] {
            std::fs::write(target, &source).map_err(|e| {
                Diagnostic::new(Code::E108, format!("cannot write {}: {e}", target.display()))
            })?;
        }
        paths.insert(module.clone(), path);
    }
    Ok(paths)
}

pub fn c_exports(
    items: &BTreeMap<String, Vec<RustItem>>,
    monomorphs: &BTreeMap<String, Vec<Vec<String>>>,
) -> String {
    let mut out = String::new();
    for (module, items) in items {
        let exports = wrapper::exports_from_items(
            module,
            items,
            monomorphs.get(module).map(Vec::as_slice).unwrap_or(&[]),
        );
        out.push_str(&wrapper::bridge_c_exports(module, &exports));
    }
    out
}
