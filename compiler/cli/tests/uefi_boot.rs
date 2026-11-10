//! End-to-end UEFI check: build the target runtime archive, stage a target
//! pack, link a real PE32+ image, and boot it under qemu with OVMF.
//!
//! The test skips with a printed reason when the runner lacks the pieces:
//! the `x86_64-unknown-uefi` Rust target, `qemu-system-x86_64`, or OVMF.

#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};
use std::time::{Duration, Instant};

const TRIPLE: &str = "x86_64-unknown-uefi";
const BANNER: &str = "hello from uefi";

fn repo() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../..")
}

fn tool(name: &str) -> Option<PathBuf> {
    let out = Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {name}"))
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let path = String::from_utf8_lossy(&out.stdout).trim().to_string();
    (!path.is_empty()).then(|| PathBuf::from(path))
}

fn ovmf() -> Option<PathBuf> {
    [
        "/usr/share/edk2/x64/OVMF.4m.fd",
        "/usr/share/OVMF/x64/OVMF.4m.fd",
        "/usr/share/edk2/ovmf/OVMF_CODE.fd",
        "/usr/share/OVMF/OVMF_CODE.fd",
        "/usr/share/edk2/x64/OVMF_CODE.4m.fd",
    ]
    .iter()
    .map(PathBuf::from)
    .find(|path| path.is_file())
}

fn rust_target_installed(target: &str) -> bool {
    let out = Command::new("rustup")
        .args(["target", "list", "--installed"])
        .output();
    match out {
        Ok(out) if out.status.success() => String::from_utf8_lossy(&out.stdout)
            .lines()
            .any(|line| line.trim() == target),
        _ => false,
    }
}

fn run(dir: &Path, cache: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rnx"))
        .args(args)
        .current_dir(dir)
        .env("RNX_CACHE_HOME", cache)
        .env("RNX_VENDOR_DIR", repo().join("compiler/stdlib/src"))
        .env("RNX_REGISTRY", "http://127.0.0.1:9")
        .env_remove("RNX_SYSROOT")
        .env_remove("RNX_OFFLINE")
        .env_remove("CI")
        .output()
        .unwrap()
}

