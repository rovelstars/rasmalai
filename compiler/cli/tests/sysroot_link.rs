#[path = "../../testkit/pack.rs"]
mod pack_fixture;
#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::Output;

const HOST: &str = "x86_64-unknown-linux-gnu";
const ARM_MUSL: &str = "aarch64-unknown-linux-musl";

fn rnx() -> &'static str {
    env!("CARGO_BIN_EXE_rnx")
}

fn vendor() -> String {
    format!("{}/../stdlib/src", env!("CARGO_MANIFEST_DIR"))
}

fn run(dir: &Path, cache: &Path, args: &[&str]) -> Output {
    std::process::Command::new(rnx())
        .args(args)
        .current_dir(dir)
        .env("RNX_CACHE_HOME", cache)
        .env("RNX_VENDOR_DIR", vendor())
        .env("RNX_REGISTRY", "http://127.0.0.1:9")
        .env_remove("RNX_SYSROOT")
        .env_remove("RNX_OFFLINE")
        .env_remove("CI")
        .output()
        .unwrap()
}

fn stdout(output: &Output) -> String {
    String::from_utf8_lossy(&output.stdout).into_owned()
}

fn stderr(output: &Output) -> String {
    String::from_utf8_lossy(&output.stderr).into_owned()
}

fn combined(output: &Output) -> String {
    format!("{}{}", stdout(output), stderr(output))
}

fn project(tag: &str) -> (crate::scratch::Scratch, PathBuf) {
    let root = crate::scratch::dir(&format!("link-proj-{tag}"));
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: {\n        name: \"link-probe\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let src = root.join("src").join("main.rnx");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, "fn Main(): Int {\n    return 42;\n}\n").unwrap();
    (root, src)
}

fn artifact_of(output: &Output) -> PathBuf {
    let text = stdout(output);
    let path = text
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("artifact: "))
        .map(|value| value.trim().trim_end_matches(" (fresh)").to_string())
        .expect("build prints an artifact path");
    PathBuf::from(path)
}

