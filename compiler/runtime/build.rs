use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::process::Command;

// Deps whose objects are merged into the archive so it links standalone
// under a system linker. Must stay in sync with bundle/Cargo.toml.
const MERGE_DEPS: [&str; 4] = ["mio", "log", "rustls", "webpki_roots"];

fn run(cmd: &mut Command, what: &str) {
    let status = cmd.status().unwrap_or_else(|e| panic!("spawn {what}: {e}"));
    assert!(status.success(), "{what} failed");
}

// OUT_DIR (<target>/<profile>/build/<unit>/out) to <target>.
fn workspace_target_dir(out_dir: &Path) -> Option<PathBuf> {
    let mut cur = out_dir.to_path_buf();
    loop {
        if cur.file_name().map(|n| n == "build").unwrap_or(false) {
            return cur.parent()?.parent().map(|p| p.to_path_buf());
        }
        cur = cur.parent()?.to_path_buf();
    }
}

// Profile dir (<target>/<profile>) above OUT_DIR: artifacts placed here sit
// next to the `rnx` binary so dev-mode dynamic linking finds them on disk.
fn profile_dir(out_dir: &Path, target_dir: &Path) -> Option<PathBuf> {
    let rel = out_dir.strip_prefix(target_dir).ok()?;
    let first = rel.iter().next()?;
    Some(target_dir.join(first))
}