fn build_runtime_archive(scratch: &Path) -> PathBuf {
    let manifest = repo().join("compiler/runtime/uefi/Cargo.toml");
    let target_dir = scratch.join("runtime-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo)
        .args(["build", "--manifest-path"])
        .arg(&manifest)
        .args(["--target", TRIPLE, "--release", "--offline", "--target-dir"])
        .arg(&target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("spawn cargo for the UEFI runtime archive");
    assert!(
        out.status.success(),
        "runtime archive build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let archive = target_dir
        .join(TRIPLE)
        .join("release")
        .join("libruntime_native.a");
    assert!(
        archive.is_file(),
        "runtime archive missing at {}",
        archive.display()
    );
    archive
}

fn stage_pack(root: &Path, archive: &Path) -> String {
    let lib = root
        .join("lib")
        .join("rustlib")
        .join(TRIPLE)
        .join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::copy(archive, lib.join("libruntime_native.a")).unwrap();
    let sha = sysroot::pack::file_sha256(&lib.join("libruntime_native.a")).unwrap();
    let manifest = |digest: &str| {
        format!(
            "{{\"version\": 2, \"target\": \"{TRIPLE}\", \"arch\": \"x86_64\", \"os\": \"uefi\", \"env\": \"none\", \"object\": \"coff\", \"libc\": \"none\", \"pointer_width\": 64, \"endian\": \"little\", \"tier\": 3, \"capabilities\": [\"alloc\", \"console\", \"clock\", \"firmware\"], \"cpu\": {{\"model\": \"x86-64\", \"features\": \"\"}}, \"loader\": null, \"kernel_min\": null, \"glibc_baseline\": null, \"prelude\": \"@uefi/prelude\", \"link\": {{\"search\": [\"lib/rustlib/{TRIPLE}/lib\"], \"crt\": {{}}, \"crt_end\": {{}}, \"libs\": {{}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": \"{sha}\"}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"{digest}\"}}}}"
        )
    };
    std::fs::write(
        root.join("sysroot.json"),
        manifest(&format!("sha256:{}", "0".repeat(64))),
    )
    .unwrap();
    let digest = sysroot::pack::Pack::digest(root).unwrap();
    std::fs::write(root.join("sysroot.json"), manifest(&digest)).unwrap();
    digest
}

fn project(root: &Path) -> PathBuf {
    let prelude = repo().join("compiler/stdlib/uefi_prelude");
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"uefi-probe\",\n        version: \"0.1.0\",\n        targets: [\"{TRIPLE}\"]\n    }},\n    dependencies: {{\n        \"@std/prelude\": {{ alias: \"@uefi/prelude\" }},\n        \"@uefi/prelude\": {{ path: \"{}\" }}\n    }}\n}}\n",
            prelude.display()
        ),
    )
    .unwrap();
    let src = root.join("src").join("main.rnx");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(
        &src,
        "fn Main(): Int {\n    println!(\"hello from uefi\");\n    return 0;\n}\n",
    )
    .unwrap();
    src
}

struct Headers {
    machine: u16,
    magic: u16,
    subsystem: u16,
    entry: u32,
}

fn headers(path: &Path) -> Headers {
    let bytes = std::fs::read(path).unwrap();
    let pe_at = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    assert_eq!(&bytes[pe_at..pe_at + 4], b"PE\0\0", "not a PE image");
    let machine = u16::from_le_bytes([bytes[pe_at + 4], bytes[pe_at + 5]]);
    let opt = pe_at + 24;
    let magic = u16::from_le_bytes([bytes[opt], bytes[opt + 1]]);
    let entry = u32::from_le_bytes(bytes[opt + 16..opt + 20].try_into().unwrap());
    let subsystem = u16::from_le_bytes([bytes[opt + 68], bytes[opt + 69]]);
    Headers {
        machine,
        magic,
        subsystem,
        entry,
    }
}

fn boot(qemu: &Path, firmware: &Path, image: &Path, scratch: &Path) -> String {
    let esp = scratch.join("esp");
    let boot_dir = esp.join("EFI").join("BOOT");
    std::fs::create_dir_all(&boot_dir).unwrap();
    std::fs::copy(image, boot_dir.join("BOOTX64.EFI")).unwrap();
    let log_path = scratch.join("serial.log");
    let log = std::fs::File::create(&log_path).unwrap();
    let stderr = std::fs::File::create(scratch.join("qemu.err")).unwrap();
    let mut child = Command::new(qemu)
        .args(["-machine", "q35", "-m", "256", "-bios"])
        .arg(firmware)
        .arg("-drive")
        .arg(format!("format=raw,file=fat:rw:{}", esp.display()))
        .args(["-display", "none", "-serial", "stdio", "-no-reboot"])
        .stdin(Stdio::null())
        .stdout(Stdio::from(log))
        .stderr(Stdio::from(stderr))
        .spawn()
        .expect("spawn qemu-system-x86_64");
    let deadline = Instant::now() + Duration::from_secs(60);
    let mut text;
    while Instant::now() < deadline {
        text = std::fs::read_to_string(&log_path).unwrap_or_default();
        if text.contains(BANNER) {
            break;
        }
        std::thread::sleep(Duration::from_millis(200));
    }
    let _ = child.kill();
    let _ = child.wait();
    text = std::fs::read_to_string(&log_path).unwrap_or_default();
    if !text.contains(BANNER) {
        let mut out = std::io::stdout();
        let _ = writeln!(out, "--- serial log ---");
        let _ = out.write_all(text.as_bytes());
        let _ = writeln!(out, "--- end ---");
    }
    text
}

#[test]
fn uefi_pack_builds_and_boots_under_ovmf() {
    let Some(qemu) = tool("qemu-system-x86_64") else {
        return eprintln!("skipping `uefi_pack_builds_and_boots_under_ovmf`: qemu-system-x86_64 is absent");
    };
    let Some(firmware) = ovmf() else {
        return eprintln!(
            "skipping `uefi_pack_builds_and_boots_under_ovmf`: OVMF firmware is absent"
        );
    };
    if !rust_target_installed(TRIPLE) {
        return eprintln!(
            "skipping `uefi_pack_builds_and_boots_under_ovmf`: rustup target {TRIPLE} is absent"
        );
    }
    for required in ["llvm-readobj"] {
        if tool(required).is_none() {
            return eprintln!("skipping `uefi_pack_builds_and_boots_under_ovmf`: {required} is absent");
        }
    }

    let work = crate::scratch::dir("uefi-boot");
    let archive = build_runtime_archive(&work);
    let pack_root = work.join("pack");
    let digest = stage_pack(&pack_root, &archive);
    let cache = crate::scratch::dir("uefi-cache");
    let added = run(
        &work,
        &cache,
        &["sysroot", "add", "--local", pack_root.to_str().unwrap()],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert_eq!(
        sysroot::pack::Pack::load(&pack_root)
            .unwrap()
            .manifest
            .integrity
            .digest,
        digest
    );

    let root = crate::scratch::dir("uefi-project");
    let main = project(&root);

    let object = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            TRIPLE,
            "--release",
            "--emit-obj",
        ],
    );
    assert!(
        object.status.success(),
        "{}",
        String::from_utf8_lossy(&object.stderr)
    );
    let object_path = String::from_utf8_lossy(&object.stdout)
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("artifact: "))
        .map(|value| value.trim().to_string())
        .expect("emit-obj artifact path");
    let obj_report = Command::new(tool("llvm-readobj").unwrap())
        .arg("--file-headers")
        .arg(&object_path)
        .output()
        .unwrap();
    let obj_text = String::from_utf8_lossy(&obj_report.stdout);
    assert!(obj_text.contains("Format: COFF-x86-64"), "{obj_text}");
    assert!(obj_text.contains("IMAGE_FILE_MACHINE_AMD64"), "{obj_text}");

    let build = run(
        &root,
        &cache,
        &["build", main.to_str().unwrap(), "--target", TRIPLE, "--release"],
    );
    assert!(
        build.status.success(),
        "{}",
        String::from_utf8_lossy(&build.stderr)
    );
    let artifact = String::from_utf8_lossy(&build.stdout)
        .lines()
        .rev()
        .find_map(|line| line.strip_prefix("artifact: "))
        .map(|value| value.trim().trim_end_matches(" (fresh)").to_string())
        .expect("artifact path");
    let exe = PathBuf::from(&artifact);
    let head = headers(&exe);
    assert_eq!(head.machine, 0x8664, "machine");
    assert_eq!(head.magic, 0x20b, "PE32+");
    assert_eq!(head.subsystem, 10, "EFI application subsystem");
    assert_ne!(head.entry, 0, "entry point");
    let report = Command::new(tool("llvm-readobj").unwrap())
        .args(["--file-headers", "--sections"])
        .arg(&exe)
        .output()
        .unwrap();
    let text = String::from_utf8_lossy(&report.stdout);
    assert!(text.contains("IMAGE_SUBSYSTEM_EFI_APPLICATION"), "{text}");
    assert!(
        text.contains("ImportTableRVA: 0x0"),
        "a UEFI image must not import from a DLL: {text}"
    );

    let serial = boot(&qemu, &firmware, &exe, &work);
    assert!(
        serial.contains(BANNER),
        "the UEFI console never printed `{BANNER}`"
    );
    assert!(
        !serial.contains("Exception Type"),
        "the firmware raised an exception while running the image"
    );
}