fn host_file(name: &str) -> Option<PathBuf> {
    let cc = std::env::var("CC").unwrap_or_else(|_| "cc".to_string());
    let out = std::process::Command::new(cc)
        .arg(format!("-print-file-name={name}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8_lossy(&out.stdout).trim().to_string();
    let path = PathBuf::from(&text);
    (text != name && path.is_file()).then_some(path)
}

fn workspace_target() -> Option<PathBuf> {
    if let Ok(dir) = std::env::var("CARGO_TARGET_DIR")
        && !dir.is_empty()
    {
        return Some(PathBuf::from(dir));
    }
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("..");
    let target = root.join("target");
    target.is_dir().then_some(target)
}

fn find_runtime_archive() -> Option<PathBuf> {
    let target = workspace_target()?;
    let mut found: Vec<(std::time::SystemTime, PathBuf)> = Vec::new();
    for profile in ["debug", "release"] {
        let dir = target.join(profile).join("build");
        let Ok(entries) = std::fs::read_dir(&dir) else {
            continue;
        };
        for entry in entries.flatten() {
            let name = entry.file_name().to_string_lossy().into_owned();
            if !name.starts_with("runtime-") {
                continue;
            }
            let archive = entry.path().join("out").join("libruntime_native.a");
            if archive.is_file() {
                let modified = std::fs::metadata(&archive)
                    .and_then(|meta| meta.modified())
                    .unwrap_or(std::time::SystemTime::UNIX_EPOCH);
                found.push((modified, archive));
            }
        }
    }
    found.sort();
    found.pop().map(|(_, path)| path)
}

struct GnuPack {
    alias_libc: Option<String>,
    abi: u32,
    extra: Option<&'static str>,
    corrupt_alias: bool,
}

impl Default for GnuPack {
    fn default() -> Self {
        GnuPack {
            alias_libc: None,
            abi: 1,
            extra: None,
            corrupt_alias: false,
        }
    }
}

fn write_gnu_pack(dir: &Path, runtime_archive: &Path, options: GnuPack) -> Result<String, String> {
    let lib = dir.join("lib").join("rustlib").join(HOST).join("lib");
    std::fs::create_dir_all(&lib).map_err(|e| format!("create {}: {e}", lib.display()))?;
    let mut names: Vec<String> = Vec::new();
    for name in [
        "Scrt1.o",
        "crti.o",
        "crtbeginS.o",
        "crtendS.o",
        "crtn.o",
        "crt1.o",
        "crtbeginT.o",
        "crtend.o",
        "rcrt1.o",
        "libm.so.6",
        "libgcc_s.so.1",
    ] {
        let source = host_file(name).ok_or_else(|| format!("host toolchain has no `{name}`"))?;
        std::fs::copy(&source, lib.join(name)).map_err(|e| format!("copy {name}: {e}"))?;
        names.push(name.to_string());
    }
    let libc_name = options
        .alias_libc
        .clone()
        .unwrap_or_else(|| "libc.so.6".to_string());
    let libc_source =
        host_file("libc.so.6").ok_or_else(|| "host toolchain has no `libc.so.6`".to_string())?;
    if options.corrupt_alias {
        std::fs::write(lib.join(&libc_name), b"not an ELF object")
            .map_err(|e| format!("write {libc_name}: {e}"))?;
    } else {
        std::fs::copy(&libc_source, lib.join(&libc_name))
            .map_err(|e| format!("copy {libc_name}: {e}"))?;
    }
    std::fs::copy(runtime_archive, lib.join("libruntime_native.a"))
        .map_err(|e| format!("copy runtime archive: {e}"))?;
    if let Some(extra) = options.extra {
        let path = dir.join("share").join("rnx").join(extra);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(&path, extra.as_bytes()).unwrap();
    }
    let runtime_sha =
        sysroot::pack::file_sha256(&lib.join("libruntime_native.a")).map_err(|e| e.to_string())?;
    let manifest = format!(
        "{{\"version\": 2, \"target\": \"{HOST}\", \"arch\": \"x86_64\", \"os\": \"linux\", \"env\": \"gnu\", \"object\": \"elf\", \"libc\": \"glibc\", \"pointer_width\": 64, \"endian\": \"little\", \"tier\": 1, \"capabilities\": [\"alloc\", \"console\", \"clock\", \"fs\", \"log\", \"net\", \"process\", \"random\", \"threads\"], \"cpu\": {{\"model\": \"x86-64\", \"features\": \"\"}}, \"loader\": \"/lib64/ld-linux-x86-64.so.2\", \"kernel_min\": null, \"glibc_baseline\": \"2.17\", \"prelude\": \"@std/prelude\", \"link\": {{\"search\": [\"lib/rustlib/{HOST}/lib\"], \"crt\": {{\"static\": [\"crt1.o\", \"crti.o\", \"crtbeginT.o\"], \"dynpie\": [\"Scrt1.o\", \"crti.o\", \"crtbeginS.o\"], \"staticpie\": [\"rcrt1.o\", \"crti.o\", \"crtbeginS.o\"]}}, \"crt_end\": {{\"static\": [\"crtend.o\", \"crtn.o\"], \"dynpie\": [\"crtendS.o\", \"crtn.o\"], \"staticpie\": [\"crtendS.o\", \"crtn.o\"]}}, \"libs\": {{\"c\": \"{libc_name}\", \"m\": \"libm.so.6\", \"pthread\": null, \"dl\": null, \"rt\": null, \"gcc_s\": \"libgcc_s.so.1\"}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": {}, \"file\": \"libruntime_native.a\", \"sha256\": \"{runtime_sha}\"}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"sha256:{}\"}}}}",
        options.abi,
        "0".repeat(64),
    );
    std::fs::write(dir.join("sysroot.json"), manifest).map_err(|e| e.to_string())?;
    let digest = sysroot::pack::Pack::digest(dir).map_err(|e| e.to_string())?;
    let sealed = std::fs::read_to_string(dir.join("sysroot.json"))
        .map_err(|e| e.to_string())?
        .replace(&"0".repeat(64), &digest[7..]);
    std::fs::write(dir.join("sysroot.json"), sealed).map_err(|e| e.to_string())?;
    Ok(digest)
}

fn install(cache: &Path, dir: &Path) -> Output {
    run(
        dir,
        cache,
        &["sysroot", "add", "--local", dir.to_str().unwrap()],
    )
}

fn pin_host(cache: &Path, root: &Path) -> Output {
    run(root, cache, &["lock", "--target", HOST])
}

fn gnu_fixture(tag: &str, options: GnuPack) -> Option<(crate::scratch::Scratch, String)> {
    let runtime = find_runtime_archive()?;
    let dir = crate::scratch::dir(&format!("gnu-pack-{tag}"));
    match write_gnu_pack(&dir, &runtime, options) {
        Ok(digest) => Some((dir, digest)),
        Err(reason) => {
            eprintln!("skipping gnu pack test `{tag}`: {reason}");
            None
        }
    }
}

#[test]
fn gnu_pack_links_and_runs() {
    let Some((fixture, digest)) = gnu_fixture("link", GnuPack::default()) else {
        return;
    };
    let cache = crate::scratch::dir("gnu-pack-cache");
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    let (root, main) = project("gnu");
    let pinned = pin_host(&cache, &root);
    assert!(pinned.status.success(), "{}", combined(&pinned));
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            HOST,
            "--release",
            "--verbose",
        ],
    );
    assert!(build.status.success(), "{}", combined(&build));
    let log = stderr(&build);
    assert!(log.contains("target pack"), "{log}");
    assert!(log.contains(&digest), "{log}");
    let artifact = artifact_of(&build);
    let exec = std::process::Command::new(&artifact).output().unwrap();
    assert_eq!(exec.status.code(), Some(42), "{}", artifact.display());
}

