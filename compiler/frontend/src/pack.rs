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

fn normalize_gen_dir(dir: &str) -> String {
    let mut out = dir.replace('\\', "/");
    while out.starts_with("./") {
        out = out[2..].to_string();
    }
    while out.len() > 1 && out.ends_with('/') {
        out.pop();
    }
    out
}

fn generated_dirs(manifest: &Manifest) -> Vec<String> {
    let mut dirs = Vec::new();
    if let Some(project) = manifest.project.as_ref() {
        if let Some(driver) = project.bindgen.as_ref() {
            for dir in driver.outputs.keys() {
                let norm = normalize_gen_dir(dir);
                if norm.is_empty() || norm == "." || norm == "/" {
                    continue;
                }
                dirs.push(norm);
            }
        }
    }
    dirs.sort();
    dirs.dedup();
    dirs
}

fn under_generated_dir(rel: &str, dirs: &[String]) -> bool {
    let flat = rel.replace('\\', "/");
    dirs.iter().any(|d| flat == *d || flat.starts_with(&format!("{d}/")))
}

pub fn bindgen_output_files(package_dir: &Path, manifest: &Manifest) -> Vec<PathBuf> {
    let mut out = Vec::new();
    for dir in generated_dirs(manifest) {
        let root = package_dir.join(&dir);
        if !root.is_dir() {
            continue;
        }
        let mut found = Vec::new();
        if walk_rnx(&root, &mut found).is_err() {
            continue;
        }
        out.extend(found);
    }
    out.sort();
    out.dedup();
    out
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
    let mut raw_dirs = Vec::new();
    if let Some(driver) = project.bindgen.as_ref() {
        raw_dirs.extend(driver.outputs.keys().cloned());
    }
    for raw in &raw_dirs {
        let norm = normalize_gen_dir(raw);
        if norm.is_empty() || norm == "." || norm == "/" || norm == ".." || norm.starts_with("../") {
            return Err(Diagnostic::new(
                Code::E108,
                format!("bindgen output `{raw}` is not a shippable output directory: declare a package-relative directory such as `src/gen`"),
            ));
        }
    }
    let gen_dirs = generated_dirs(manifest);
    let mut entries: Vec<(String, &str)> =
        vec![("main entry".to_string(), project.entries.main.as_str())];
    if let Some(lib) = project.entries.lib.as_deref() {
        entries.push(("lib entry".to_string(), lib));
    }
    for (name, target) in &project.entries.bins {
        entries.push((format!("bin entry `{name}`"), target.as_str()));
    }
    for (label, target) in &entries {
        let hit = gen_dirs.iter().find(|d| {
            let flat = target.replace('\\', "/");
            flat == **d || flat.starts_with(&format!("{d}/"))
        });
        if let Some(dir) = hit {
            return Err(Diagnostic::new(
                Code::E108,
                format!("package {label} `{target}` lives under bindgen output `{dir}`: generated bindings ship as OCI artifacts, never registry chunks (move hand-written sources out of generated directories)"),
            ));
        }
    }
    Ok(())
}

pub fn package_doc_json(package_dir: &Path, manifest: &Manifest) -> Result<String, Diagnostic> {
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx pack needs a [project] manifest".to_string())
    })?;
    let entry = project.main_path(package_dir);
    let extra = bindgen_output_files(package_dir, manifest);
    let graph = crate::modules::ModuleGraph::build_with_extra(&entry, &extra)?;
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
) -> Result<Vec<String>, Diagnostic> {
    validate_for_pack(package_dir, manifest)?;
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx pack needs a [project] manifest".to_string())
    })?;
    let mut rels: BTreeSet<String> = BTreeSet::new();
    let gen_dirs = generated_dirs(manifest);
    let mut push_abs = |abs: &Path| {
        if let Ok(rel) = abs.strip_prefix(package_dir) {
            let text = rel.to_string_lossy().replace('\\', "/");
            if !text.is_empty() && !excluded(&text) && !under_generated_dir(&text, &gen_dirs) {
                rels.insert(text);
            }
        }
    };
    push_abs(&package_dir.join("Project.config"));
    // Project.deplock is consumer-only; never publish it.
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
    Ok(names)
}

