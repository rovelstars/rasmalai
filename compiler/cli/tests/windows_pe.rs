#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TRIPLE: &str = "x86_64-pc-windows-gnu";

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

fn assemble(dir: &Path, name: &str, src: &str) -> PathBuf {
    let mc = tool("llvm-mc").expect("llvm-mc checked by caller");
    let source = dir.join(format!("{name}.s"));
    let object = dir.join(format!("{name}.o"));
    std::fs::write(&source, src).unwrap();
    let out = Command::new(mc)
        .args(["-triple=x86_64-pc-windows-gnu", "-filetype=obj", "-o"])
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

fn import_library(dir: &Path, dll: &str, exports: &[&str]) -> PathBuf {
    let dlltool = tool("llvm-dlltool").expect("llvm-dlltool checked by caller");
    let mut def = format!("LIBRARY {dll}\nEXPORTS\n");
    for export in exports {
        def.push_str(export);
        def.push('\n');
    }
    let def_path = dir.join(format!("{dll}.def"));
    std::fs::write(&def_path, def).unwrap();
    let lib = dir.join(format!("{dll}.lib"));
    let out = Command::new(dlltool)
        .args(["-m", "i386:x86-64", "-d"])
        .arg(&def_path)
        .arg("-l")
        .arg(&lib)
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "llvm-dlltool: {}",
        String::from_utf8_lossy(&out.stderr)
    );
    lib
}

fn runtime_stub(dir: &Path, symbols: &[&str]) -> PathBuf {
    let mut src = String::from(".intel_syntax noprefix\n.text\n");
    for symbol in symbols {
        src.push_str(&format!(
            ".globl {symbol}\n{symbol}:\n    xor eax, eax\n    ret\n"
        ));
    }
    let object = assemble(dir, "runtime_stub", &src);
    archive(dir, "libruntime_native.a", &[object])
}

struct Pack {
    dir: PathBuf,
    digest: String,
}

fn write_pack(root: &Path, crt: &Path, runtime: &Path, import_lib: &Path) -> Pack {
    let lib = root.join("lib").join("rustlib").join(TRIPLE).join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::copy(crt, lib.join("crt2.o")).unwrap();
    std::fs::copy(runtime, lib.join("libruntime_native.a")).unwrap();
    std::fs::copy(import_lib, lib.join("libkernel32.a")).unwrap();
    let sha = sysroot::pack::file_sha256(&lib.join("libruntime_native.a")).unwrap();
    let manifest = |digest: &str| {
        format!(
            "{{\"version\": 2, \"target\": \"{TRIPLE}\", \"arch\": \"x86_64\", \"os\": \"windows\", \"env\": \"gnu\", \"object\": \"coff\", \"libc\": \"mingw\", \"pointer_width\": 64, \"endian\": \"little\", \"tier\": 2, \"capabilities\": [\"alloc\", \"console\", \"clock\", \"fs\", \"log\", \"net\", \"process\", \"random\", \"threads\"], \"cpu\": {{\"model\": \"x86-64\", \"features\": \"\"}}, \"loader\": null, \"kernel_min\": null, \"glibc_baseline\": null, \"prelude\": \"@std/prelude\", \"link\": {{\"search\": [\"lib/rustlib/{TRIPLE}/lib\"], \"crt\": {{\"static\": [\"crt2.o\"]}}, \"crt_end\": {{}}, \"libs\": {{\"kernel32\": \"libkernel32.a\"}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": \"{sha}\"}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"{digest}\"}}}}"
        )
    };
    std::fs::write(
        root.join("sysroot.json"),
        manifest(&format!("sha256:{}", "0".repeat(64))),
    )
    .unwrap();
    let digest = sysroot::pack::Pack::digest(root).unwrap();
    std::fs::write(root.join("sysroot.json"), manifest(&digest)).unwrap();
    Pack {
        dir: root.to_path_buf(),
        digest,
    }
}

fn project(tag: &str) -> (crate::scratch::Scratch, PathBuf) {
    let root = crate::scratch::dir(&format!("pe-proj-{tag}"));
    std::fs::write(
        root.join("Project.config"),
        "export default {\n    project: {\n        name: \"pe-probe\",\n        version: \"0.1.0\"\n    }\n}\n",
    )
    .unwrap();
    let src = root.join("src").join("main.rnx");
    std::fs::create_dir_all(src.parent().unwrap()).unwrap();
    std::fs::write(&src, "fn Main(): Int {\n    return 42;\n}\n").unwrap();
    (root, src)
}

fn machine(path: &Path) -> u16 {
    let bytes = std::fs::read(path).unwrap();
    let pe_at = u32::from_le_bytes([bytes[0x3c], bytes[0x3d], bytes[0x3e], bytes[0x3f]]) as usize;
    u16::from_le_bytes([bytes[pe_at + 4], bytes[pe_at + 5]])
}