#[test]
fn pack_digest_change_invalidates_the_build_key() {
    let Some((fixture, first_digest)) = gnu_fixture("key", GnuPack::default()) else {
        return;
    };
    let cache = crate::scratch::dir("gnu-key-cache");
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    let (root, main) = project("key");
    let pinned = pin_host(&cache, &root);
    assert!(pinned.status.success(), "{}", combined(&pinned));
    let args = [
        "build",
        main.to_str().unwrap(),
        "--target",
        HOST,
        "--release",
    ];
    let first = run(&root, &cache, &args);
    assert!(first.status.success(), "{}", combined(&first));
    let second = run(&root, &cache, &args);
    assert!(second.status.success(), "{}", combined(&second));
    assert!(stdout(&second).contains("(fresh)"), "{}", stdout(&second));

    let runtime = find_runtime_archive().expect("runtime archive");
    let second_fixture = GnuPack {
        extra: Some("second"),
        ..GnuPack::default()
    };
    let digest = write_gnu_pack(&fixture, &runtime, second_fixture).unwrap();
    assert_ne!(digest, first_digest);
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    let repinned = pin_host(&cache, &root);
    assert!(repinned.status.success(), "{}", combined(&repinned));
    let third = run(&root, &cache, &args);
    assert!(third.status.success(), "{}", combined(&third));
    assert!(!stdout(&third).contains("(fresh)"), "{}", stdout(&third));
}

#[test]
fn runtime_abi_mismatch_fails_e124() {
    let Some((fixture, _)) = gnu_fixture(
        "abi",
        GnuPack {
            abi: 2,
            ..GnuPack::default()
        },
    ) else {
        return;
    };
    let cache = crate::scratch::dir("gnu-abi-cache");
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    let (root, main) = project("abi");
    let pinned = pin_host(&cache, &root);
    assert!(pinned.status.success(), "{}", combined(&pinned));
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            HOST,
            "--release",
        ],
    );
    assert_eq!(build.status.code(), Some(1), "{}", combined(&build));
    let text = stderr(&build);
    assert!(text.contains("E124"), "{text}");
    assert!(text.contains("runtime ABI mismatch"), "{text}");
    assert!(text.contains("expected 1"), "{text}");
    assert!(text.contains("pack records 2"), "{text}");
}

#[test]
fn corrupt_pack_runtime_fails_e124() {
    let Some((fixture, digest)) = gnu_fixture("corrupt", GnuPack::default()) else {
        return;
    };
    let cache = crate::scratch::dir("gnu-corrupt-cache");
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    let (root, main) = project("corrupt");
    let pinned = pin_host(&cache, &root);
    assert!(pinned.status.success(), "{}", combined(&pinned));
    let runtime = cache
        .join("sysroots")
        .join(HOST)
        .join(&digest)
        .join("lib")
        .join("rustlib")
        .join(HOST)
        .join("lib")
        .join("libruntime_native.a");
    let mut bytes = std::fs::read(&runtime).unwrap();
    bytes[0] ^= 0xff;
    std::fs::write(&runtime, bytes).unwrap();
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            HOST,
            "--release",
        ],
    );
    assert_eq!(build.status.code(), Some(1), "{}", combined(&build));
    let text = stderr(&build);
    assert!(text.contains("E124"), "{text}");
    assert!(
        text.contains("integrity digest mismatch") || text.contains("runtime sha256"),
        "{text}"
    );
}

