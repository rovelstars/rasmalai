use std::path::{Path, PathBuf};
use std::process::Command;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TargetTriple(pub String);

#[derive(Clone, Debug)]
pub enum LinkInput {
    ObjectBytes(Vec<u8>),
    ObjectPath(PathBuf),
    ArchiveBytes(Vec<u8>),
    ArchiveRef(&'static [u8]),
    ArchivePath(PathBuf),
    Lib(String),
}

#[derive(Debug)]
pub enum LinkError {
    UnsupportedTarget(String),
    NoLinker(String),
    Io(String),
    LinkFailed(String),
    Native(String),
}

impl std::fmt::Display for LinkError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            LinkError::UnsupportedTarget(t) => write!(f, "linker: unsupported target `{t}`"),
            LinkError::NoLinker(m) => write!(f, "linker: {m}"),
            LinkError::Io(m) => write!(f, "linker io: {m}"),
            LinkError::LinkFailed(m) => write!(f, "link failed: {m}"),
            LinkError::Native(m) => write!(f, "linker: {m}"),
        }
    }
}

impl std::error::Error for LinkError {}

pub mod core;
pub mod native;
pub mod target;

pub fn host_triple() -> TargetTriple {
    TargetTriple(format!("{}-unknown-linux-gnu", std::env::consts::ARCH))
}

pub fn link_executable(
    inputs: &[LinkInput],
    output_path: &Path,
    target: &TargetTriple,
    release: bool,
) -> Result<(), LinkError> {
    if !target.0.contains("linux") {
        return Err(LinkError::UnsupportedTarget(target.0.clone()));
    }
    let dir = std::env::temp_dir().join(format!("rnx-link-{}-{}", std::process::id(), output_name(output_path)));
    std::fs::create_dir_all(&dir).map_err(|e| LinkError::Io(e.to_string()))?;
    let staged = stage_inputs(inputs, &dir)?;
    let (driver, ld_flag) = pick_driver()?;
    let mut cmd = Command::new(&driver);
    if let Some(flag) = ld_flag {
        cmd.arg(flag);
    }
    if release {
        cmd.arg("-Wl,--gc-sections");
        cmd.arg("-s");
    }
    for s in &staged {
        cmd.arg(s);
    }
    for lib in ["pthread", "dl", "m", "c"] {
        cmd.arg(format!("-l{lib}"));
    }
    cmd.arg("-o").arg(output_path);
    let out = cmd.output().map_err(|e| LinkError::Io(e.to_string()))?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return Err(LinkError::LinkFailed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

/// Dynamic-link parameters for dev-mode builds: link the user object
/// against an on-disk `libruntime_native.so` instead of the embedded
/// static archive, so `ld` skips archive extraction and relocation
/// parsing entirely. Release builds never use this path.
#[derive(Clone, Debug)]
pub struct DynLink {
    pub lib_dir: PathBuf,
    pub libs: Vec<String>,
}

/// Locate a directory holding `libruntime_native.so`: `$RNX_RUNTIME_DIR`,
/// the `rnx` binary's own directory, or its parent's directory (covers
/// test binaries under `target/<profile>/deps/`). `None` means dev falls
/// back to the static link.
pub fn find_shared_runtime_dir() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("RNX_RUNTIME_DIR") {
        let p = PathBuf::from(dir);
        if p.join("libruntime_native.so").exists() {
            return Some(p);
        }
    }
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let mut cands = vec![exe_dir.to_path_buf()];
    if exe_dir.file_name().map(|n| n == "deps").unwrap_or(false) {
        if let Some(p) = exe_dir.parent() {
            cands.push(p.to_path_buf());
        }
    }
    cands
        .into_iter()
        .find(|d| d.join("libruntime_native.so").exists())
}

pub fn link_executable_dynamic(
    objects: &[LinkInput],
    output_path: &Path,
    target: &TargetTriple,
    dyn_link: &DynLink,
) -> Result<(), LinkError> {
    if !target.0.contains("linux") {
        return Err(LinkError::UnsupportedTarget(target.0.clone()));
    }
    let dir = std::env::temp_dir().join(format!("rnx-link-{}-{}", std::process::id(), output_name(output_path)));
    std::fs::create_dir_all(&dir).map_err(|e| LinkError::Io(e.to_string()))?;
    let staged = stage_inputs(objects, &dir)?;
    let (driver, ld_flag) = pick_driver()?;
    let mut cmd = Command::new(&driver);
    if let Some(flag) = ld_flag {
        cmd.arg(flag);
    }
    cmd.arg("-O0");
    cmd.arg("-Wl,--build-id=none");
    for s in &staged {
        cmd.arg(s);
    }
    cmd.arg(format!("-L{}", dyn_link.lib_dir.display()));
    for lib in &dyn_link.libs {
        cmd.arg(format!("-l{lib}"));
    }
    cmd.arg(format!("-Wl,-rpath,{}", dyn_link.lib_dir.display()));
    for lib in ["pthread", "dl", "m", "c"] {
        cmd.arg(format!("-l{lib}"));
    }
    cmd.arg("-o").arg(output_path);
    let out = cmd.output().map_err(|e| LinkError::Io(e.to_string()))?;
    let _ = std::fs::remove_dir_all(&dir);
    if !out.status.success() {
        return Err(LinkError::LinkFailed(
            String::from_utf8_lossy(&out.stderr).trim().to_string(),
        ));
    }
    Ok(())
}

pub fn bundle_static_library(
    user_object: &[u8],
    runtime_archive: &[u8],
    output_path: &Path,
) -> Result<(), LinkError> {
    for tool in ["ar", "ranlib"] {
        if !has_program(tool) {
            return Err(LinkError::NoLinker(format!("`{tool}` not found: install binutils")));
        }
    }
    let out_name = output_name(output_path);
    let dir = std::env::temp_dir().join(format!("rnx-ar-{}-{out_name}", std::process::id()));
    std::fs::create_dir_all(&dir).map_err(|e| LinkError::Io(e.to_string()))?;
    let result = bundle_inner(user_object, runtime_archive, output_path, &dir);
    let _ = std::fs::remove_dir_all(&dir);
    result
}

fn bundle_inner(
    user_object: &[u8],
    runtime_archive: &[u8],
    output_path: &Path,
    dir: &Path,
) -> Result<(), LinkError> {
    std::fs::write(dir.join("user.o"), user_object).map_err(|e| LinkError::Io(e.to_string()))?;
    std::fs::write(dir.join("rt.a"), runtime_archive).map_err(|e| LinkError::Io(e.to_string()))?;
    let members = Command::new("ar")
        .arg("t")
        .arg(dir.join("rt.a"))
        .output()
        .map_err(|e| LinkError::Io(e.to_string()))?;
    if !members.status.success() {
        return Err(LinkError::LinkFailed(
            String::from_utf8_lossy(&members.stderr).trim().to_string(),
        ));
    }
    let mut names: Vec<String> = String::from_utf8_lossy(&members.stdout)
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && *l != "/" && *l != "__.SYMDEF")
        .map(|l| l.to_string())
        .collect();
    names.sort();
    let ext = dir.join("rtx");
    std::fs::create_dir_all(&ext).map_err(|e| LinkError::Io(e.to_string()))?;
    let extract = Command::new("ar")
        .arg("x")
        .arg(dir.join("rt.a"))
        .current_dir(&ext)
        .output()
        .map_err(|e| LinkError::Io(e.to_string()))?;
    if !extract.status.success() {
        return Err(LinkError::LinkFailed(
            String::from_utf8_lossy(&extract.stderr).trim().to_string(),
        ));
    }
    let staged = dir.join("libout.a");
    let mut cmd = Command::new("ar");
    cmd.arg("cqD").arg(&staged).arg(dir.join("user.o"));
    for n in &names {
        cmd.arg(ext.join(n));
    }
    let repack = cmd.output().map_err(|e| LinkError::Io(e.to_string()))?;
    if !repack.status.success() {
        return Err(LinkError::LinkFailed(
            String::from_utf8_lossy(&repack.stderr).trim().to_string(),
        ));
    }
    let index = Command::new("ranlib")
        .arg("-D")
        .arg(&staged)
        .output()
        .map_err(|e| LinkError::Io(e.to_string()))?;
    if !index.status.success() {
        return Err(LinkError::LinkFailed(
            String::from_utf8_lossy(&index.stderr).trim().to_string(),
        ));
    }
    std::fs::copy(&staged, output_path).map_err(|e| LinkError::Io(e.to_string()))?;
    Ok(())
}

