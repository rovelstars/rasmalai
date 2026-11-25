use std::path::PathBuf;
use std::process::Command;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolchainInfo {
    pub cargo: String,
    pub rustc: String,
    pub llvm: String,
    pub host: String,
    pub cargo_path: PathBuf,
    pub rustc_path: PathBuf,
}

pub fn find_tool(name: &str) -> Result<PathBuf, String> {
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        let candidate = dir.join(name);
        if candidate.is_file() {
            return Ok(candidate);
        }
    }
    Err(format!("cannot find `{name}` on PATH; install a Rust toolchain for `rust:` dependencies"))
}

fn output(command: &mut Command) -> Result<String, String> {
    let out = command
        .output()
        .map_err(|e| format!("cannot run Rust toolchain: {e}"))?;
    if !out.status.success() {
        let err = String::from_utf8_lossy(&out.stderr).trim().to_string();
        return Err(format!("Rust toolchain check failed: {}", if err.is_empty() { "non-zero exit status" } else { &err }));
    }
    Ok(String::from_utf8_lossy(&out.stdout).into_owned())
}

pub fn detect_toolchain() -> Result<ToolchainInfo, String> {
    let rustc_path = find_tool("rustc")?;
    let cargo_path = find_tool("cargo")?;
    let verbose = output(Command::new(&rustc_path).arg("-vV"))?;
    let rustc = verbose
        .lines()
        .find_map(|line| line.strip_prefix("rustc "))
        .map(|line| line.split_whitespace().next().unwrap_or("").to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "cannot parse `rustc -vV` output".to_string())?;
    let host = verbose
        .lines()
        .find_map(|line| line.strip_prefix("host: "))
        .map(|line| line.trim().to_string())
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "cannot find `host` in `rustc -vV` output".to_string())?;
    let llvm = std::env::var("RNX_RUSTC_LLVM")
        .ok()
        .filter(|value| !value.trim().is_empty())
        .map(|value| normalize_llvm_version(&value))
        .or_else(|| {
            verbose
                .lines()
                .find_map(|line| line.strip_prefix("LLVM version: "))
                .map(|line| normalize_llvm_version(line.trim()))
                .filter(|s| !s.is_empty())
        })
        .ok_or_else(|| "cannot find `LLVM version` in `rustc -vV` output".to_string())?;
    let cargo_line = output(Command::new(&cargo_path).arg("--version"))?;
    let cargo = cargo_line
        .split_whitespace()
        .nth(1)
        .unwrap_or("")
        .to_string();
    if cargo.is_empty() {
        return Err("cannot parse `cargo --version` output".to_string());
    }
    Ok(ToolchainInfo { cargo, rustc, llvm, host, cargo_path, rustc_path })
}

pub fn normalize_llvm_version(value: &str) -> String {
    let value = value.trim().trim_start_matches("llvm");
    let mut parts = value.split('.');
    let major = parts.next().unwrap_or("");
    match parts.next() {
        Some(minor) => format!("{major}.{minor}"),
        None if major.len() == 3 => format!("{}.{}", &major[..2], &major[2..]),
        None => major.to_string(),
    }
}

pub fn llvm_compatible(rustc_llvm: &str, rnx_llvm: &str) -> bool {
    normalize_llvm_version(rustc_llvm) == normalize_llvm_version(rnx_llvm)
}

pub fn llvm_fallback_warning(rustc_llvm: &str, rnx_llvm: &str) -> String {
    let rustc_llvm = normalize_llvm_version(rustc_llvm);
    let rnx_llvm = normalize_llvm_version(rnx_llvm);
    let series = rnx_llvm.split('.').next().unwrap_or(&rnx_llvm);
    format!(
        "warning: rustc reports LLVM {rustc_llvm} but rnx uses LLVM {rnx_llvm}.\n  Falling back to a native staticlib/cdylib bridge.\n  Cross-language inlining is disabled.\n  Install a Rust toolchain with LLVM {series}.x to enable bitcode import."
    )
}

pub fn version_tuple(value: &str) -> Option<(u64, u64, u64)> {
    let digits = value.trim().trim_start_matches("rustc ");
    let mut parts = digits.split('.');
    let major = parts.next()?.parse().ok()?;
    let minor = parts.next()?.parse().ok()?;
    let patch = parts.next().unwrap_or("0").split_whitespace().next().unwrap_or("0").parse().ok()?;
    Some((major, minor, patch))
}

pub fn version_key(value: &str) -> (u64, u64, u64) {
    version_tuple(value).unwrap_or((0, 0, 0))
}

pub fn check_msrv(required: Option<&str>, rustc_version: &str) -> Result<(), String> {
    let Some(required) = required else {
        return Ok(());
    };
    let Some(need) = version_tuple(required) else {
        return Err(format!("invalid MSRV `{required}` (expected `major.minor.patch`)"));
    };
    let Some(have) = version_tuple(rustc_version) else {
        return Err(format!("cannot parse rustc version `{rustc_version}`"));
    };
    if have < need {
        return Err(format!(
            "rustc {rustc_version} is too old for this dependency graph (MSRV {required}); upgrade the Rust toolchain"
        ));
    }
    Ok(())
}

pub fn toolchain_fingerprint(toolchain: &ToolchainInfo) -> String {
    format!("{}|{}|{}", toolchain.rustc, toolchain.llvm, toolchain.host)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_llvm_versions() {
        assert_eq!(normalize_llvm_version("22.1.8"), "22.1");
        assert_eq!(normalize_llvm_version("llvm22"), "22");
        assert_eq!(normalize_llvm_version("23.1"), "23.1");
    }

    #[test]
    fn compatible_ignores_patch_version() {
        assert!(llvm_compatible("22.1.8", "22.1"));
        assert!(!llvm_compatible("23.1", "22.1"));
    }

    #[test]
    fn fallback_warning_matches_contract() {
        assert_eq!(
            llvm_fallback_warning("23.1.0", "22.1"),
            "warning: rustc reports LLVM 23.1 but rnx uses LLVM 22.1.\n  Falling back to a native staticlib/cdylib bridge.\n  Cross-language inlining is disabled.\n  Install a Rust toolchain with LLVM 22.x to enable bitcode import."
        );
    }

    #[test]
    fn msrv_gate() {
        assert!(check_msrv(Some("1.80.0"), "1.98.1").is_ok());
        assert!(check_msrv(Some("1.98.2"), "1.98.1").is_err());
        assert!(check_msrv(None, "not-a-version").is_ok());
    }

    #[test]
    fn local_toolchain_is_detected() {
        let Ok(info) = detect_toolchain() else {
            return;
        };
        assert!(!info.rustc.is_empty());
        assert!(!info.cargo.is_empty());
        assert!(!info.llvm.is_empty());
        assert!(!info.host.is_empty());
    }
}
