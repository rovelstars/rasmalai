//! End-to-end WASI check: build the target runtime archive, stage a target
//! pack, link a real module with the in-house wasm linker, and run it under
//! a preview 1 host.
//!
//! The test skips with a printed reason when the runner lacks the pieces:
//! the `wasm32-unknown-unknown` Rust target, Node, or Deno. The harness
//! drives the test binary cargo built; the release gate in
//! `requirements/11_TOOLING.md` names `target/release/rnx` explicitly.

#[cfg(test)]
#[path = "../../testkit/scratch.rs"]
mod scratch;

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

const TRIPLE: &str = "wasm32-unknown-wasi";
const BANNER: &str = "hello wasi";
const RUNNER: &str = r#"import { WASI } from "node:wasi";
import { readFile } from "node:fs/promises";
const wasi = new WASI({ version: "preview1", args: [], env: {} });
const bytes = await readFile(process.argv[2]);
const module = await WebAssembly.compile(bytes);
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exit(wasi.start(instance));
"#;

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

fn rnx(dir: &Path, cache: &Path, args: &[&str]) -> Output {
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
    let manifest = repo().join("compiler/runtime/wasi/Cargo.toml");
    let target_dir = scratch.join("runtime-target");
    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".to_string());
    let out = Command::new(cargo)
        .args(["build", "--manifest-path"])
        .arg(&manifest)
        .args([
            "--target",
            "wasm32-unknown-unknown",
            "--release",
            "--offline",
            "--target-dir",
        ])
        .arg(&target_dir)
        .env_remove("RUSTFLAGS")
        .env_remove("CARGO_ENCODED_RUSTFLAGS")
        .env_remove("CARGO_BUILD_TARGET")
        .env_remove("CARGO_TARGET_DIR")
        .output()
        .expect("spawn cargo for the WASI runtime archive");
    assert!(
        out.status.success(),
        "runtime archive build failed:\n{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let archive = target_dir
        .join("wasm32-unknown-unknown")
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
    let lib = root.join("lib").join("rustlib").join(TRIPLE).join("lib");
    std::fs::create_dir_all(&lib).unwrap();
    let runtime = lib.join("libruntime_native.a");
    std::fs::copy(archive, &runtime).unwrap();
    let sha = sysroot::pack::file_sha256(&runtime).unwrap();
    let manifest = |digest: &str| {
        format!(
            "{{\"version\": 2, \"target\": \"{TRIPLE}\", \"arch\": \"wasm32\", \"os\": \"wasi\", \"env\": \"none\", \"object\": \"wasm\", \"libc\": \"none\", \"pointer_width\": 32, \"endian\": \"little\", \"tier\": 3, \"capabilities\": [\"alloc\", \"console\", \"clock\", \"fs\", \"log\", \"random\"], \"cpu\": {{\"model\": \"generic\", \"features\": \"\"}}, \"loader\": null, \"kernel_min\": null, \"glibc_baseline\": null, \"prelude\": \"@std/wasi\", \"link\": {{\"search\": [\"lib/rustlib/{TRIPLE}/lib\"], \"crt\": {{}}, \"crt_end\": {{}}, \"libs\": {{}}, \"builtins\": null, \"runtime\": \"libruntime_native.a\"}}, \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": \"{sha}\"}}, \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"{digest}\"}}}}"
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

fn project(root: &Path, source: &str) -> PathBuf {
    std::fs::write(
        root.join("Project.config"),
        format!(
            "export default {{\n    project: {{\n        name: \"wasi-probe\",\n        version: \"0.1.0\",\n        targets: [\"{TRIPLE}\"]\n    }}\n}}\n"
        ),
    )
    .unwrap();
    let main = root.join("src").join("main.rnx");
    std::fs::create_dir_all(main.parent().unwrap()).unwrap();
    std::fs::write(&main, source).unwrap();
    main
}

fn run_guest(host: &Path, runner: &Path, module: &Path, extra: &[&str]) -> Output {
    Command::new(host)
        .args(extra)
        .arg(runner)
        .arg(module)
        .output()
        .expect("spawn the WASI host")
}

#[test]
fn wasi_pack_builds_and_runs_under_a_preview1_host() {
    let Some(node) = tool("node") else {
        return eprintln!(
            "skipping `wasi_pack_builds_and_runs_under_a_preview1_host`: node is absent"
        );
    };
    if !rust_target_installed("wasm32-unknown-unknown") {
        return eprintln!(
            "skipping `wasi_pack_builds_and_runs_under_a_preview1_host`: rustup target wasm32-unknown-unknown is absent"
        );
    }

    let work = crate::scratch::dir("wasi-run");
    let archive = build_runtime_archive(&work);
    let pack_root = work.join("pack");
    let digest = stage_pack(&pack_root, &archive);
    let cache = crate::scratch::dir("wasi-cache");
    let added = rnx(
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

    let runner = work.join("runner.mjs");
    std::fs::write(&runner, RUNNER).unwrap();

    let root = crate::scratch::dir("wasi-project");
    let main = project(
        &root,
        "fn Main(): Int {\n    println!(\"hello wasi\");\n    return 0;\n}\n",
    );
    let built = rnx(
        &root,
        &cache,
        &[
            "build",
            "--target",
            TRIPLE,
            "--release",
            main.to_str().unwrap(),
        ],
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let module = root
        .join(".rnx-cache")
        .join("build")
        .join("release")
        .join("wasi-probe.wasm");
    assert!(module.is_file(), "artifact missing at {}", module.display());
    let bytes = std::fs::read(&module).unwrap();
    assert_eq!(&bytes[..4], b"\0asm", "artifact is a WebAssembly module");

    let node_run = run_guest(&node, &runner, &module, &[]);
    assert_eq!(
        String::from_utf8_lossy(&node_run.stdout),
        format!("{BANNER}\n"),
        "stderr: {}",
        String::from_utf8_lossy(&node_run.stderr)
    );
    assert_eq!(node_run.status.code(), Some(0));

    if let Some(deno) = tool("deno") {
        let deno_run = run_guest(&deno, &runner, &module, &["run", "--allow-read"]);
        assert_eq!(
            String::from_utf8_lossy(&deno_run.stdout),
            format!("{BANNER}\n"),
            "deno stderr: {}",
            String::from_utf8_lossy(&deno_run.stderr)
        );
        assert_eq!(deno_run.status.code(), Some(0));
    } else {
        eprintln!("deno is absent: only node ran the module");
    }

    let exit_root = crate::scratch::dir("wasi-exit");
    let exit_main = project(
        &exit_root,
        "fn Main(): Int {\n    wasi_println!(\"count\", 42, true);\n    eprintln!(\"to stderr\");\n    return 3;\n}\n",
    );
    let exit_build = rnx(
        &exit_root,
        &cache,
        &[
            "build",
            "--target",
            TRIPLE,
            "--release",
            exit_main.to_str().unwrap(),
        ],
    );
    assert!(
        exit_build.status.success(),
        "{}",
        String::from_utf8_lossy(&exit_build.stderr)
    );
    let exit_module = exit_root
        .join(".rnx-cache")
        .join("build")
        .join("release")
        .join("wasi-probe.wasm");
    let exit_run = run_guest(&node, &runner, &exit_module, &[]);
    assert_eq!(
        String::from_utf8_lossy(&exit_run.stdout),
        "count 42 true\n"
    );
    assert!(
        String::from_utf8_lossy(&exit_run.stderr).contains("to stderr"),
        "stderr: {}",
        String::from_utf8_lossy(&exit_run.stderr)
    );
    assert_eq!(exit_run.status.code(), Some(3));
}

#[test]
fn emit_object_needs_no_target_pack() {
    if !rust_target_installed("wasm32-unknown-unknown") {
        return eprintln!(
            "skipping `emit_object_needs_no_target_pack`: rustup target wasm32-unknown-unknown is absent"
        );
    }
    let work = crate::scratch::dir("wasi-emit-obj");
    let root = crate::scratch::dir("wasi-emit-project");
    let main = project(
        &root,
        "fn Main(): Int {\n    println!(\"hello wasi\");\n    return 0;\n}\n",
    );
    let cache = crate::scratch::dir("wasi-emit-cache");
    let built = rnx(
        &root,
        &cache,
        &[
            "build",
            "--target",
            TRIPLE,
            "--release",
            "--emit-obj",
            main.to_str().unwrap(),
        ],
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );
    let object = root
        .join(".rnx-cache")
        .join("build")
        .join("release")
        .join("wasi-probe.o");
    assert!(object.is_file(), "object missing at {}", object.display());
    let bytes = std::fs::read(&object).unwrap();
    assert_eq!(&bytes[..4], b"\0asm", "object is a wasm binary");
    let text = String::from_utf8_lossy(&bytes);
    assert!(
        text.contains("linking"),
        "object carries the linking metadata section"
    );
    assert!(
        text.contains("reloc.CODE"),
        "object carries code relocations"
    );
    let _ = work;
}

#[test]
fn in_house_output_matches_wasm_ld() {
    let Some(node) = tool("node") else {
        return eprintln!("skipping `in_house_output_matches_wasm_ld`: node is absent");
    };
    let Some(wasm_ld) = tool("wasm-ld") else {
        return eprintln!(
            "skipping `in_house_output_matches_wasm_ld`: wasm-ld is absent (it is only a differential reference)"
        );
    };
    if !rust_target_installed("wasm32-unknown-unknown") {
        return eprintln!(
            "skipping `in_house_output_matches_wasm_ld`: rustup target wasm32-unknown-unknown is absent"
        );
    }

    let work = crate::scratch::dir("wasi-diff");
    let archive = build_runtime_archive(&work);
    let pack_root = work.join("pack");
    stage_pack(&pack_root, &archive);
    let cache = crate::scratch::dir("wasi-diff-cache");
    let added = rnx(
        &work,
        &cache,
        &["sysroot", "add", "--local", pack_root.to_str().unwrap()],
    );
    assert!(added.status.success());

    let root = crate::scratch::dir("wasi-diff-project");
    let main = project(
        &root,
        "fn Main(): Int {\n    println!(\"hello wasi\");\n    return 0;\n}\n",
    );
    let obj = rnx(
        &root,
        &cache,
        &[
            "build",
            "--target",
            TRIPLE,
            "--release",
            "--emit-obj",
            main.to_str().unwrap(),
        ],
    );
    assert!(
        obj.status.success(),
        "{}",
        String::from_utf8_lossy(&obj.stderr)
    );
    let object = root
        .join(".rnx-cache")
        .join("build")
        .join("release")
        .join("wasi-probe.o");
    let ours = root
        .join(".rnx-cache")
        .join("build")
        .join("release")
        .join("wasi-probe.wasm");
    let built = rnx(
        &root,
        &cache,
        &[
            "build",
            "--target",
            TRIPLE,
            "--release",
            main.to_str().unwrap(),
        ],
    );
    assert!(
        built.status.success(),
        "{}",
        String::from_utf8_lossy(&built.stderr)
    );

    let reference = work.join("reference.wasm");
    let link = Command::new(&wasm_ld)
        .arg("-o")
        .arg(&reference)
        .arg(&object)
        .arg(&archive)
        .output()
        .expect("spawn wasm-ld");
    assert!(
        link.status.success(),
        "wasm-ld: {}",
        String::from_utf8_lossy(&link.stderr)
    );

    let runner = work.join("runner.mjs");
    std::fs::write(&runner, RUNNER).unwrap();
    let ours_run = run_guest(&node, &runner, &ours, &[]);
    let reference_run = run_guest(&node, &runner, &reference, &[]);
    assert_eq!(
        String::from_utf8_lossy(&ours_run.stdout),
        format!("{BANNER}\n")
    );
    assert_eq!(
        String::from_utf8_lossy(&reference_run.stdout),
        String::from_utf8_lossy(&ours_run.stdout),
        "both links print the same line"
    );
    assert_eq!(ours_run.status.code(), reference_run.status.code());

    let ours_shape = module_shape(&std::fs::read(&ours).unwrap());
    let reference_shape = module_shape(&std::fs::read(&reference).unwrap());
    assert_eq!(ours_shape.imports, reference_shape.imports, "import names");
    assert_eq!(ours_shape.exports, reference_shape.exports, "export names");
    assert!(
        ours_shape.memory_pages >= reference_shape.memory_pages,
        "the in-house link keeps more of the archive, so its initial memory covers at least the reference: {} < {}",
        ours_shape.memory_pages,
        reference_shape.memory_pages
    );
    assert!(
        ours_shape.functions >= reference_shape.functions,
        "no dead-code elimination yet: {} < {}",
        ours_shape.functions,
        reference_shape.functions
    );
}

struct Shape {
    imports: Vec<(String, String)>,
    exports: Vec<String>,
    memory_pages: u32,
    functions: u32,
}

fn module_shape(bytes: &[u8]) -> Shape {
    let mut shape = Shape {
        imports: Vec::new(),
        exports: Vec::new(),
        memory_pages: 0,
        functions: 0,
    };
    let mut pos = 8usize;
    while pos < bytes.len() {
        let id = bytes[pos];
        pos += 1;
        let (size, used) = read_uleb(&bytes[pos..]);
        pos += used;
        let end = pos + size as usize;
        match id {
            2 => {
                let (count, used) = read_uleb(&bytes[pos..]);
                let mut at = pos + used;
                for _ in 0..count {
                    let (module, used) = read_name(&bytes[at..]);
                    at += used;
                    let (name, used) = read_name(&bytes[at..]);
                    at += used;
                    let kind = bytes[at];
                    at += 1;
                    match kind {
                        0 => {
                            let (_, used) = read_uleb(&bytes[at..]);
                            at += used;
                        }
                        1 => {
                            at += 1;
                            let (flags, used) = read_uleb(&bytes[at..]);
                            at += used;
                            let (_, used) = read_uleb(&bytes[at..]);
                            at += used;
                            if flags & 1 != 0 {
                                let (_, used) = read_uleb(&bytes[at..]);
                                at += used;
                            }
                        }
                        2 => {
                            let (flags, used) = read_uleb(&bytes[at..]);
                            at += used;
                            let (_, used) = read_uleb(&bytes[at..]);
                            at += used;
                            if flags & 1 != 0 {
                                let (_, used) = read_uleb(&bytes[at..]);
                                at += used;
                            }
                        }
                        3 => {
                            at += 1;
                            let (_, used) = read_uleb(&bytes[at..]);
                            at += used;
                        }
                        _ => panic!("import kind {kind}"),
                    }
                    shape.imports.push((module, name));
                }
            }
            3 => {
                let (count, _) = read_uleb(&bytes[pos..]);
                shape.functions = count;
            }
            5 => {
                let (count, used) = read_uleb(&bytes[pos..]);
                assert_eq!(count, 1, "one memory");
                let mut at = pos + used;
                let (_, used) = read_uleb(&bytes[at..]);
                at += used;
                let (pages, _) = read_uleb(&bytes[at..]);
                shape.memory_pages = pages;
            }
            7 => {
                let (count, used) = read_uleb(&bytes[pos..]);
                let mut at = pos + used;
                for _ in 0..count {
                    let (name, used) = read_name(&bytes[at..]);
                    at += used;
                    at += 1;
                    let (_, used) = read_uleb(&bytes[at..]);
                    at += used;
                    shape.exports.push(name);
                }
            }
            _ => {}
        }
        pos = end;
    }
    shape.imports.sort();
    shape.exports.sort();
    shape
}

fn read_uleb(bytes: &[u8]) -> (u32, usize) {
    let mut value = 0u32;
    let mut shift = 0;
    let mut used = 0;
    loop {
        let byte = bytes[used];
        used += 1;
        value |= ((byte & 0x7f) as u32) << shift;
        if byte & 0x80 == 0 {
            return (value, used);
        }
        shift += 7;
    }
}

fn read_name(bytes: &[u8]) -> (String, usize) {
    let (len, used) = read_uleb(bytes);
    let text = String::from_utf8_lossy(&bytes[used..used + len as usize]).into_owned();
    (text, used + len as usize)
}