pub fn package_file_list(
    package_dir: &Path,
    manifest: &Manifest,
) -> Result<Vec<PackedFile>, Diagnostic> {
    let names = collect_pack_names(package_dir, manifest)?;
    let mut out = Vec::with_capacity(names.len());
    for name in names {
        let size = std::fs::metadata(package_dir.join(&name))
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {name}: {e}")))?
            .len();
        out.push(PackedFile { path: name, size });
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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PackageGuide {
    pub slug: String,
    pub title: String,
    pub source: String,
    pub description: String,
    pub section: Option<String>,
    pub order: Option<String>,
}

fn guide_slug_for_stem(stem: &str) -> Option<String> {
    let mut out = String::new();
    for c in stem.to_ascii_lowercase().chars() {
        if c.is_ascii_alphanumeric() {
            out.push(c);
        } else if !out.is_empty() && !out.ends_with('-') {
            out.push('-');
        }
    }
    while out.ends_with('-') {
        out.pop();
    }
    if out.len() >= 1 && out.len() <= 64 && out.as_bytes()[0] != b'-' {
        Some(out)
    } else {
        None
    }
}

fn split_guide_frontmatter(text: &str) -> (Vec<(String, String)>, &str) {
    let Some(rest) = text.strip_prefix("---\n") else {
        return (Vec::new(), text);
    };
    let mut fields = Vec::new();
    let mut consumed = 0usize;
    for line in rest.split_inclusive('\n') {
        consumed += line.len();
        let trimmed = line.strip_suffix('\n').unwrap_or(line);
        let trimmed = trimmed.strip_suffix('\r').unwrap_or(trimmed);
        if trimmed == "---" {
            let body = rest[consumed..].trim_start_matches(['\n', '\r']);
            return (fields, body);
        }
        if let Some(colon) = trimmed.find(':') {
            let key = trimmed[..colon].trim();
            let mut value = trimmed[colon + 1..].trim().to_string();
            if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
                value = value[1..value.len() - 1].to_string();
            }
            if matches!(key, "title" | "description" | "section" | "order") {
                fields.push((key.to_string(), value));
            }
        }
    }
    (Vec::new(), text)
}

fn guide_field<'a>(fields: &'a [(String, String)], key: &str) -> Option<&'a str> {
    fields.iter().find(|(k, _)| k == key).map(|(_, v)| v.as_str())
}

fn humanize_guide_stem(stem: &str) -> String {
    stem.split(['-', '_', ' '])
        .filter(|w| !w.is_empty())
        .map(|w| {
            let mut chars = w.chars();
            match chars.next() {
                Some(first) => first.to_uppercase().collect::<String>() + chars.as_str(),
                None => String::new(),
            }
        })
        .collect::<Vec<_>>()
        .join(" ")
}

fn guide_title_for_body(body: &str, fields: &[(String, String)], stem: &str) -> String {
    for line in body.lines() {
        if let Some(heading) = line.strip_prefix("# ") {
            let heading = heading.trim();
            if !heading.is_empty() {
                return heading.to_string();
            }
        }
    }
    if let Some(title) = guide_field(fields, "title") {
        let title = title.trim();
        if !title.is_empty() {
            return title.to_string();
        }
    }
    humanize_guide_stem(stem)
}

