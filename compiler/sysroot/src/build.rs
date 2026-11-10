use crate::pack::{self, MANIFEST_FILE, Pack};
use crate::target::{Libc, ObjectFormat, Target};
use diagnostics::{Code, Diagnostic};
use serde_json::json;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};

const BUILTINS_PREFIX: &str = "libclang_rt.builtins-";

#[derive(Clone, Debug, Default)]
pub struct BuildOptions {
    pub from: Vec<PathBuf>,
    pub runtime: Option<PathBuf>,
    pub builtins: Option<PathBuf>,
    pub loader: Option<String>,
    pub kernel_min: Option<String>,
    pub glibc_baseline: Option<String>,
    pub prelude: Option<String>,
    pub abi: Option<u32>,
    pub force: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BuiltPack {
    pub dir: PathBuf,
    pub digest: String,
    pub target: String,
}

fn fail(message: impl Into<String>) -> Diagnostic {
    Diagnostic::new(
        Code::E124,
        format!("cannot build target pack: {}", message.into()),
    )
}

fn file_name(path: &Path) -> Result<String, Diagnostic> {
    path.file_name()
        .and_then(|name| name.to_str())
        .map(str::to_string)
        .ok_or_else(|| fail(format!("`{}` has no file name", path.display())))
}

pub fn build(target: &Target, out: &Path, options: &BuildOptions) -> Result<BuiltPack, Diagnostic> {
    if out.exists() {
        if !options.force {
            return Err(fail(format!(
                "`{}` already exists; pass `--force` to replace it",
                out.display()
            )));
        }
        std::fs::remove_dir_all(out)
            .map_err(|e| fail(format!("cannot clear `{}`: {e}", out.display())))?;
    }

    let lib_rel = format!("lib/rustlib/{}/lib", target.triple);
    let lib_dir = out.join(&lib_rel);
    std::fs::create_dir_all(&lib_dir)
        .map_err(|e| fail(format!("cannot create `{}`: {e}", lib_dir.display())))?;

    let candidates = candidate_dirs(&options.from, target);
    let mut stager = Stager {
        dir: lib_dir.clone(),
        staged: BTreeMap::new(),
    };

    let runtime = match &options.runtime {
        Some(path) => path.clone(),
        None => find(&candidates, &[target.runtime.file.clone()]).ok_or_else(|| {
            fail(format!(
                "no `{}`; pass `--runtime <archive>` or point `--from` at a staged pack",
                target.runtime.file
            ))
        })?,
    };
    if !runtime.is_file() {
        return Err(fail(format!(
            "runtime archive `{}` is not a file",
            runtime.display()
        )));
    }
    let runtime_name = stager.stage(&runtime)?;
    let runtime_sha =
        pack::file_sha256(&stager.dir.join(&runtime_name)).map_err(|e| e.diagnostic())?;

    let builtins = if let Some(path) = &options.builtins {
        Some(stager.stage(path)?)
    } else if let Some(name) = &target.link.builtins {
        find(&candidates, &[name.clone()])
            .map(|path| stager.stage(&path))
            .transpose()?
    } else {
        find_builtins(&candidates, target)
            .map(|path| stager.stage(&path))
            .transpose()?
    };

    let libc = find_lib(&candidates, "c", target);
    if let Some(path) = &libc {
        stager.stage(path)?;
    }

    let mut libs = BTreeMap::new();
    for name in target.link.libs.keys() {
        let found = find_lib(&candidates, name, target).or_else(|| {
            if name == "gcc_s" || target.libc == Libc::None {
                None
            } else {
                libc.clone()
            }
        });
        let staged = match found {
            Some(path) => Some(stager.stage(&path)?),
            None => None,
        };
        libs.insert(name.clone(), staged);
    }
    if target.libc == Libc::Musl
        && !libs.contains_key("gcc_s")
        && let Some(path) = find(&candidates, &["libgcc_eh.a".to_string()])
    {
        libs.insert("gcc_s".to_string(), Some(stager.stage(&path)?));
    }

    let (crt, crt_end) = discover_crt(&candidates, target, &mut stager)?;

    let search = vec![lib_rel.clone()];
    let manifest = |digest: &str| {
        json!({
            "version": 2,
            "target": target.triple,
            "arch": target.arch.as_str(),
            "os": target.os.as_str(),
            "env": target.env.as_str(),
            "object": target.object.as_str(),
            "libc": target.libc.as_str(),
            "pointer_width": target.pointer_width,
            "endian": target.endian.as_str(),
            "tier": match target.tier.as_str() {
                "1" => 1,
                "2" => 2,
                _ => 3,
            },
            "capabilities": target.capabilities.iter().map(|c| c.as_str()).collect::<Vec<_>>(),
            "cpu": {"model": target.cpu.model, "features": target.cpu.features},
            "loader": options.loader.clone().or_else(|| target.loader.clone()),
            "kernel_min": options.kernel_min.clone().or_else(|| target.kernel_min.clone()),
            "glibc_baseline": options.glibc_baseline.clone().or_else(|| target.glibc_baseline.clone()),
            "prelude": options.prelude.clone().unwrap_or_else(|| target.prelude.clone()),
            "link": {
                "search": search,
                "crt": crt,
                "crt_end": crt_end,
                "libs": libs,
                "builtins": builtins,
                "runtime": runtime_name,
            },
            "runtime": {
                "abi": options.abi.unwrap_or(1),
                "file": runtime_name,
                "sha256": runtime_sha,
            },
            "integrity": {"algorithm": "sha256", "digest": digest},
        })
    };

    let manifest_path = out.join(MANIFEST_FILE);
    write_manifest(&manifest_path, &manifest(&placeholder()))?;
    let digest = pack::Pack::digest(out).map_err(|e| e.diagnostic())?;
    write_manifest(&manifest_path, &manifest(&digest))?;
    let pack = Pack::load(out).map_err(|e| e.diagnostic())?;
    Ok(BuiltPack {
        dir: out.to_path_buf(),
        digest,
        target: pack.target.triple.clone(),
    })
}

fn write_manifest(path: &Path, value: &serde_json::Value) -> Result<(), Diagnostic> {
    let body = serde_json::to_vec_pretty(value)
        .map_err(|e| fail(format!("cannot encode `{}`: {e}", path.display())))?;
    std::fs::write(path, body).map_err(|e| fail(format!("cannot write `{}`: {e}", path.display())))
}

fn placeholder() -> String {
    format!("sha256:{}", "0".repeat(64))
}

struct Stager {
    dir: PathBuf,
    staged: BTreeMap<String, Vec<u8>>,
}

impl Stager {
    fn stage(&mut self, source: &Path) -> Result<String, Diagnostic> {
        if !source.is_file() {
            return Err(fail(format!("`{}` is not a file", source.display())));
        }
        let name = file_name(source)?;
        let bytes = std::fs::read(source)
            .map_err(|e| fail(format!("cannot read `{}`: {e}", source.display())))?;
        if let Some(existing) = self.staged.get(&name)
            && existing != &bytes
        {
            return Err(fail(format!(
                "two different files want to be `{name}` in the pack; stage one input tree"
            )));
        }
        self.staged.insert(name.clone(), bytes.clone());
        std::fs::write(self.dir.join(&name), bytes)
            .map_err(|e| fail(format!("cannot write `{}` into the pack: {e}", name)))?;
        Ok(name)
    }
}

fn find(candidates: &[PathBuf], names: &[String]) -> Option<PathBuf> {
    for dir in candidates {
        for name in names {
            let path = dir.join(name);
            if path.is_file() {
                return Some(path);
            }
        }
    }
    None
}

fn find_lib(candidates: &[PathBuf], name: &str, target: &Target) -> Option<PathBuf> {
    let mut names = vec![format!("lib{name}.a")];
    if target.object == ObjectFormat::MachO {
        names.push(format!("lib{name}.tbd"));
    }
    if target.os == crate::target::Os::Windows {
        names.push(format!("lib{name}.lib"));
        names.push(format!("{name}.lib"));
    }
    find(candidates, &names)
}

fn find_builtins(candidates: &[PathBuf], target: &Target) -> Option<PathBuf> {
    let name = format!("{BUILTINS_PREFIX}{}.a", target.arch.as_str());
    find(candidates, &[name])
}

fn candidate_dirs(roots: &[PathBuf], target: &Target) -> Vec<PathBuf> {
    let mut out: Vec<PathBuf> = Vec::new();
    for root in roots {
        let mut push = |rel: String| {
            let path = if rel.is_empty() {
                root.clone()
            } else {
                root.join(&rel)
            };
            if path.is_dir() && !out.contains(&path) {
                out.push(path);
            }
        };
        push(String::new());
        push(format!("lib/rustlib/{}/lib", target.triple));
        push("lib".to_string());
        push("lib64".to_string());
        push("usr/lib".to_string());
        push("usr/lib64".to_string());
        push(format!("lib/{}", target.triple));
        push(format!("usr/lib/{}", target.triple));
        if target.os == crate::target::Os::Linux {
            let arch = target.arch.as_str();
            let env = match target.libc {
                Libc::Musl => "musl",
                Libc::Glibc => "gnu",
                _ => target.env.as_str(),
            };
            push(format!("usr/lib/{arch}-linux-{env}"));
            push(format!("lib/{arch}-linux-{env}"));
            push(format!("usr/lib/{arch}-linux-{env}/lib"));
        }
        push("ucrt64/lib".to_string());
        push("mingw64/lib".to_string());
        push("usr/ucrt64/lib".to_string());
        push("usr/mingw64/lib".to_string());
        push("usr/lib/clang".to_string());
        for base in ["lib/clang", "usr/lib/clang"] {
            let clang = root.join(base);
            let Ok(entries) = std::fs::read_dir(&clang) else {
                continue;
            };
            let mut versions: Vec<PathBuf> = entries.flatten().map(|entry| entry.path()).collect();
            versions.sort();
            for version in versions {
                if !version.is_dir() {
                    continue;
                }
                for sub in ["lib/linux", "lib", "lib/windows", "lib/darwin"] {
                    push(
                        version
                            .join(sub)
                            .strip_prefix(root)
                            .map(|p| p.to_string_lossy().replace('\\', "/"))
                            .unwrap_or_default(),
                    );
                }
            }
        }
    }
    out
}

type CrtModes = BTreeMap<String, Vec<String>>;

fn discover_crt(
    candidates: &[PathBuf],
    target: &Target,
    stager: &mut Stager,
) -> Result<(CrtModes, CrtModes), Diagnostic> {
    let modes: &[(&str, &[&str], &[&str])] = match target.object {
        ObjectFormat::Elf => &[
            (
                "static",
                &["crt1.o", "crti.o", "crtbeginT.o"],
                &["crtend.o", "crtn.o"],
            ),
            (
                "dynpie",
                &["Scrt1.o", "crti.o", "crtbeginS.o"],
                &["crtendS.o", "crtn.o"],
            ),
            (
                "staticpie",
                &["rcrt1.o", "crti.o", "crtbeginS.o"],
                &["crtendS.o", "crtn.o"],
            ),
        ],
        ObjectFormat::Coff => &[("static", &["crt2.o", "crtbegin.o"], &["crtend.o"])],
        ObjectFormat::MachO => &[("static", &["crt1.o"], &[])],
        ObjectFormat::Wasm => &[],
    };

    let mut crt = BTreeMap::new();
    let mut crt_end = BTreeMap::new();
    for (mode, start_names, end_names) in modes {
        let start = longest_prefix(candidates, start_names);
        if start.is_empty() {
            continue;
        }
        let mut staged = Vec::with_capacity(start.len());
        for path in &start {
            staged.push(stager.stage(path)?);
        }
        crt.insert((*mode).to_string(), staged);

        let end = longest_end(candidates, end_names);
        if !end.is_empty() {
            let mut staged = Vec::with_capacity(end.len());
            for path in end {
                staged.push(stager.stage(&path)?);
            }
            crt_end.insert((*mode).to_string(), staged);
        }
    }
    Ok((crt, crt_end))
}

fn longest_end(candidates: &[PathBuf], names: &[&str]) -> Vec<PathBuf> {
    if names.is_empty() {
        return Vec::new();
    }
    let full: Vec<PathBuf> = names
        .iter()
        .filter_map(|name| find(candidates, &[(*name).to_string()]))
        .collect();
    if full.len() == names.len() {
        return full;
    }
    let last = names[names.len() - 1];
    find(candidates, &[last.to_string()]).into_iter().collect()
}

fn longest_prefix(candidates: &[PathBuf], names: &[&str]) -> Vec<PathBuf> {
    if names.is_empty() {
        return Vec::new();
    }
    for take in (1..=names.len()).rev() {
        let mut paths = Vec::with_capacity(take);
        let mut all = true;
        for name in &names[..take] {
            match find(candidates, &[(*name).to_string()]) {
                Some(path) => paths.push(path),
                None => {
                    all = false;
                    break;
                }
            }
        }
        if all {
            return paths;
        }
    }
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    struct Scratch(PathBuf);

    impl Drop for Scratch {
        fn drop(&mut self) {
            let _ = fs::remove_dir_all(&self.0);
        }
    }

    fn scratch(tag: &str) -> Scratch {
        let dir = std::env::temp_dir().join(format!(
            "rnx-pack-build-{tag}-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map(|d| d.as_nanos())
                .unwrap_or(0)
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).unwrap();
        Scratch(dir)
    }

    fn touch(dir: &Path, name: &str, bytes: &[u8]) {
        fs::write(dir.join(name), bytes).unwrap();
    }

    fn musl_source(scratch: &Scratch) -> PathBuf {
        let source = scratch.0.join("source");
        fs::create_dir_all(&source).unwrap();
        for name in [
            "crt1.o",
            "crti.o",
            "crtn.o",
            "libc.a",
            "libruntime_native.a",
            "libgcc_eh.a",
        ] {
            touch(&source, name, name.as_bytes());
        }
        touch(&source, "libclang_rt.builtins-x86_64.a", b"builtins");
        source
    }

    #[test]
    fn build_seals_a_pack_from_a_flat_source() {
        let scratch = scratch("flat");
        let source = musl_source(&scratch);
        let out = scratch.0.join("pack");
        let target = Target::parse("x86_64-unknown-linux-musl").unwrap();
        let built = build(
            &target,
            &out,
            &BuildOptions {
                from: vec![source],
                runtime: Some(scratch.0.join("source").join("libruntime_native.a")),
                ..BuildOptions::default()
            },
        )
        .unwrap();
        assert_eq!(built.target, target.triple);
        let pack = Pack::load(&out).unwrap();
        assert_eq!(
            pack.manifest.link.crt.get("static").unwrap(),
            &["crt1.o", "crti.o"]
        );
        assert_eq!(
            pack.manifest.link.crt_end.get("static").unwrap(),
            &["crtn.o"]
        );
        assert_eq!(
            pack.manifest.link.libs.get("c").unwrap().as_deref(),
            Some("libc.a")
        );
        assert_eq!(
            pack.manifest.link.libs.get("m").unwrap().as_deref(),
            Some("libc.a")
        );
        assert_eq!(
            pack.manifest.link.libs.get("gcc_s").unwrap().as_deref(),
            Some("libgcc_eh.a")
        );
        assert_eq!(
            pack.manifest.runtime.sha256.as_deref(),
            Some(
                pack::file_sha256(
                    &out.join("lib/rustlib/x86_64-unknown-linux-musl/lib/libruntime_native.a")
                )
                .unwrap()
                .as_str()
            )
        );
        assert_eq!(pack.manifest.integrity.digest, built.digest);
    }

    #[test]
    fn build_refuses_to_overwrite_without_force() {
        let scratch = scratch("force");
        let source = musl_source(&scratch);
        let out = scratch.0.join("pack");
        let target = Target::parse("x86_64-unknown-linux-musl").unwrap();
        let options = BuildOptions {
            from: vec![source],
            runtime: Some(scratch.0.join("source").join("libruntime_native.a")),
            ..BuildOptions::default()
        };
        build(&target, &out, &options).unwrap();
        let err = build(&target, &out, &options).unwrap_err();
        assert!(err.message.contains("--force"), "{}", err.message);
        build(
            &target,
            &out,
            &BuildOptions {
                force: true,
                ..options
            },
        )
        .unwrap();
    }

    #[test]
    fn build_requires_a_runtime() {
        let scratch = scratch("runtime");
        let target = Target::parse("x86_64-unknown-linux-musl").unwrap();
        let err = build(&target, &scratch.0.join("pack"), &BuildOptions::default()).unwrap_err();
        assert!(err.message.contains("--runtime"), "{}", err.message);
    }

    #[test]
    fn build_rejects_conflicting_file_names() {
        let scratch = scratch("collision");
        let one = scratch.0.join("one");
        let two = scratch.0.join("two");
        fs::create_dir_all(&one).unwrap();
        fs::create_dir_all(&two).unwrap();
        touch(&one, "libc.a", b"one");
        touch(&two, "libc.a", b"two");
        touch(&one, "libruntime_native.a", b"runtime");
        let target = Target::parse("x86_64-unknown-linux-musl").unwrap();
        let err = build(
            &target,
            &scratch.0.join("pack"),
            &BuildOptions {
                from: vec![one.clone()],
                runtime: Some(one.join("libruntime_native.a")),
                builtins: Some(two.join("libc.a")),
                ..BuildOptions::default()
            },
        )
        .unwrap_err();
        assert!(
            err.message.contains("two different files"),
            "{}",
            err.message
        );
    }

    #[test]
    fn build_handles_runtime_only_targets() {
        let scratch = scratch("uefi");
        let runtime = scratch.0.join("libruntime_native.a");
        touch(&scratch.0, "libruntime_native.a", b"uefi runtime");
        let target = Target::parse("x86_64-unknown-uefi").unwrap();
        let built = build(
            &target,
            &scratch.0.join("pack"),
            &BuildOptions {
                runtime: Some(runtime),
                ..BuildOptions::default()
            },
        )
        .unwrap();
        let pack = Pack::load(&built.dir).unwrap();
        assert!(pack.manifest.link.crt.is_empty());
        assert!(pack.manifest.link.libs.is_empty());
        assert_eq!(pack.manifest.runtime.file, "libruntime_native.a");
    }
}
