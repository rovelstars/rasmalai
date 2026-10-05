use crate::checksum;
use crate::project::Manifest;
use crate::tar::TarWriter;
use diagnostics::{Code, Diagnostic};
use std::collections::BTreeSet;
use std::path::{Path, PathBuf};

fn walk_rnx(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Diagnostic> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read {}: {e}", dir.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk_rnx(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "rnx") {
            out.push(path);
        }
    }
    Ok(())
}

fn walk_md(dir: &Path, out: &mut Vec<PathBuf>) -> Result<(), Diagnostic> {
    let entries = std::fs::read_dir(dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {}: {e}", dir.display())))?;
    for entry in entries {
        let entry = entry.map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read {}: {e}", dir.display()))
        })?;
        let path = entry.path();
        if path.is_dir() {
            walk_md(&path, out)?;
        } else if path.extension().is_some_and(|e| e == "md") {
            out.push(path);
        }
    }
    Ok(())
}

fn excluded(rel: &str) -> bool {
    rel == "target"
        || rel.starts_with("target/")
        || rel == ".git"
        || rel.starts_with(".git/")
        || rel == ".rnx"
        || rel.starts_with(".rnx/")
        || rel == ".rnx-cache"
        || rel.starts_with(".rnx-cache/")
        || rel == "tests"
        || rel.starts_with("tests/")
}

fn sanitize(name: &str) -> String {
    name.chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() || c == '_' || c == '-' || c == '.' {
                c
            } else {
                '_'
            }
        })
        .collect()
}

pub fn package_stem(manifest: &Manifest) -> Result<String, Diagnostic> {
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx pack needs a [project] manifest".to_string())
    })?;
    Ok(format!("{}-{}", sanitize(&project.name), sanitize(&project.version)))
}

fn is_pack_name(name: &str) -> bool {
    if name.starts_with('@') {
        return crate::project::is_package_name(name);
    }
    !name.is_empty()
        && name.bytes().all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-')
}

fn is_pack_version(version: &str) -> bool {
    let (core, pre) = match version.split_once('-') {
        Some((c, p)) => (c, Some(p)),
        None => (version, None),
    };
    let parts: Vec<&str> = core.split('.').collect();
    if parts.len() != 3
        || !parts.iter().all(|p| !p.is_empty() && p.bytes().all(|b| b.is_ascii_digit()))
    {
        return false;
    }
    match pre {
        None => true,
        Some(p) => {
            !p.is_empty()
                && p.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
        }
    }
}

pub fn validate_for_pack(
    package_dir: &Path,
    manifest: &Manifest,
) -> Result<(), Diagnostic> {
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx pack needs a [project] manifest".to_string())
    })?;
    if !is_pack_name(&project.name) {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "invalid package name `{}`: use lowercase alphanumeric characters and hyphens",
                project.name
            ),
        ));
    }
    if !is_pack_version(&project.version) {
        return Err(Diagnostic::new(
            Code::E108,
            format!(
                "invalid package version `{}`: use SemVer `X.Y.Z`",
                project.version
            ),
        ));
    }
    let entry = package_dir.join(&project.entries.main);
    if !entry.is_file() {
        return Err(Diagnostic::new(
            Code::E108,
            format!("package entry `{}` does not exist", project.entries.main),
        ));
    }
    Ok(())
}

pub fn package_doc_json(package_dir: &Path, manifest: &Manifest) -> Result<String, Diagnostic> {
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx pack needs a [project] manifest".to_string())
    })?;
    let entry = project.main_path(package_dir);
    let graph = crate::modules::ModuleGraph::build(&entry)?;
    let mut docs = Vec::new();
    for f in &graph.files {
        if f.path.extension().is_some_and(|e| e == "rnx") && f.path.starts_with(package_dir) {
            let modname =
                if f.key.is_empty() { project.name.clone() } else { f.key.clone() };
            docs.push(crate::doc::collect_module(&modname, &f.module, false));
        }
    }
    docs.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(crate::doc::modules_to_json(&docs))
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackedFile {
    pub path: String,
    pub size: u64,
}