pub fn package_guides(package_dir: &Path, manifest: &Manifest) -> Result<Vec<PackageGuide>, Diagnostic> {
    let project = manifest.project.as_ref().ok_or_else(|| {
        Diagnostic::new(Code::E108, "rnx pack needs a [project] manifest".to_string())
    })?;
    let docs = project.entries.docs.as_deref().map(|d| d.trim_end_matches('/')).unwrap_or("");
    if docs.is_empty() {
        return Ok(Vec::new());
    }
    let docs_dir = package_dir.join(docs);
    if !docs_dir.is_dir() {
        return Ok(Vec::new());
    }
    let mut found = Vec::new();
    walk_md(&docs_dir, &mut found)?;
    let mut rels = std::collections::BTreeSet::new();
    for abs in &found {
        let rel = abs.strip_prefix(package_dir).unwrap_or(abs).to_string_lossy().replace('\\', "/");
        let depth = rel.strip_prefix(&format!("{docs}/")).unwrap_or(&rel);
        if depth.split('/').count() > 2 {
            return Err(Diagnostic::new(
                Code::E108,
                format!("entries.docs allows at most one nesting level (`{rel}` is too deep)"),
            ));
        }
        rels.insert(rel);
    }
    let mut guides = Vec::new();
    let mut seen = std::collections::BTreeSet::new();
    for rel in &rels {
        let bytes = std::fs::read(package_dir.join(rel)).map_err(|e| {
            Diagnostic::new(Code::E108, format!("cannot read {rel}: {e}"))
        })?;
        let text = String::from_utf8(bytes).map_err(|_| {
            Diagnostic::new(Code::E108, format!("cannot publish {rel}: not valid UTF-8"))
        })?;
        let stem = Path::new(rel)
            .file_stem()
            .map(|s| s.to_string_lossy().into_owned())
            .unwrap_or_default();
        let Some(slug) = guide_slug_for_stem(&stem) else {
            return Err(Diagnostic::new(
                Code::E108,
                format!("guide `{rel}` maps to an unusable slug (name the file with lowercase letters, digits, and dashes, at most 64 characters)"),
            ));
        };
        if !seen.insert(slug.clone()) {
            return Err(Diagnostic::new(
                Code::E108,
                format!("duplicate guide slug `{slug}` (from `{rel}`; rename one of the files sharing the name)"),
            ));
        }
        let (fields, body) = split_guide_frontmatter(&text);
        let title = guide_title_for_body(body, &fields, &stem);
        let opt = |key: &str| {
            guide_field(&fields, key).map(str::trim).filter(|v| !v.is_empty()).map(str::to_string)
        };
        guides.push(PackageGuide {
            slug,
            title,
            source: body.to_string(),
            description: guide_field(&fields, "description").unwrap_or("").to_string(),
            section: opt("section"),
            order: opt("order"),
        });
    }
    guides.sort_by(|a, b| a.slug.cmp(&b.slug));
    Ok(guides)
}

pub fn build_package_tar(
    package_dir: &Path,
    manifest: &Manifest,
) -> Result<Vec<u8>, Diagnostic> {
    let names = collect_pack_names(package_dir, manifest)?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(names.len());
    for n in &names {
        let content = std::fs::read(package_dir.join(n))
            .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {n}: {e}")))?;
        files.push((n.clone(), content));
    }
    assemble_tar(&names, &files)
}

