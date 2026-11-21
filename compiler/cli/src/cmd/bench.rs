use super::*;
use std::path::{Path, PathBuf};

fn artifact_from(stdout: &str) -> Option<PathBuf> {
    for line in stdout.lines() {
        let Some(rest) = line.trim().strip_prefix("artifact: ") else {
            continue;
        };
        let rest = rest.strip_suffix(" (fresh)").unwrap_or(rest);
        let path = match rest.find(" [") {
            Some(i) => &rest[..i],
            None => rest,
        };
        return Some(PathBuf::from(path.trim()));
    }
    None
}

pub(super) fn run_bench(
    package: Option<String>,
    filter: Option<String>,
    jobs: Option<usize>,
    memory_cap: Option<u64>,
    verbose: bool,
) {
    let targets = cli::test_targets(package.as_deref()).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let exe = std::env::current_exe().unwrap_or_else(|e| {
        eprintln!("rnx bench: cannot find the running binary: {e}");
        std::process::exit(1);
    });
    let mut code = 0;
    for (pkg_root, _) in &targets {
        let artifact = match build_harness(&exe, pkg_root, filter.as_deref(), jobs, memory_cap, verbose) {
            Ok(path) => path,
            Err(message) => {
                eprintln!("{message}");
                std::process::exit(1);
            }
        };
        let status = std::process::Command::new(&artifact)
            .current_dir(pkg_root)
            .status()
            .unwrap_or_else(|e| {
                eprintln!("rnx bench: cannot run {}: {e}", artifact.display());
                std::process::exit(1);
            });
        if !status.success() {
            code = 1;
        }
    }
    std::process::exit(code);
}

fn build_harness(
    exe: &Path,
    pkg_root: &Path,
    filter: Option<&str>,
    jobs: Option<usize>,
    memory_cap: Option<u64>,
    verbose: bool,
) -> Result<PathBuf, String> {
    let mut build = std::process::Command::new(exe);
    build
        .arg("build")
        .arg("--release")
        .arg("--bench-harness")
        .arg(filter.unwrap_or(""))
        .current_dir(pkg_root);
    if let Some(j) = jobs {
        build.arg("--jobs").arg(j.to_string());
    }
    if let Some(m) = memory_cap {
        build.arg("--memory-cap").arg(m.to_string());
    }
    let out = build
        .output()
        .map_err(|e| format!("rnx bench: cannot start the compiler: {e}"))?;
    let stdout = String::from_utf8_lossy(&out.stdout);
    let stderr = String::from_utf8_lossy(&out.stderr);
    if !out.status.success() {
        return Err(format!("{stdout}{stderr}").trim_end().to_string());
    }
    if !stderr.trim().is_empty() {
        eprint!("{stderr}");
    }
    let artifact = artifact_from(&stdout)
        .ok_or_else(|| format!("rnx bench: no artifact for {}", pkg_root.display()))?;
    if verbose {
        eprintln!("rnx bench: {}", artifact.display());
    }
    Ok(artifact)
}
