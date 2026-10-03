use crate::project::{self, ProjectConfig};
use diagnostics::{Code, Diagnostic};
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

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
    let dir = fetch_git_dependency(name, git_url, rev, &anchor.join(".rnx").join("cache").join("git"))?;
    std::fs::canonicalize(&dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read `{}`: {e}", dir.display())))
}

fn excluded_dir(name: &str) -> bool {
    matches!(name, ".git" | "target" | "tests" | ".rnx")
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn short_rev_sanitizes() {
        assert_eq!(short_rev("v0.1.0"), "v0.1.0");
        assert_eq!(short_rev("abc123def456789"), "abc123def456");
        assert_eq!(short_rev("///"), "rev");
    }
}
