//! Stage a ready-to-install `x86_64-unknown-uefi` target pack.
//!
//! ```sh
//! cargo run -p sysroot --example uefi_pack -- \
//!   --runtime compiler/runtime/uefi/target/x86_64-unknown-uefi/release/libruntime_native.a \
//!   --out /tmp/uefi-pack
//! rnx sysroot add --local /tmp/uefi-pack
//! ```
//!
//! The manifest is written with a placeholder digest, then sealed with
//! `sysroot::pack::digest`, so the bytes installed are the bytes verified.

use std::path::{Path, PathBuf};

const TRIPLE: &str = "x86_64-unknown-uefi";

fn argument(args: &[String], name: &str) -> Option<String> {
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == name {
            return it.next().cloned();
        }
        if let Some(value) = arg.strip_prefix(&format!("{name}=")) {
            return Some(value.to_string());
        }
    }
    None
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let runtime = argument(&args, "--runtime").unwrap_or_else(|| {
        eprintln!(
            "usage: uefi_pack --runtime <libruntime_native.a> [--out <dir>]\n\
             build the archive first:\n\
             cargo build --manifest-path compiler/runtime/uefi/Cargo.toml --target {TRIPLE} --release"
        );
        std::process::exit(2);
    });
    let out = argument(&args, "--out").unwrap_or_else(|| "uefi-pack".to_string());
    let out = PathBuf::from(out);
    let runtime = Path::new(&runtime);
    if !runtime.is_file() {
        eprintln!("no runtime archive at {}", runtime.display());
        std::process::exit(1);
    }

    let lib = out.join("lib").join("rustlib").join(TRIPLE).join("lib");
    if let Err(e) = std::fs::create_dir_all(&lib) {
        eprintln!("cannot create {}: {e}", lib.display());
        std::process::exit(1);
    }
    let archived = lib.join("libruntime_native.a");
    if let Err(e) = std::fs::copy(runtime, &archived) {
        eprintln!("cannot copy the runtime archive: {e}");
        std::process::exit(1);
    }
    let sha = match sysroot::pack::file_sha256(&archived) {
        Ok(sha) => sha,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };

    let manifest = |digest: &str| {
        format!(
            "{{\n  \"version\": 2,\n  \"target\": \"{TRIPLE}\",\n  \"arch\": \"x86_64\",\n  \"os\": \"uefi\",\n  \"env\": \"none\",\n  \"object\": \"coff\",\n  \"libc\": \"none\",\n  \"pointer_width\": 64,\n  \"endian\": \"little\",\n  \"tier\": 3,\n  \"capabilities\": [\"alloc\", \"console\", \"clock\", \"firmware\"],\n  \"cpu\": {{\"model\": \"x86-64\", \"features\": \"\"}},\n  \"loader\": null,\n  \"kernel_min\": null,\n  \"glibc_baseline\": null,\n  \"prelude\": \"@uefi/prelude\",\n  \"link\": {{\n    \"search\": [\"lib/rustlib/{TRIPLE}/lib\"],\n    \"crt\": {{}},\n    \"crt_end\": {{}},\n    \"libs\": {{}},\n    \"builtins\": null,\n    \"runtime\": \"libruntime_native.a\"\n  }},\n  \"runtime\": {{\"abi\": 1, \"file\": \"libruntime_native.a\", \"sha256\": \"{sha}\"}},\n  \"integrity\": {{\"algorithm\": \"sha256\", \"digest\": \"{digest}\"}}\n}}\n"
        )
    };

    let manifest_path = out.join(sysroot::pack::MANIFEST_FILE);
    let placeholder = format!("sha256:{}", "0".repeat(64));
    if let Err(e) = std::fs::write(&manifest_path, manifest(&placeholder)) {
        eprintln!("cannot write the manifest: {e}");
        std::process::exit(1);
    }
    let digest = match sysroot::pack::Pack::digest(&out) {
        Ok(digest) => digest,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if let Err(e) = std::fs::write(&manifest_path, manifest(&digest)) {
        eprintln!("cannot seal the manifest: {e}");
        std::process::exit(1);
    }
    if let Err(e) = sysroot::pack::Pack::load(&out) {
        eprintln!("staged pack does not load: {e}");
        std::process::exit(1);
    }
    println!("{} {}", out.display(), digest);
}