fn assemble_tar(names: &[String], files: &[(String, Vec<u8>)]) -> Result<Vec<u8>, Diagnostic> {
    let mut dirs: BTreeSet<String> = BTreeSet::new();
    for n in names {
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
    for n in names {
        ordered.push((n.clone(), false));
    }
    ordered.sort();
    let mut buf: Vec<u8> = Vec::new();
    {
        let mut tar = TarWriter::new(&mut buf);
        for (rel, is_dir) in &ordered {
            if *is_dir {
                tar.add_dir(rel)?;
            } else if let Some((_, bytes)) = files.iter().find(|(g, _)| g == rel) {
                tar.add_file(rel, bytes)?;
            } else {
                return Err(Diagnostic::new(Code::E108, format!("cannot read {rel}: missing entry")));
            }
        }
        tar.finish()?;
    }
    Ok(buf)
}

#[derive(Debug)]
pub struct MinifiedTar {
    pub tar: Vec<u8>,
    pub files: Vec<(String, Vec<u8>)>,
}

pub fn build_minified_package_tar(
    package_dir: &Path,
    manifest: &Manifest,
) -> Result<MinifiedTar, Diagnostic> {
    let names = collect_pack_names(package_dir, manifest)?;
    let mut files: Vec<(String, Vec<u8>)> = Vec::with_capacity(names.len());
    for n in &names {
        if crate::minify::is_minified_rel(n) {
            let raw = std::fs::read(package_dir.join(n))
                .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {n}: {e}")))?;
            let text = String::from_utf8(raw).map_err(|_| {
                Diagnostic::new(Code::E108, format!("cannot minify {n}: not valid UTF-8"))
            })?;
            let min = crate::minify::verify_and_minify(n, &text)?;
            files.push((n.clone(), min.into_bytes()));
        } else {
            let content = std::fs::read(package_dir.join(n))
                .map_err(|e| Diagnostic::new(Code::E108, format!("cannot read {n}: {e}")))?;
            files.push((n.clone(), content));
        }
    }
    let tar = assemble_tar(&names, &files)?;
    Ok(MinifiedTar { tar, files })
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

pub fn pack_package_gz_minified(
    package_dir: &Path,
    manifest: &Manifest,
    out_dir: &Path,
) -> Result<(PathBuf, PathBuf, Vec<(String, u64)>), Diagnostic> {
    use crate::gzip;
    let stem = package_stem(manifest)?;
    let built = build_minified_package_tar(package_dir, manifest)?;
    let gz = gzip::compress_gzip(&built.tar);
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
    let sizes = built
        .files
        .iter()
        .map(|(n, b)| (n.clone(), b.len() as u64))
        .collect();
    Ok((gz_path, sha_path, sizes))
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

    fn fixture_dir(tag: &str, readme: Option<(&str, &str)>) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("pack-{tag}"));
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

    fn fixture_bindgen_dir(tag: &str, outputs: &str) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("pack-{tag}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            format!(
                "export default {{\n    project: {{\n        name: \"probe\",\n        version: \"1.2.3\"\n    }},\n    bindgen: {{\n        headers: [\"build/foo.h\"],\n        native_lib: \"foo\",\n        outputs: {{\n{outputs}\n        }}\n    }}\n}}\n"
            ),
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        dir
    }

    #[test]
    fn file_list_matches_tar_contents() {
        let dir = fixture_dir("files", Some(("README.md", "# probe\n")));
        let manifest = fixture_manifest(&dir);
        let files = package_file_list(&dir, &manifest).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, vec!["Project.config", "README.md", "src/main.rnx"]);
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
    }

    #[test]
    fn deplock_stays_on_disk_but_out_of_bundle() {
        let dir = fixture_dir("deplock", Some(("README.md", "# probe\n")));
        std::fs::write(dir.join("Project.deplock"), "export default {}\n").unwrap();
        let manifest = fixture_manifest(&dir);
        let files = package_file_list(&dir, &manifest).unwrap();
        assert!(!files.iter().any(|f| f.path == "Project.deplock"));
        let tar = build_package_tar(&dir, &manifest).unwrap();
        assert!(!String::from_utf8_lossy(&tar).contains("Project.deplock"));
        assert!(dir.join("Project.deplock").is_file());
    }

    #[test]
    fn readme_prefers_md_then_plain_then_default() {        let dir = fixture_dir("readme-md", Some(("README.md", "# md\n")));
        assert_eq!(package_readme_text(&dir, "probe"), "# md\n");
        let dir = fixture_dir("readme-plain", Some(("README", "plain\n")));
        assert_eq!(package_readme_text(&dir, "probe"), "plain\n");
        let dir = fixture_dir("readme-none", None);
        assert_eq!(package_readme_text(&dir, "probe"), "# probe\n");
    }

    fn fixture_guides_dir(tag: &str, files: &[(&str, &str)]) -> crate::scratch::Scratch {
        let dir = crate::scratch::dir(&format!("pack-guides-{tag}"));
        std::fs::create_dir_all(dir.join("src")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\"\n    },\n    entries: {\n        main: \"src/main.rnx\",\n        docs: \"docs\"\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main(): Int { return 0; }\n").unwrap();
        for (rel, text) in files {
            let path = dir.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(&path, text).unwrap();
        }
        dir
    }

    #[test]
    fn guides_empty_without_docs_entry() {
        let dir = fixture_dir("guides-none", None);
        let manifest = fixture_manifest(&dir);
        assert_eq!(package_guides(&dir, &manifest).unwrap(), Vec::new());
    }

    #[test]
    fn guides_flow_for_a_real_std_library() {
        let std_bytes =
            std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../stdlib/src/bytes");
        assert!(std_bytes.join("lib.rnx").is_file());
        let project = crate::project::ProjectConfig {
            name: "@std/bytes".to_string(),
            version: "0.1.0".to_string(),
            description: String::new(),
            engine: String::new(),
            entries: crate::project::Entries {
                main: "src/lib.rnx".to_string(),
                lib: None,
                docs: Some("docs".to_string()),
                bins: Default::default(),
            },
            registry: None,
            registries: Default::default(),
            dependencies: Default::default(),
            permissions: None,
            keywords: Vec::new(),
            artifacts: None,
            license: None,
            native_pins: Default::default(),
            tasks: Default::default(),
            bindgen: None,
        };
        let manifest = Manifest { project: Some(project), workspace: None };
        let guides = package_guides(&std_bytes, &manifest).unwrap();
        let slugs: Vec<&str> = guides.iter().map(|g| g.slug.as_str()).collect();
        assert_eq!(slugs, vec!["getting-started", "overview"]);
        assert_eq!(guides[0].title, "Getting started with @std/bytes");
        assert_eq!(guides[1].title, "@std/bytes");
        assert!(guides.iter().all(|g| g.source.starts_with("# ")));
        assert!(guides[0].source.contains("```rnx"));
    }

    #[test]
    fn guides_derive_slug_title_and_sort() {
        let dir = fixture_guides_dir(
            "sort",
            &[
                ("docs/zebra.md", "# Zebra\n\nStripes.\n"),
                ("docs/apple.md", "# Apple\n\nFruit.\n"),
                ("docs/sub/nested.md", "# Nested\n\nOne level down.\n"),
            ],
        );
        let manifest = fixture_manifest(&dir);
        let guides = package_guides(&dir, &manifest).unwrap();
        let slugs: Vec<&str> = guides.iter().map(|g| g.slug.as_str()).collect();
        assert_eq!(slugs, vec!["apple", "nested", "zebra"]);
        assert_eq!(guides[0].title, "Apple");
        assert_eq!(guides[0].source, "# Apple\n\nFruit.\n");
        assert_eq!(guides[0].description, "");
        assert_eq!(guides[0].section, None);
        assert_eq!(guides[0].order, None);
    }

    #[test]
    fn guides_title_prefers_heading_then_frontmatter_then_filename() {
        let dir = fixture_guides_dir(
            "titles",
            &[
                ("docs/aaa.md", "---\ntitle: \"Frontmatter\"\n---\n\n# Heading\n"),
                ("docs/bbb.md", "---\ntitle: \"Frontmatter\"\n---\n\nNo heading here.\n"),
                ("docs/my_quick-start.md", "Body without heading or frontmatter.\n"),
            ],
        );
        let manifest = fixture_manifest(&dir);
        let guides = package_guides(&dir, &manifest).unwrap();
        let by_slug = |slug: &str| guides.iter().find(|g| g.slug == slug).unwrap().title.clone();
        assert_eq!(by_slug("aaa"), "Heading");
        assert_eq!(by_slug("bbb"), "Frontmatter");
        assert_eq!(by_slug("my-quick-start"), "My Quick Start");
        let front = guides.iter().find(|g| g.slug == "bbb").unwrap();
        assert_eq!(front.source, "No heading here.\n");
        assert_eq!(front.description, "");
    }

    #[test]
    fn guides_frontmatter_carries_description_section_order() {
        let dir = fixture_guides_dir(
            "meta",
            &[(
                "docs/start.md",
                "---\ntitle: \"Start\"\ndescription: \"Log in first.\"\nsection: \"Basics\"\norder: \"2\"\nicon: \"Rocket\"\n---\n\n# Start\n",
            )],
        );
        let manifest = fixture_manifest(&dir);
        let guides = package_guides(&dir, &manifest).unwrap();
        assert_eq!(guides.len(), 1);
        assert_eq!(guides[0].description, "Log in first.");
        assert_eq!(guides[0].section.as_deref(), Some("Basics"));
        assert_eq!(guides[0].order.as_deref(), Some("2"));
        assert!(!guides[0].source.contains("icon"), "{}", guides[0].source);
    }

    #[test]
    fn guides_slug_mapping_folds_punctuation() {
        assert_eq!(guide_slug_for_stem("Hello World!").as_deref(), Some("hello-world"));
        assert_eq!(guide_slug_for_stem("a--b__c").as_deref(), Some("a-b-c"));
        assert_eq!(guide_slug_for_stem("UPPER").as_deref(), Some("upper"));
        assert_eq!(guide_slug_for_stem("---a---").as_deref(), Some("a"));
        assert_eq!(guide_slug_for_stem("!!!"), None);
        assert_eq!(guide_slug_for_stem(""), None);
        assert_eq!(guide_slug_for_stem(&"a".repeat(64)).unwrap().len(), 64);
        assert_eq!(guide_slug_for_stem(&"a".repeat(65)), None);
    }

    #[test]
    fn guides_reject_unmappable_names_duplicates_depth_and_bytes() {
        let dir = fixture_guides_dir("bad-slug", &[("docs/!!!.md", "# Nope\n")]);
        let manifest = fixture_manifest(&dir);
        let err = package_guides(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("docs/!!!.md"), "{}", err.message);
        let dir = fixture_guides_dir(
            "dup-slug",
            &[("docs/a b.md", "# One\n"), ("docs/sub/a-b.md", "# Two\n")],
        );
        let manifest = fixture_manifest(&dir);
        let err = package_guides(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("duplicate guide slug `a-b`"), "{}", err.message);
        let dir = fixture_guides_dir("too-deep", &[("docs/a/b/c.md", "# Deep\n")]);
        let manifest = fixture_manifest(&dir);
        let err = package_guides(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("one nesting level"), "{}", err.message);
        let dir = fixture_guides_dir("nonutf", &[("docs/blob.md", "# Blob\n")]);
        std::fs::write(dir.join("docs").join("blob.md"), [0xff, 0xfe, 0x00]).unwrap();
        let manifest = fixture_manifest(&dir);
        let err = package_guides(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("not valid UTF-8"), "{}", err.message);
    }

    #[test]
    fn minified_tar_minifies_rnx_but_not_config() {
        let dir = fixture_dir("min", Some(("README.md", "# probe\n")));
        std::fs::write(
            dir.join("src").join("main.rnx"),
            "// plain comment\n/** Doc for Main. */\nfn Main(): Int {\n    // inside\n    return 0;\n}\n",
        )
        .unwrap();
        let manifest = fixture_manifest(&dir);
        let formatted_config = std::fs::read_to_string(dir.join("Project.config")).unwrap();
        assert_eq!(
            formatted_config,
            crate::project::format_manifest_text(&formatted_config).unwrap()
        );
        let built = build_minified_package_tar(&dir, &manifest).unwrap();
        let by_name = |name: &str| {
            built.files.iter().find(|(n, _)| n == name).unwrap().1.clone()
        };
        let main = String::from_utf8(by_name("src/main.rnx")).unwrap();
        assert_eq!(main, "fn Main():Int{return 0;}");
        assert!(!main.contains("comment"), "{main}");
        assert!(!main.contains("Doc for Main"), "{main}");
        let config = by_name("Project.config");
        assert_eq!(config, formatted_config.as_bytes());
        assert!(config.windows(2).any(|w| w == b"\n "), "config lost formatting");
        let plain = build_package_tar(&dir, &manifest).unwrap();
        assert!(String::from_utf8_lossy(&plain).contains("// plain comment"));
    }

    #[test]
    fn minified_tar_refuses_unformatted_rnx() {
        let dir = fixture_dir("minfmt", None);
        std::fs::write(dir.join("src").join("main.rnx"), "fn Main():Int{return 0;}\n").unwrap();
        let manifest = fixture_manifest(&dir);
        let err = build_minified_package_tar(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("src/main.rnx"), "{}", err.message);
    }

    #[test]
    fn doc_snapshot_covers_declared_bindgen_outputs() {
        let (_guard, _cache) = crate::fetch::testkit::isolate_cache("gendocs");
        let pin = env!("CARGO_PKG_VERSION");
        crate::fetch::testkit::plant_store_package(
            "@std/prelude",
            pin,
            &"ef".repeat(32),
            "",
            "export fn hello(): Int { return 1; }\n",
        );
        let dir = fixture_bindgen_dir("gendocs", "            \"src/gen\": { title: \"sqlite3.rnx\" }");
        std::fs::create_dir_all(dir.join("src").join("gen")).unwrap();
        std::fs::write(
            dir.join("src").join("gen").join("sqlite3.rnx"),
            "//! Generated by rnx-bindgen 0.1.0 from sqlite3.h (upstream=test@sha256:abc).\n/** Open a handle. */\nunsafe fn sqlite3Open(): Int {\n    return 0;\n}\n\n/** Result code. */\nexport const SQLITE_OK: Int = 0\n\nexport {\n    sqlite3Open\n}\n",
        )
        .unwrap();
        let manifest = fixture_manifest(&dir);
        let json = package_doc_json(&dir, &manifest).unwrap();
        assert!(json.contains("Open a handle."), "{json}");
        assert!(json.contains("Result code."), "{json}");
        assert!(json.contains("sqlite3Open"), "{json}");
    }

    #[test]
    fn declared_bindgen_outputs_stay_out_of_chunks() {
        let dir = fixture_bindgen_dir(
            "gen",
            "            \"src/gen\": { title: \"sqlite3.rnx\" },\n            \"extra/out/\": { title: \"extra.rnx\" }",
        );
        std::fs::create_dir_all(dir.join("src").join("gen")).unwrap();
        std::fs::write(dir.join("src").join("gen").join("sqlite3.rnx"), "generated\n").unwrap();
        std::fs::write(dir.join("src").join("gen").join("stale.rnx"), "stale rename leftover\n").unwrap();
        std::fs::write(dir.join("src").join("gen").join("bindgen.oci.json"), "{}\n").unwrap();
        std::fs::create_dir_all(dir.join("extra").join("out")).unwrap();
        std::fs::write(dir.join("extra").join("out").join("extra.rnx"), "generated\n").unwrap();
        let manifest = fixture_manifest(&dir);
        let files = package_file_list(&dir, &manifest).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(paths.contains(&"Project.config"), "{paths:?}");
        assert!(paths.contains(&"src/main.rnx"), "{paths:?}");
        assert!(!paths.iter().any(|p| p.starts_with("src/gen")), "{paths:?}");
        assert!(!paths.iter().any(|p| p.starts_with("extra/out")), "{paths:?}");
        let tar = build_package_tar(&dir, &manifest).unwrap();
        let text = String::from_utf8_lossy(&tar);
        assert!(text.contains("src/main.rnx"), "{paths:?}");
        for leaked in ["src/gen/sqlite3.rnx", "src/gen/stale.rnx", "extra/out/extra.rnx"] {
            assert!(!text.contains(leaked), "generated file leaked into tar: {leaked}");
        }
    }

    #[test]
    fn gen_dir_without_driver_still_ships() {
        let dir = fixture_dir("nogendriver", None);
        std::fs::create_dir_all(dir.join("src").join("gen")).unwrap();
        std::fs::write(
            dir.join("src").join("gen").join("hand.rnx"),
            "fn Main(): Int { return 0; }\n",
        )
        .unwrap();
        let manifest = fixture_manifest(&dir);
        let files = package_file_list(&dir, &manifest).unwrap();
        assert!(files.iter().any(|f| f.path == "src/gen/hand.rnx"));
    }

    #[test]
    fn entry_inside_generated_dir_fails_closed() {
        let dir = fixture_bindgen_dir("genentry", "            \"src/gen\": { title: \"foo.rnx\" }");
        std::fs::create_dir_all(dir.join("src").join("gen")).unwrap();
        std::fs::write(
            dir.join("Project.config"),
            "export default {\n    project: {\n        name: \"probe\",\n        version: \"1.2.3\"\n    },\n    entries: {\n        main: \"src/gen/main.rnx\"\n    },\n    bindgen: {\n        headers: [\"build/foo.h\"],\n        native_lib: \"foo\",\n        outputs: {\n            \"src/gen\": { title: \"foo.rnx\" }\n        }\n    }\n}\n",
        )
        .unwrap();
        std::fs::write(dir.join("src").join("gen").join("main.rnx"), "fn Main(): Int { return 0; }\n")
            .unwrap();
        let manifest = fixture_manifest(&dir);
        let err = package_file_list(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("src/gen/main.rnx"), "{}", err.message);
        assert!(err.message.contains("OCI"), "{}", err.message);
    }

    #[test]
    fn degenerate_bindgen_output_dir_fails_closed() {
        let dir = fixture_bindgen_dir("gendot", "            \".\": { title: \"foo.rnx\" }");
        let manifest = fixture_manifest(&dir);
        let err = package_file_list(&dir, &manifest).unwrap_err();
        assert_eq!(err.code, diagnostics::Code::E108);
        assert!(err.message.contains("output directory"), "{}", err.message);
    }
}