fn run_command_output(cmd: &mut Command, what: &str) -> String {
    let out = cmd.output().unwrap_or_else(|e| panic!("spawn {what}: {e}"));
    assert!(
        out.status.success(),
        "{what} failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    String::from_utf8(out.stdout).expect("bundle cargo output is utf-8")
}

// Direct rustc for wasm: reactor_stub code needs no third-party crates,
// matching the previous silent-skip behavior when no rlibs were found.
fn build_wasm_archive(src: &Path, archive: &Path, target: &str) {
    let rustc = std::env::var("RUSTC").unwrap_or_else(|_| "rustc".to_string());
    let mut cmd = Command::new(&rustc);
    cmd.env("CARGO_PKG_VERSION", env!("CARGO_PKG_VERSION"));
    cmd.args([
        "--edition=2024",
        "--crate-type=staticlib",
        "--crate-name",
        "runtime_native",
        "--cfg",
        "reactor_stub",
        "--target",
        target,
    ]);
    cmd.arg(src).arg("-o").arg(archive).args([
        "-C",
        "opt-level=2",
        "-C",
        "debuginfo=0",
        "-C",
        "panic=abort",
        "-C",
        "codegen-units=1",
    ]);
    run(&mut cmd, "run rustc for runtime archive");
    assert!(archive.exists(), "runtime archive build failed");
}

// Host-CPU codegen flags safe to forward into the hermetic bundle build.
// RNX_RUNTIME_TARGET_CPU (e.g. "native") is the explicit opt-in used by
// benchmarking; an explicit parent RUSTFLAGS/CARGO_ENCODED_RUSTFLAGS
// -C target-cpu or -C target-feature is honored the same way. Everything
// else stays scrubbed so the archive is reproducible.
fn forwarded_cpu_flags() -> Vec<String> {
    let mut out = Vec::new();
    if let Ok(cpu) = std::env::var("RNX_RUNTIME_TARGET_CPU") {
        let cpu = cpu.trim();
        if !cpu.is_empty() {
            out.push(format!("-C target-cpu={cpu}"));
        }
    }
    for key in ["RUSTFLAGS", "CARGO_ENCODED_RUSTFLAGS"] {
        let val = std::env::var(key).unwrap_or_default();
        let mut it = val.split_whitespace().peekable();
        while let Some(arg) = it.next() {
            if arg == "-C" {
                if let Some(next) = it.next() {
                    if next.starts_with("target-cpu=") || next.starts_with("target-feature=") {
                        out.push("-C".to_string());
                        out.push(next.to_string());
                    }
                }
            } else if let Some(rest) = arg.strip_prefix("-C") {
                if rest.starts_with("target-cpu=") || rest.starts_with("target-feature=") {
                    out.push(arg.to_string());
                }
            } else if let Some(rest) = arg.strip_prefix("--target-cpu=") {
                out.push(format!("-C target-cpu={rest}"));
            }
        }
    }
    out.sort();
    out.dedup();
    out
}

// Native: build the hermetic bundle project with a nested cargo invocation
// and read exact artifact paths from its JSON messages. Never scans cargo's
// own artifact directories, so cold parallel builds cannot race.
fn build_native_archive(
    manifest_dir: &Path,
    archive: &Path,
    target: &str,
    target_dir: &Path,
) {
    let bundle_manifest = manifest_dir.join("bundle").join("Cargo.toml");
    let bundle_target_dir = target_dir.join("bundle");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let mut cmd = Command::new(&cargo);
    cmd.args([
        "build",
        "--manifest-path",
        &bundle_manifest.to_string_lossy(),
        "--target-dir",
        &bundle_target_dir.to_string_lossy(),
        "--target",
        target,
        "--profile",
        "release",
        "--message-format",
        "json-render-diagnostics",
        "--config",
        "profile.release.opt-level=3",
        "--config",
        "profile.release.codegen-units=1",
        "--config",
        "profile.release.panic=\"abort\"",
    ]);
    if std::env::var("RUSTC").is_ok() {
        cmd.env("RUSTC", std::env::var("RUSTC").unwrap());
    }
    if std::env::var("CARGO_NET_OFFLINE").as_deref() == Ok("true") {
        cmd.arg("--offline");
    }
    for key in [
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
        "CARGO_BUILD_TARGET",
        "CARGO_BUILD_TARGET_DIR",
        "CARGO_TARGET_DIR",
        "CARGO_INCREMENTAL",
    ] {
        cmd.env_remove(key);
    }
    let cpu_flags = forwarded_cpu_flags();
    if !cpu_flags.is_empty() {
        cmd.env("RUSTFLAGS", cpu_flags.join(" "));
    }
    let stdout = run_command_output(&mut cmd, "bundle cargo build");
    let mut artifacts: BTreeMap<String, PathBuf> = BTreeMap::new();
    let mut shared_lib: Option<PathBuf> = None;
    for line in stdout.lines() {
        let msg: serde_json::Value = match serde_json::from_str(line) {
            Ok(m) => m,
            Err(_) => continue,
        };
        if msg.get("reason").and_then(|r| r.as_str()) != Some("compiler-artifact") {
            continue;
        }
        let name = match msg
            .get("target")
            .and_then(|t| t.get("name"))
            .and_then(|n| n.as_str())
        {
            Some(n) => n,
            None => continue,
        };
        // MinGW staticlibs are `.a`, MSVC ones `.lib`; accept either.
        let want_native = name == "runtime_native";
        let path = msg
            .get("filenames")
            .and_then(|f| f.as_array())
            .into_iter()
            .flatten()
            .filter_map(|f| f.as_str())
            .find(|f| {
                if want_native {
                    f.ends_with(".a") || f.ends_with(".lib")
                } else {
                    f.ends_with(".rlib")
                }
            })
            .map(PathBuf::from);
        if let Some(p) = path {
            artifacts.insert(name.to_string(), p);
        }
        if name == "runtime_native" {
            if let Some(so) = msg
                .get("filenames")
                .and_then(|f| f.as_array())
                .into_iter()
                .flatten()
                .filter_map(|f| f.as_str())
                .find(|f| f.ends_with(".so") || f.ends_with(".dylib") || f.ends_with(".dll"))
                .map(PathBuf::from)
            {
                shared_lib = Some(so);
            }
        }
    }
    let native = artifacts.get("runtime_native").unwrap_or_else(|| {
        panic!("bundle build produced no runtime_native archive")
    });
    std::fs::copy(native, archive).expect("copy runtime archive to OUT_DIR");
    if let Some(so) = shared_lib {
        let file_name = so.file_name().expect("shared lib file name");
        std::fs::copy(&so, archive.with_file_name(file_name))
            .expect("copy runtime shared lib to OUT_DIR");
        if let Some(profile) = archive.parent().and_then(|o| profile_dir(o, target_dir)) {
            let _ = std::fs::copy(&so, profile.join(file_name));
        }
    }
    let mut rlibs: BTreeMap<&str, PathBuf> = BTreeMap::new();
    for dep in MERGE_DEPS {
        match artifacts.get(dep) {
            Some(p) => {
                rlibs.insert(dep, p.clone());
            }
            None => panic!("bundle build produced no rlib for {dep}"),
        }
    }
    merge_objects(archive, &rlibs);
}

fn merge_objects(archive: &Path, rlibs: &BTreeMap<&str, PathBuf>) {
    let tmp = archive.parent().unwrap().join("rt-merge");
    let _ = std::fs::remove_dir_all(&tmp);
    std::fs::create_dir_all(&tmp).expect("rt-merge dir");
    let mut merged = false;
    // Deterministic order: BTreeMap iteration is sorted by dep name.
    for rlib in rlibs.values() {
        let list = Command::new("ar")
            .arg("t")
            .arg(rlib)
            .output()
            .expect("ar t");
        assert!(list.status.success(), "ar t failed");
        let members: Vec<String> = String::from_utf8_lossy(&list.stdout)
            .lines()
            .filter(|m| !m.ends_with(".rmeta"))
            .map(|m| m.to_string())
            .collect();
        if members.is_empty() {
            continue;
        }
        let mut extract = Command::new("ar");
        extract.arg("x").arg(rlib);
        for m in &members {
            extract.arg(m);
        }
        extract.current_dir(&tmp);
        run(&mut extract, "ar x");
        merged = true;
    }
    if merged {
        let mut add = Command::new("ar");
        add.arg("r").arg(archive).current_dir(&tmp);
        let mut objects: Vec<_> = std::fs::read_dir(&tmp)
            .expect("read rt-merge")
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().map(|e| e == "o").unwrap_or(false))
            .collect();
        objects.sort();
        for o in &objects {
            add.arg(o.file_name().unwrap());
        }
        run(&mut add, "ar r");
        let mut ranlib = Command::new("ranlib");
        ranlib.arg(archive);
        run(&mut ranlib, "ranlib");
    }
    let _ = std::fs::remove_dir_all(&tmp);
}

fn main() {
    println!("cargo:rerun-if-changed=src/native");
    println!("cargo:rerun-if-changed=src/json_tape.rs");
    println!("cargo:rerun-if-changed=src/json_scanner.rs");
    println!("cargo:rerun-if-changed=src/reactor.rs");
    println!("cargo:rerun-if-changed=src/reactor_wasm.rs");
    println!("cargo:rerun-if-changed=bundle/Cargo.toml");
    let out = PathBuf::from(std::env::var("OUT_DIR").expect("OUT_DIR"));
    let archive = out.join("libruntime_native.a");
    let manifest = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("manifest"));
    let src = manifest.join("src").join("native").join("mod.rs");
    let target = std::env::var("TARGET").unwrap_or_default();
    if target.starts_with("wasm32") {
        build_wasm_archive(&src, &archive, &target);
    } else {
        let target_dir =
            workspace_target_dir(&out).expect("workspace target dir above OUT_DIR");
        build_native_archive(&manifest, &archive, &target, &target_dir);
    }
    assert!(archive.exists(), "runtime archive build failed");
}
