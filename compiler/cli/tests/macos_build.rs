#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

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

fn run(dir: &Path, cache: &Path, args: &[&str]) -> Output {
    Command::new(env!("CARGO_BIN_EXE_rnx"))
        .args(args)
        .current_dir(dir)
        .env("RNX_CACHE_HOME", cache)
        .env(
            "RNX_VENDOR_DIR",
            format!("{}/../stdlib/src", env!("CARGO_MANIFEST_DIR")),
        )
        .env("RNX_REGISTRY", "http://127.0.0.1:9")
        .env_remove("RNX_SYSROOT")
        .env_remove("RNX_OFFLINE")
        .env_remove("CI")
        .output()
        .unwrap()
}

fn assemble(dir: &Path, name: &str, triple: &str, src: &str) -> PathBuf {
    let mc = tool("llvm-mc").expect("llvm-mc checked by caller");
    let source = dir.join(format!("{name}.s"));
    let object = dir.join(format!("{name}.o"));
    std::fs::write(&source, src).unwrap();
    let out = Command::new(mc)
        .arg(format!("-triple={triple}"))
        .args(["-filetype=obj", "-o"])
        .arg(&object)
        .arg(&source)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "assemble {name}: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    object
}

fn archive(dir: &Path, name: &str, objects: &[PathBuf]) -> PathBuf {
    let ar = tool("llvm-ar").expect("llvm-ar checked by caller");
    let lib = dir.join(name);
    let out = Command::new(ar)
        .arg("cr")
        .arg(&lib)
        .args(objects)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "llvm-ar: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    lib
}

const TBD: &str = "--- !tapi-tbd\ntbd-version: 4\ntargets: [ arm64-macos, x86_64-macos ]\ninstall-name: /usr/lib/libSystem.B.dylib\nexports:\n  - targets: [ arm64-macos, x86_64-macos ]\n    symbols: [ _exit, _write ]\n...\n";

struct Fixture {
    scratch: crate::scratch::Scratch,
    pack: PathBuf,
    digest: String,
}

fn build_pack(tag: &str, triple: &str, arm: bool) -> Fixture {
    let scratch = crate::scratch::dir(&format!("macos-pack-{tag}"));
    let crt_src = if arm {
        ".text\n.globl _start\n_start:\n    bl _main\n    bl _exit\n"
    } else {
        ".intel_syntax noprefix\n.text\n.globl _start\n_start:\n    call _main\n    call _exit\n"
    };
    let crt = assemble(&scratch, "crt1", triple, crt_src);
    let mut stub_src = if arm {
        ".text\n".to_string()
    } else {
        ".intel_syntax noprefix\n.text\n".to_string()
    };
    for symbol in [
        "_rnx_alloc",
        "_rnx_any_release",
        "_rnx_any_retain",
        "_rnx_array_get",
        "_rnx_error_take",
        "_rnx_fatal_span",
        "_rnx_genref_invalidate",
        "_rnx_gmap_free",
        "_rnx_gmap_keys",
        "_rnx_heap_track",
        "_rnx_init",
        "_rnx_note_fields",
        "_rnx_note_type",
        "_rnx_release",
        "_rnx_release_array",
        "_rnx_report_fatal",
        "_rnx_report_uncaught",
        "_rnx_retain",
        "_rnx_set_assert_strict",
    ] {
        if arm {
            stub_src.push_str(&format!(".globl {symbol}\n{symbol}:\n    ret\n"));
        } else {
            stub_src.push_str(&format!(
                ".globl {symbol}\n{symbol}:\n    xor eax, eax\n    ret\n"
            ));
        }
    }
    let stub = assemble(&scratch, "runtime_stub", triple, &stub_src);
    let runtime = archive(&scratch, "libruntime_native.a", &[stub]);
    let root = scratch.join("pack");
    let lib = root.join("lib").join("rustlib").join(triple).join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::copy(&crt, lib.join("crt1.o")).unwrap();
    std::fs::copy(&runtime, lib.join("libruntime_native.a")).unwrap();
    std::fs::write(lib.join("libSystem.tbd"), TBD).unwrap();
    let sha = sysroot::pack::file_sha256(&lib.join("libruntime_native.a")).unwrap();
    let arch = triple.split('-').next().unwrap();
    let manifest = |digest: &str| {
        format!(
            "{{\"version\": 2, \"target\": \"{triple}\", \"arch\": \"{arch}\", \"os\": \"macos\", \"env\": \"none\", \"object\": \"macho\", \"libc\": \"libsystem\", \"pointer_width\": 64, \"endian\": \"little\", \"tier\": 2, \"capabilities\": [\"alloc\", \"console\", \"clock\", \"fs\", \"log\", \"net\", \"process\", \"random\", \"threads\"], \"cpu\": {{\"model\": \"generic\", \"features\": \"\"}}, \"loader\": \"/usr/lib/dyld\", \"kernel_min\": null, \"glibc_baseline\": null, \"prelude\": \"@std/prelude\", \"link\": {{\"search\": [\"lib/rustlib/{triple}/lib\"], \"crt\": {{\"static\": [\"crt1.o\"]}}, \"crt_end\": {{}}, \"libs\": {{\"System\": \"libSystem.tbd\"}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": \"{sha}\"}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"{digest}\"}}}}"
        )
    };
    std::fs::write(
        root.join("sysroot.json"),
        manifest(&format!("sha256:{}", "0".repeat(64))),
    )
    .unwrap();
    let digest = sysroot::pack::Pack::digest(&root).unwrap();
    std::fs::write(root.join("sysroot.json"), manifest(&digest)).unwrap();
    Fixture {
        scratch,
        pack: root,
        digest,
    }
}