#[test]
fn missing_link_map_file_is_rejected() {
    let Some((fixture, _)) = gnu_fixture("missing", GnuPack::default()) else {
        return;
    };
    std::fs::remove_file(
        fixture
            .join("lib")
            .join("rustlib")
            .join(HOST)
            .join("lib")
            .join("Scrt1.o"),
    )
    .unwrap();
    let cache = crate::scratch::dir("gnu-missing-cache");
    let added = install(&cache, &fixture);
    assert_eq!(added.status.code(), Some(1), "{}", combined(&added));
    let text = stderr(&added);
    assert!(text.contains("E124"), "{text}");
    assert!(text.contains("Scrt1.o"), "{text}");
}

#[test]
fn link_map_library_alias_is_consumed() {
    let Some((fixture, _)) = gnu_fixture(
        "alias",
        GnuPack {
            alias_libc: Some("libc-packed.so.6".to_string()),
            ..GnuPack::default()
        },
    ) else {
        return;
    };
    let cache = crate::scratch::dir("gnu-alias-cache");
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    let (root, main) = project("alias");
    let pinned = pin_host(&cache, &root);
    assert!(pinned.status.success(), "{}", combined(&pinned));
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            HOST,
            "--release",
        ],
    );
    assert!(build.status.success(), "{}", combined(&build));

    let runtime = find_runtime_archive().expect("runtime archive");
    let broken = crate::scratch::dir("gnu-alias-broken");
    write_gnu_pack(
        &broken,
        &runtime,
        GnuPack {
            alias_libc: Some("libc-broken.so.6".to_string()),
            corrupt_alias: true,
            ..GnuPack::default()
        },
    )
    .unwrap();
    let added = install(&cache, &broken);
    assert!(added.status.success(), "{}", combined(&added));
    let repinned = pin_host(&cache, &root);
    assert!(repinned.status.success(), "{}", combined(&repinned));
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            HOST,
            "--release",
        ],
    );
    assert_eq!(build.status.code(), Some(1), "{}", combined(&build));
    assert!(
        stderr(&build).contains("libc-broken.so.6"),
        "the link map alias must be consumed: {}",
        stderr(&build)
    );
}

#[test]
fn unpinned_host_pack_keeps_the_embedded_runtime() {
    let Some((fixture, _)) = gnu_fixture(
        "unpinned",
        GnuPack {
            abi: 2,
            ..GnuPack::default()
        },
    ) else {
        return;
    };
    let cache = crate::scratch::dir("gnu-unpinned-cache");
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    let (root, main) = project("unpinned");
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            HOST,
            "--release",
            "--verbose",
        ],
    );
    assert!(build.status.success(), "{}", combined(&build));
    assert!(
        !stderr(&build).contains("target pack"),
        "an unpinned host pack must not be selected: {}",
        stderr(&build)
    );
    let exec = std::process::Command::new(artifact_of(&build))
        .output()
        .unwrap();
    assert_eq!(exec.status.code(), Some(42));

    let pinned = pin_host(&cache, &root);
    assert!(pinned.status.success(), "{}", combined(&pinned));
    let pinned_build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            HOST,
            "--release",
        ],
    );
    assert_eq!(
        pinned_build.status.code(),
        Some(1),
        "{}",
        combined(&pinned_build)
    );
    assert!(
        stderr(&pinned_build).contains("runtime ABI mismatch"),
        "{}",
        stderr(&pinned_build)
    );
}

#[test]
fn aarch64_pack_emits_an_object() {
    let fixture = crate::scratch::dir("arm-pack");
    let digest = crate::pack_fixture::write_pack(&fixture, ARM_MUSL, "arm");
    let cache = crate::scratch::dir("arm-pack-cache");
    let added = install(&cache, &fixture);
    assert!(added.status.success(), "{}", combined(&added));
    assert!(!digest.is_empty());
    let (root, main) = project("arm");
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            ARM_MUSL,
            "--emit-obj",
        ],
    );
    assert!(build.status.success(), "{}", combined(&build));
    let object = std::fs::read(artifact_of(&build)).unwrap();
    assert_eq!(&object[0..4], b"\x7fELF", "not an ELF object");
    assert_eq!(
        u16::from_le_bytes([object[18], object[19]]),
        183,
        "expected EM_AARCH64"
    );
}