#[test]
fn windows_pack_builds_a_pe_image() {
    for required in ["llvm-mc", "llvm-ar", "llvm-dlltool", "llvm-readobj"] {
        if tool(required).is_none() {
            eprintln!("skipping `windows_pack_builds_a_pe_image`: {required} is absent");
            return;
        }
    }
    let scratch = crate::scratch::dir("win-pack");
    let crt = assemble(
        &scratch,
        "crt2",
        r#"
.intel_syntax noprefix
.text
.globl mainCRTStartup
mainCRTStartup:
    sub rsp, 40
    call main
    mov ecx, eax
    call ExitProcess
    add rsp, 40
    ret
"#,
    );
    let import = import_library(&scratch, "kernel32.dll", &["ExitProcess"]);
    let runtime = runtime_stub(
        &scratch,
        &[
            "__main",
            "rnx_alloc",
            "rnx_any_release",
            "rnx_any_retain",
            "rnx_array_get",
            "rnx_error_take",
            "rnx_fatal_span",
            "rnx_genref_invalidate",
            "rnx_gmap_free",
            "rnx_gmap_keys",
            "rnx_heap_track",
            "rnx_init",
            "rnx_note_fields",
            "rnx_note_type",
            "rnx_release",
            "rnx_release_array",
            "rnx_report_fatal",
            "rnx_report_uncaught",
            "rnx_retain",
            "rnx_set_assert_strict",
        ],
    );
    let pack_root = scratch.join("pack");
    let pack = write_pack(&pack_root, &crt, &runtime, &import);
    let cache = crate::scratch::dir("win-cache");
    let added = run(
        &scratch,
        &cache,
        &["sysroot", "add", "--local", pack.dir.to_str().unwrap()],
    );
    assert!(
        added.status.success(),
        "{}",
        String::from_utf8_lossy(&added.stderr)
    );
    assert_eq!(
        sysroot::pack::Pack::load(&pack.dir)
            .unwrap()
            .manifest
            .integrity
            .digest,
        pack.digest
    );

    let (root, main) = project("build");
    let build = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            TRIPLE,
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
    assert_eq!(machine(&exe), 0x8664);
    let readobj = tool("llvm-readobj").unwrap();
    let report = Command::new(&readobj)
        .args(["--file-headers", "--sections", "--coff-imports"])
        .arg(&exe)
        .output()
        .unwrap();
    assert!(
        report.status.success(),
        "llvm-readobj: {}",
        String::from_utf8_lossy(&report.stderr)
    );
    let text = String::from_utf8_lossy(&report.stdout);
    assert!(
        text.contains("Subsystem: IMAGE_SUBSYSTEM_WINDOWS_CUI"),
        "{text}"
    );
    assert!(text.contains("Symbol: ExitProcess"), "{text}");
    assert!(text.contains("kernel32.dll"), "{text}");
    assert!(text.contains("Name: .pdata"), "{text}");
    assert!(text.contains("Name: .xdata"), "{text}");

    let first = std::fs::read(&exe).unwrap();
    let artifact_path = PathBuf::from(&artifact);
    let stamp = artifact_path.with_file_name(format!(
        "{}.fingerprint",
        artifact_path.file_name().unwrap().to_string_lossy()
    ));
    std::fs::remove_file(&artifact_path).unwrap();
    let _ = std::fs::remove_file(stamp);
    let rebuild = run(
        &root,
        &cache,
        &[
            "build",
            main.to_str().unwrap(),
            "--target",
            TRIPLE,
            "--release",
        ],
    );
    assert!(
        rebuild.status.success(),
        "{}",
        String::from_utf8_lossy(&rebuild.stderr)
    );
    assert_eq!(
        first,
        std::fs::read(&exe).unwrap(),
        "two PE builds are not identical"
    );
}

#[test]
fn missing_pack_file_is_e124() {
    for required in ["llvm-mc", "llvm-ar", "llvm-dlltool"] {
        if tool(required).is_none() {
            eprintln!("skipping `missing_pack_file_is_e124`: {required} is absent");
            return;
        }
    }
    let scratch = crate::scratch::dir("win-missing");
    let crt = assemble(
        &scratch,
        "crt2",
        ".intel_syntax noprefix\n.text\n.globl mainCRTStartup\nmainCRTStartup:\n    ret\n",
    );
    let import = import_library(&scratch, "kernel32.dll", &["ExitProcess"]);
    let runtime = runtime_stub(&scratch, &["__main"]);
    let root = scratch.join("broken-pack");
    let lib = root.join("lib").join("rustlib").join(TRIPLE).join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    std::fs::copy(&crt, lib.join("crt2.o")).unwrap();
    std::fs::copy(&runtime, lib.join("libruntime_native.a")).unwrap();
    std::fs::copy(&import, lib.join("libkernel32.a")).unwrap();
    let sha = sysroot::pack::file_sha256(&lib.join("libruntime_native.a")).unwrap();
    let body = format!(
        "{{\"version\": 2, \"target\": \"{TRIPLE}\", \"arch\": \"x86_64\", \"os\": \"windows\", \"env\": \"gnu\", \"object\": \"coff\", \"libc\": \"mingw\", \"pointer_width\": 64, \"endian\": \"little\", \"tier\": 2, \"capabilities\": [\"alloc\", \"console\", \"clock\", \"fs\", \"log\", \"net\", \"process\", \"random\", \"threads\"], \"cpu\": {{\"model\": \"x86-64\", \"features\": \"\"}}, \"loader\": null, \"kernel_min\": null, \"glibc_baseline\": null, \"prelude\": \"@std/prelude\", \"link\": {{\"search\": [\"lib/rustlib/{TRIPLE}/lib\"], \"crt\": {{\"static\": [\"missing.o\"]}}, \"crt_end\": {{}}, \"libs\": {{}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": \"{sha}\"}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"sha256:{}\"}}}}",
        "0".repeat(64)
    );
    std::fs::write(root.join("sysroot.json"), &body).unwrap();
    let digest = sysroot::pack::Pack::digest(&root).unwrap();
    std::fs::write(
        root.join("sysroot.json"),
        body.replace(&"0".repeat(64), &digest[7..]),
    )
    .unwrap();
    let cache = crate::scratch::dir("win-missing-cache");
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