fn project(tag: &str) -> (crate::scratch::Scratch, PathBuf) {
    let root = crate::scratch::dir(&format!("macos-proj-{tag}"));
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: {\n        name: \"mac-probe\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let src = root.join("src").join("main.rnx");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, "fn Main(): Int {\n    return 42;\n}\n").unwrap();
    (root, src)
}

fn build_and_inspect(tag: &str, triple: &str, arm: bool) {
    for required in ["llvm-mc", "llvm-ar", "llvm-objdump"] {
        if tool(required).is_none() {
            eprintln!("skipping `{tag}`: {required} is absent");
            return;
        }
    }
    let fixture = build_pack(tag, triple, arm);
    let cache = crate::scratch::dir(&format!("macos-cache-{tag}"));
    let added = run(
        &fixture.scratch,
        &cache,
        &["sysroot", "add", "--local", fixture.pack.to_str().unwrap()],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert_eq!(
        sysroot::pack::Pack::load(&fixture.pack)
            .unwrap()
            .manifest
            .integrity
            .digest,
        fixture.digest
    );
    let (root, main) = project(tag);
    let emit = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            triple,
            "--emit-obj",
        ],
    );
    assert!(
        emit.status.success(),
        "{}",
        String::from_utf8_lossy(&emit.stderr)
    );
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            triple,
            "--release",
        ],
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
    let objdump = tool("llvm-objdump").unwrap();
    let report = Command::new(&objdump)
        .args(["--macho", "--private-headers"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(report.status.success());
    let text = String::from_utf8_lossy(&report.stdout);
    assert!(text.contains("MH_MAGIC_64"), "{text}");
    assert!(text.contains("EXECUTE"), "{text}");
    assert!(
        text.contains(if arm { "ARM64" } else { "X86_64" }),
        "{text}"
    );
    assert!(text.contains("LC_MAIN"), "{text}");
    assert!(text.contains("LC_LOAD_DYLIB"), "{text}");
    assert!(text.contains("/usr/lib/libSystem.B.dylib"), "{text}");
    assert!(text.contains("LC_CODE_SIGNATURE"), "{text}");
    assert!(text.contains("LC_DYLD_INFO_ONLY"), "{text}");
    assert!(text.contains("segname __PAGEZERO"), "{text}");
    assert!(text.contains("segname __DATA"), "{text}");

    let first = std::fs::read(&exe).unwrap();
    let stamp = exe.with_file_name(format!(
        "{}.fingerprint",
        exe.file_name().unwrap().to_string_lossy()
    ));
    std::fs::remove_file(&exe).unwrap();
    let _ = std::fs::remove_file(stamp);
    let rebuild = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            triple,
            "--release",
        ],
    );
    assert!(
        rebuild.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuild.stderr)
    );
    assert_eq!(first, std::fs::read(&exe).unwrap());
}

