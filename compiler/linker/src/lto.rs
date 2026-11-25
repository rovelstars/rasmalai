use crate::{LinkError, LinkInput, TargetTriple};
use crate::native::{system_file_in, system_search_dirs_for};
use std::path::{Path, PathBuf};
use std::process::Command;
use sysroot::Sysroot;

fn rustc_sysroot() -> Result<PathBuf, LinkError> {
    let output = Command::new("rustc")
        .arg("--print")
        .arg("sysroot")
        .output()
        .map_err(|e| LinkError::NoLinker(format!("cannot run rustc for LTO: {e}")))?;
    if !output.status.success() {
        return Err(LinkError::NoLinker("rustc --print sysroot failed".to_string()));
    }
    Ok(PathBuf::from(String::from_utf8_lossy(&output.stdout).trim()))
}

pub fn find_rust_lld() -> Result<PathBuf, LinkError> {
    let sysroot = rustc_sysroot()?;
    let host = TargetTriple(crate::host_triple().0);
    let path = sysroot
        .join("lib/rustlib")
        .join(&host.0)
        .join("bin/gcc-ld/ld.lld");
    if path.is_file() {
        return Ok(path);
    }
    Err(LinkError::NoLinker(format!(
        "no rust-lld with LLVM plugin support at {} (install the rustc component `rust-std` for {})",
        path.display(),
        host.0
    )))
}

fn stage(inputs: &[LinkInput], dir: &Path) -> Result<(Vec<PathBuf>, Vec<PathBuf>), LinkError> {
    let mut objects = Vec::new();
    let mut archives = Vec::new();
    for (index, input) in inputs.iter().enumerate() {
        match input {
            LinkInput::ObjectBytes(bytes) => {
                let path = dir.join(format!("bitcode{index}.bc"));
                std::fs::write(&path, bytes)
                    .map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
                objects.push(path);
            }
            LinkInput::ObjectPath(path) => objects.push(path.clone()),
            LinkInput::ArchiveBytes(bytes) => {
                let path = dir.join(format!("archive{index}.a"));
                std::fs::write(&path, bytes)
                    .map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
                archives.push(path);
            }
            LinkInput::ArchiveRef(bytes) | LinkInput::ArchiveRefKnown { bytes, .. } => {
                let path = dir.join(format!("archive{index}.a"));
                std::fs::write(&path, bytes)
                    .map_err(|e| LinkError::Io(format!("{}: {e}", path.display())))?;
                archives.push(path);
            }
            LinkInput::ArchivePath(path) => archives.push(path.clone()),
            LinkInput::Lib(name) => {
                return Err(LinkError::Unsupported(format!(
                    "`-l{name}` is not supported in the LTO link; pass an explicit archive path"
                )));
            }
        }
    }
    Ok((objects, archives))
}

pub fn link_lto(
    inputs: &[LinkInput],
    output: &Path,
    target: &TargetTriple,
) -> Result<(), LinkError> {
    let lld = find_rust_lld()?;
    let sysroot = Sysroot::discover(None);
    let dir = std::env::temp_dir().join(format!(
        "rnx-lto-{}-{}",
        std::process::id(),
        output.file_name().map(|s| s.to_string_lossy()).unwrap_or_default()
    ));
    std::fs::create_dir_all(&dir).map_err(|e| LinkError::Io(e.to_string()))?;
    let result = (|| {
        let (objects, archives) = stage(inputs, &dir)?;
        let crt = ["Scrt1.o", "crti.o", "crtbeginS.o"];
        let crt_end = ["crtendS.o", "crtn.o"];
        let mut cmd = Command::new(&lld);
        cmd.arg("--sysroot=/")
            .arg("--eh-frame-hdr")
            .arg("-m")
            .arg("elf_x86_64")
            .arg("-pie")
            .arg("--allow-multiple-definition")
            .arg("-dynamic-linker")
            .arg("/lib64/ld-linux-x86-64.so.2");
        for dir in system_search_dirs_for(&sysroot, &target.0) {
            cmd.arg("-L").arg(dir);
        }
        for name in crt {
            let path = system_file_in(&sysroot, &target.0, name)?;
            cmd.arg(path);
        }
        for path in &objects {
            cmd.arg(path);
        }
        for path in &archives {
            cmd.arg(path);
        }
        for name in crt_end {
            let path = system_file_in(&sysroot, &target.0, name)?;
            cmd.arg(path);
        }
        for lib in ["c", "m", "pthread", "dl", "gcc_s"] {
            cmd.arg(format!("-l{lib}"));
        }
        cmd.arg("-o").arg(output);
        let result = cmd
            .output()
            .map_err(|e| LinkError::Io(format!("cannot run rust-lld: {e}")))?;
        if !result.status.success() {
            return Err(LinkError::LinkFailed(
                String::from_utf8_lossy(&result.stderr).trim().to_string(),
            ));
        }
        Ok(())
    })();
    let _ = std::fs::remove_dir_all(&dir);
    result
}
