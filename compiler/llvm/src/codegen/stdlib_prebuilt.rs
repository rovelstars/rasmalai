use super::lower_fn::{build_module, check_supported};
use super::{finish_object, OptLevel};
use diagnostics::{Code, Diagnostic};
use inkwell::context::Context;
use std::path::{Path, PathBuf};

const STDLIB_ENTRY: &str = concat!(
    "import { Vec4f } from \"@std/simd\";\n",
    "import { Map } from \"@std/collections\";\n",
    "import { Math } from \"@std/math\";\n",
    "import { File } from \"@std/fs\";\n",
    "import { ByteBuffer } from \"@std/bytes\";\n",
    "import { Clock } from \"@std/time\";\n",
    "import { Rng } from \"@std/random\";\n",
    "import { Mutex } from \"@std/sync\";\n",
    "import { Env } from \"@std/env\";\n",
    "import { Process } from \"@std/process\";\n",
    "import { OS } from \"@std/os\";\n",
    "import { blackBox } from \"@std/testing\";\n",
    "import { HttpStatus } from \"@std/web\";\n",
    "import { TcpStream } from \"@std/net\";\n",
    "import { encodeRequestLine } from \"@std/net/http\";\n",
    "import { JSON } from \"@std/json\";\n",
    "import { stdin } from \"@std/io\";\n",
    "fn Main(): Int { return 0; }\n",
);

fn first_diag(diags: Vec<Diagnostic>, fallback: &str) -> Diagnostic {
    diags
        .into_iter()
        .next()
        .unwrap_or_else(|| Diagnostic::new(Code::E108, fallback.to_string()))
}

fn on_path(name: &str) -> bool {
    std::env::var_os("PATH").map_or(false, |paths| {
        std::env::split_paths(&paths).any(|dir| dir.join(name).is_file())
    })
}

fn link_shared(obj: &Path, out: &Path) -> Result<(), Diagnostic> {
    let driver = ["cc", "gcc", "clang"]
        .into_iter()
        .find(|name| on_path(name))
        .ok_or_else(|| {
            Diagnostic::new(
                Code::E108,
                "prebuilt stdlib: no C driver found (cc, gcc, or clang)".to_string(),
            )
        })?;
    let status = std::process::Command::new(driver)
        .arg("-shared")
        .arg("-o")
        .arg(out)
        .arg(obj)
        .status()
        .map_err(|e| Diagnostic::new(Code::E108, format!("prebuilt stdlib: spawn cc: {e}")))?;
    if !status.success() {
        return Err(Diagnostic::new(
            Code::E108,
            "prebuilt stdlib: cc -shared failed".to_string(),
        ));
    }
    Ok(())
}

fn build_stdlib_object() -> Result<Vec<u8>, Diagnostic> {
    let mut module = frontend::modules::ModuleGraph::from_source(STDLIB_ENTRY)?;
    let desugared = frontend::desugar::desugar(&mut module);
    if !desugared.is_empty() {
        return Err(first_diag(desugared, "prebuilt stdlib: desugar failed"));
    }
    let mut errors = Vec::new();
    for diag in frontend::semantic::check(&module) {
        if diag.code.is_warning() {
            continue;
        }
        errors.push(diag);
    }
    if !errors.is_empty() {
        return Err(first_diag(errors, "prebuilt stdlib: typecheck failed"));
    }
    let mut lowered = lir::lower::lower(&module)?;
    for func in &mut lowered.functions {
        func.is_pub = true;
    }
    let verrs = runtime::lir_pipeline::optimize_for_release(&mut lowered, "Main");
    if !verrs.is_empty() {
        return Err(first_diag(verrs, "prebuilt stdlib: release opt broke verification"));
    }
    for func in &lowered.functions {
        check_supported(&lowered, func)?;
    }
    let context = Context::create();
    let (module, _) = build_module(&context, &lowered, "rnx_stdlib", true, None)?;
    finish_object(&module, "rnx_stdlib", OptLevel::Release, None)
}

pub fn stdlib_entry_source() -> &'static str {
    STDLIB_ENTRY
}

pub fn ensure_prebuilt_stdlib() -> Result<PathBuf, Diagnostic> {
    ensure_prebuilt_stdlib_in(&frontend::cache::global_cache_dir())
}

pub fn ensure_prebuilt_stdlib_in(cache_root: &Path) -> Result<PathBuf, Diagnostic> {
    if !cfg!(target_os = "linux") {
        return Err(Diagnostic::new(
            Code::E108,
            "prebuilt stdlib is Linux-only; JIT falls back to inline stdlib".to_string(),
        ));
    }
    if let Some(hit) = runtime::stdlib_cache::cached_stdlib_in(cache_root) {
        return Ok(hit);
    }
    let object = build_stdlib_object()?;
    let dir = runtime::stdlib_cache::artifact_dir_in(cache_root);
    std::fs::create_dir_all(&dir)
        .map_err(|e| Diagnostic::new(Code::E108, format!("prebuilt stdlib: cache dir: {e}")))?;
    let stamp = std::process::id();
    let obj_path = dir.join(format!("stdlib-{stamp}.o"));
    let so_tmp = dir.join(format!("librnx_stdlib-{stamp}.so.tmp"));
    std::fs::write(&obj_path, &object)
        .map_err(|e| Diagnostic::new(Code::E108, format!("prebuilt stdlib: write object: {e}")))?;
    let linked = link_shared(&obj_path, &so_tmp);
    let _ = std::fs::remove_file(&obj_path);
    linked?;
    let final_path = dir.join(runtime::stdlib_cache::LIB_FILENAME);
    std::fs::rename(&so_tmp, &final_path)
        .map_err(|e| Diagnostic::new(Code::E108, format!("prebuilt stdlib: publish: {e}")))?;
    runtime::stdlib_cache::mark_cached_in(cache_root)
        .map_err(|e| Diagnostic::new(Code::E108, format!("prebuilt stdlib: fingerprint: {e}")))?;
    Ok(final_path)
}