#[test]
fn missing_uefi_pack_is_e124() {
    if !rust_target_installed(TRIPLE) {
        return eprintln!("skipping `missing_uefi_pack_is_e124`: rustup target {TRIPLE} is absent");
    }
    let cache = crate::scratch::dir("uefi-nopack-cache");
    let root = crate::scratch::dir("uefi-nopack-project");
    let main = project(&root);
    let out = Command::new(env!("CARGO_BIN_EXE_rnx"))
        .args(["build", main.to_str().unwrap(), "--target", TRIPLE, "--release"])
        .current_dir(&root)
        .env("RNX_CACHE_HOME", cache.path())
        .env("RNX_VENDOR_DIR", repo().join("compiler/stdlib/src"))
        .env("RNX_REGISTRY", "http://127.0.0.1:9")
        .env("RNX_OFFLINE", "1")
        .env_remove("RNX_SYSROOT")
        .env_remove("CI")
        .output()
        .unwrap();
    assert_eq!(out.status.code(), Some(1));
    let text = String::from_utf8_lossy(&out.stderr).into_owned()
        + &String::from_utf8_lossy(&out.stdout).into_owned();
    assert!(text.contains("E124"), "{text}");
    assert!(text.contains(TRIPLE), "{text}");
    assert!(text.contains("rnx sysroot add"), "{text}");
}