fn collect_pack_names(
    package_dir: &Path,
    manifest: &Manifest,
) -> Result<(Vec<String>, Vec<(String, Vec<u8>)>), Diagnostic> {
    validate_for_pack(package_dir, manifest)?;
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx pack needs a [project] manifest".to_string())
    })?;
    let mut rels: BTreeSet<String> = BTreeSet::new();
    let mut push_abs = |abs: &Path| {
        if let Ok(rel) = abs.strip_prefix(package_dir) {
            let text = rel.to_string_lossy().replace('\\', "/");
            if !text.is_empty() && !excluded(&text) {
                rels.insert(text);
            }
        }
    };
    push_abs(&package_dir.join("Project.config"));
    let deplock = package_dir.join("Project.deplock");
    if deplock.is_file() {
        push_abs(&deplock);
    }
    let entry = package_dir.join(&project.entries.main);
    if entry.is_file() {
        push_abs(&entry);
    }
    if let Some(lib) = project.entries.lib.as_deref() {
        let path = package_dir.join(lib);
        if path.is_file() {
            push_abs(&path);
        }
    }
    for target in project.entries.bins.values() {
        let path = package_dir.join(target);
        if path.is_file() {
            push_abs(&path);
        }
    }
    let src_dir = package_dir.join("src");
    if src_dir.is_dir() {
        let mut found = Vec::new();
        walk_rnx(&src_dir, &mut found)?;
        for f in found {
            push_abs(&f);
        }
    }
    if let Some(docs) = project.entries.docs.as_deref() {
        let docs_dir = package_dir.join(docs);
        if docs_dir.is_dir() {
            let mut found = Vec::new();
            walk_md(&docs_dir, &mut found)?;
            for f in found {
                push_abs(&f);
            }
        }
    }
    for name in ["README", "README.md", "LICENSE", "LICENSE.txt"] {
        let p = package_dir.join(name);
        if p.is_file() {
            push_abs(&p);
        }
    }
    let mut names: Vec<String> = rels.into_iter().collect();
    names.sort();
    let mut generated: Vec<(String, Vec<u8>)> = Vec::new();
    match package_doc_json(package_dir, manifest) {
        Ok(json) => generated.push((".rnx/doc.json".to_string(), json.into_bytes())),
        Err(e) => {
            return Err(Diagnostic::new(
                Code::E108,
                format!("cannot extract package docs: {}", e.message),
            ));
        }
    }
    for (rel, _) in &generated {
        names.push(rel.clone());
    }
    names.sort();
    Ok((names, generated))
}

pub fn package_file_list(
    package_dir: &Path,
    manifest: &Manifest,
) -> Result<Vec<PackedFile>, Diagnostic> {
    let (names, generated) = collect_pack_names(package_dir, manifest)?;
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        if let Some((_, bytes)) = generated.iter().find(|(g, _)| g == &name) {
            out.push(PackedFile { path: name, size: bytes.len() as u64 });
        } else {
            let size = std::fs::metadata(package_dir.join(&name))
                .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {name}: {e}")))?
                .len();
            out.push(PackedFile { path: name, size });
        }
    }
    out.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(out)
}

pub fn package_readme_text(package_dir: &Path, fallback_name: &str) -> String {
    for name in ["README.md", "README"] {
        if let Ok(text) = std::fs::read_to_string(package_dir.join(name)) {
            return text;
        }
    }
    format!("# {fallback_name}\n")
}

pub fn build_package_tar(
    package_dir: &Path,
    manifest: &Manifest,
) -> Result<Vec<u8>, Diagnostic> {
    let (names, generated) = collect_pack_names(package_dir, manifest)?;
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    for n in &names {
        let mut prefix = String::new();
        for part in n.split('/').take(n.split('/').count() - 1) {
            if !prefix.is_empty() {
                prefix.push('/');
            }
            prefix.push_str(part);
            dirs.insert(prefix.clone());
        }
    }
    let mut ordered: Vec<(String, bool)> = Vec::new();
    for d in &dirs {
        ordered.push((format!("{d}/"), true));
    }
    for n in &names {
        ordered.push((n.clone(), false));
    }
    ordered.sort();
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut tar = TarWriter::new(&mut buf);
        for (rel, is_dir) in &ordered {
            if *is_dir {
                tar.add_dir(rel)?;
            } else if let Some((_, bytes)) = generated.iter().find(|(g, _)| g == rel) {
                tar.add_file(rel, bytes)?;
            } else {
                let content = std::fs::read(package_dir.join(rel)).map_err(|e| {
                    Diagnostic::new(Code::E108, format!("cannot read {rel}: {e}"))
                })?;
                tar.add_file(rel, &content)?;
            }
        }
        tar.finish()?;
    }
    Ok(buf)
}

pub fn pack_package(
    package_dir: &Path,
    manifest: &Manifest,
    out_dir: &Path,
) -> Result<(PathBuf, PathBuf), Diagnostic> {
    let stem = package_stem(manifest)?;
    let buf = build_package_tar(package_dir, manifest)?;
    std::fs::create_dir_all(out_dir).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write {}: {e}", out_dir.display()))
    })?;
    let tar_path = out_dir.join(format!("{stem}.tar"));
    std::fs::write(&tar_path, &buf).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write {}: {e}", tar_path.display()))
    })?;
    let digest = checksum::Sha256::hexdigest(&buf);
    let sha_path = out_dir.join(format!("{stem}.sha256"));
    let line = format!("{digest}  {stem}.tar\n");
    std::fs::write(&sha_path, line).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write {}: {e}", sha_path.display()))
    })?;
    Ok((tar_path, sha_path))
}