fn output_name(path: &Path) -> String {
    path.file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_else(|| "a.out".to_string())
}

fn pick_driver() -> Result<(String, Option<String>), LinkError> {
    let cc = first_present(&["cc", "gcc", "clang"]).ok_or_else(|| {
        LinkError::NoLinker("no C driver found: install gcc or clang".to_string())
    })?;
    if has_program("mold") {
        return Ok((cc, Some("-fuse-ld=mold".to_string())));
    }
    if has_program("ld.lld") {
        return Ok((cc, Some("-fuse-ld=lld".to_string())));
    }
    Ok((cc, None))
}

fn first_present(names: &[&str]) -> Option<String> {
    names.iter().find(|n| has_program(n)).map(|n| n.to_string())
}

fn has_program(name: &str) -> bool {
    std::env::var_os("PATH").map_or(false, |paths| {
        std::env::split_paths(&paths).any(|d| d.join(name).is_file())
    })
}

fn stage_inputs(inputs: &[LinkInput], dir: &Path) -> Result<Vec<PathBuf>, LinkError> {
    let mut out = Vec::with_capacity(inputs.len());
    for (i, input) in inputs.iter().enumerate() {
        match input {
            LinkInput::ObjectBytes(b) => {
                let p = dir.join(format!("mod{i}.o"));
                std::fs::write(&p, b).map_err(|e| LinkError::Io(e.to_string()))?;
                out.push(p);
            }
            LinkInput::ObjectPath(p) => out.push(p.clone()),
            LinkInput::ArchiveBytes(b) => {
                let p = dir.join(format!("lib{i}.a"));
                std::fs::write(&p, b).map_err(|e| LinkError::Io(e.to_string()))?;
                out.push(p);
            }
            LinkInput::ArchiveRef(b) => {
                let p = dir.join(format!("lib{i}.a"));
                std::fs::write(&p, b).map_err(|e| LinkError::Io(e.to_string()))?;
                out.push(p);
            }
            LinkInput::ArchivePath(p) => out.push(p.clone()),
            LinkInput::Lib(name) => {
                out.push(PathBuf::from(format!("-l{name}")));
            }
        }
    }
    Ok(out)
}