#[test]
fn arm64_pack_builds_a_macho_image() {
    build_and_inspect("arm", "aarch64-apple-darwin", true);
}

#[test]
fn x86_64_pack_builds_a_macho_image() {
    build_and_inspect("x86", "x86_64-apple-darwin", false);
}

#[test]
fn missing_pack_file_is_e124() {
    for required in ["llvm-mc", "llvm-ar"] {
        if tool(required).is_none() {
            eprintln!("skipping: {required} is absent");
            return;
        }
    }
    let scratch = crate::scratch::dir("macos-missing");
    let triple = "aarch64-apple-darwin";
    let crt = assemble(
        &scratch,
        "crt1",
        triple,
        ".text\n.globl _start\n_start:\n    ret\n",
    );
    let stub = assemble(
        &scratch,
        "stub",
        triple,
        ".text\n.globl _rnx_init\n_rnx_init:\n    ret\n",
    );
    let runtime = archive(&scratch, "libruntime_native.a", &[stub]);
    let root = scratch.join("pack");
    let lib = root.join("lib").join("rustlib").join(triple).join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::copy(&crt, lib.join("crt1.o")).unwrap();
    std::fs::copy(&runtime, lib.join("libruntime_native.a")).unwrap();
    std::fs::write(lib.join("libSystem.tbd"), TBD).unwrap();
    let sha = sysroot::pack::file_sha256(&lib.join("libruntime_native.a")).unwrap();
    let body = format!(
        "{{\"version\": 2, \"target\": \"{triple}\", \"arch\": \"aarch64\", \"os\": \"macos\", \"env\": \"none\", \"object\": \"macho\", \"libc\": \"libsystem\", \"pointer_width\": 64, \"endian\": \"little\", \"tier\": 2, \"capabilities\": [\"alloc\", \"console\", \"clock\", \"fs\", \"log\", \"net\", \"process\", \"random\", \"threads\"], \"cpu\": {{\"model\": \"generic\", \"features\": \"\"}}, \"loader\": \"/usr/lib/dyld\", \"kernel_min\": null, \"glibc_baseline\": null, \"prelude\": \"@std/prelude\", \"link\": {{\"search\": [\"lib/rustlib/{triple}/lib\"], \"crt\": {{\"static\": [\"missing.o\"]}}, \"crt_end\": {{}}, \"libs\": {{\"System\": \"libSystem.tbd\"}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": \"{sha}\"}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"sha256:{}\"}}}}",
        "0".repeat(64)
    );
    std::fs::write(root.join("sysroot.json"), &body).unwrap();
    let digest = sysroot::pack::Pack::digest(&root).unwrap();
    std::fs::write(
        root.join("sysroot.json"),
        body.replace(&"0".repeat(64), &digest[7..]),
    )
    .unwrap();
    let cache = crate::scratch::dir("macos-missing-cache");
    let added = run(
        &scratch,
        &cache,
        &["sysroot", "add", "--local", root.to_str().unwrap()],
    );
    assert_eq!(added.status.code(), Some(1));
    let text = String::from_utf8_lossy(&added.stderr);
    assert!(text.contains("E124"), "{text}");
    assert!(text.contains("missing.o"), "{text}");
}