pub fn pack_package_gz(
    package_dir: &Path,
    manifest: &Manifest,
    out_dir: &Path,
) -> Result<(PathBuf, PathBuf), Diagnostic> {
    use crate::gzip;
    let stem = package_stem(manifest)?;
    let tar = build_package_tar(package_dir, manifest)?;
    let gz = gzip::compress_gzip(&tar);
    std::fs::create_dir_all(out_dir).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write {}: {e}", out_dir.display()))
    })?;
    let gz_path = out_dir.join(format!("{stem}.tar.gz"));
    std::fs::write(&gz_path, &gz).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write {}: {e}", gz_path.display()))
    })?;
    let digest = checksum::Sha256::hexdigest(&gz);
    let sha_path = out_dir.join(format!("{stem}.tar.gz.sha256"));
    let line = format!("{digest}  {stem}.tar.gz\n");
    std::fs::write(&sha_path, line).map_err(|e| {
        Diagnostic::new(Code::E108, format!("cannot write {}: {e}", sha_path.display()))
    })?;
    Ok((gz_path, sha_path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scoped_names_follow_package_rules() {
        assert!(is_pack_name("@acme/widget"));
        assert!(is_pack_name("@a0/b-c9"));
        assert!(!is_pack_name("@Acme/widget"));
        assert!(!is_pack_name("@/widget"));
        assert!(!is_pack_name("@acme/"));
        assert!(!is_pack_name("@acme"));
        assert!(!is_pack_name("@acme/widget/extra"));
    }

    #[test]
    fn unscoped_names_keep_prior_rules() {
        assert!(is_pack_name("widget"));
        assert!(is_pack_name("my-pkg-2"));
        assert!(!is_pack_name(""));
        assert!(!is_pack_name("MyPkg"));
        assert!(!is_pack_name("my_pkg"));
        assert!(!is_pack_name("my pkg"));
        assert!(!is_pack_name("my/pkg"));
    }

    #[test]
    fn prerelease_versions_accepted() {
        assert!(is_pack_version("1.2.3"));
        assert!(is_pack_version("0.0.0"));
        assert!(is_pack_version("1.2.3-alpha"));
        assert!(is_pack_version("1.2.3-rc-1"));
        assert!(is_pack_version("10.20.30-beta.1"));
    }

    #[test]
    fn malformed_versions_rejected() {
        assert!(!is_pack_version(""));
        assert!(!is_pack_version("1.2"));
        assert!(!is_pack_version("1.2.3.4"));
        assert!(!is_pack_version("1.2.x"));
        assert!(!is_pack_version("v1.2.3"));
        assert!(!is_pack_version("1.2.3-"));
        assert!(!is_pack_version("1.2.3-***"));
        assert!(!is_pack_version("1.2.3-alpha_beta"));
    }

    fn fixture_dir(tag: &str, readme: Option<(&str, &str)>) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rnx-pack-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        if let Some((name, text)) = readme {
            std::fs::write(dir.join(name), text).unwrap();
        }
        dir
    }

    fn fixture_manifest(dir: &Path) -> Manifest {
        let project =
            crate::project::ProjectConfig::load_from_dir(dir).unwrap().expect("manifest");
        Manifest { project: Some(project), workspace: None }
    }

    #[test]
    fn file_list_matches_tar_contents() {
        let dir = fixture_dir("files", Some(("README.md", "# probe\n")));
        let manifest = fixture_manifest(&dir);
        let files = package_file_list(&dir, &manifest).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec![".rnx/doc.json", "Project.config", "README.md", "src/main.rnx"]);
        for f in &files {
            assert!(!f.path.contains('\\'), "{}", f.path);
            if f.path == "README.md" {
                assert_eq!(f.size, "# probe\n".len() as u64);
            }
            if f.path == "src/main.rnx" {
                assert_eq!(
                    f.size,
                    std::fs::metadata(dir.join(&f.path)).unwrap().len()
                );
            }
        }
        let tar = build_package_tar(&dir, &manifest).unwrap();
        let text = String::from_utf8_lossy(&tar);
        for f in &files {
            assert!(text.contains(f.path.as_str()), "tar misses {}", f.path);
        }
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn readme_prefers_md_then_plain_then_default() {
        let dir = fixture_dir("readme-md", Some(("README.md", "# md\n")));
        assert_eq!(package_readme_text(&dir, "probe"), "# md\n");
        let _ = std::fs::remove_dir_all(&dir);
        let dir = fixture_dir("readme-plain", Some(("README", "plain\n")));
        assert_eq!(package_readme_text(&dir, "probe"), "plain\n");
        let _ = std::fs::remove_dir_all(&dir);
        let dir = fixture_dir("readme-none", None);
        assert_eq!(package_readme_text(&dir, "probe"), "# probe\n");
        let _ = std::fs::remove_dir_all(&dir);
    }
}

