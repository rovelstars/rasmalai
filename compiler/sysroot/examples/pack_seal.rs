//! Seal a hand-written `sysroot.json` v2 pack with its integrity digest.
//!
//! ```sh
//! cargo run -p sysroot --example pack_seal -- --dir /tmp/musl-pack
//! rnx sysroot add --local /tmp/musl-pack x86_64-unknown-linux-musl
//! ```
//!
//! CI uses this for the musl row: the manifest is written from the files
//! the job stages, and the digest is computed by the same code that
//! verifies it on install. It is a dev example, not a shipped command.

use std::path::{Path, PathBuf};

fn usage() -> ! {
    eprintln!("usage: pack_seal --dir <pack-dir>");
    std::process::exit(2);
}

fn fail(message: String) -> ! {
    eprintln!("{message}");
    std::process::exit(1);
}

fn main() {
    let mut args = std::env::args().skip(1);
    let mut dir = None;
    while let Some(arg) = args.next() {
        match arg.as_str() {
            "--dir" => dir = args.next().map(PathBuf::from),
            other => {
                eprintln!("unknown argument `{other}`");
                usage();
            }
        }
    }
    let Some(dir) = dir else { usage() };
    seal(&dir);
}

fn seal(dir: &Path) {
    let manifest_path = dir.join(sysroot::pack::MANIFEST_FILE);
    let body = std::fs::read(&manifest_path)
        .unwrap_or_else(|e| fail(format!("cannot read {}: {e}", manifest_path.display())));
    let mut value: serde_json::Value = serde_json::from_slice(&body)
        .unwrap_or_else(|e| fail(format!("{}: invalid JSON: {e}", manifest_path.display())));
    let placeholder = format!("sha256:{}", "0".repeat(64));
    value["integrity"]["digest"] = serde_json::Value::String(placeholder);
    write(&manifest_path, &value);

    let digest = sysroot::pack::Pack::digest(dir).unwrap_or_else(|e| fail(e.to_string()));
    value["integrity"]["digest"] = serde_json::Value::String(digest.clone());
    write(&manifest_path, &value);

    sysroot::pack::Pack::load(dir)
        .unwrap_or_else(|e| fail(format!("staged pack does not load: {e}")));
    println!("{digest}");
}

fn write(path: &Path, value: &serde_json::Value) {
    let body = serde_json::to_vec_pretty(value)
        .unwrap_or_else(|e| fail(format!("cannot serialize {}: {e}", path.display())));
    std::fs::write(path, body)
        .unwrap_or_else(|e| fail(format!("cannot write {}: {e}", path.display())));
}
