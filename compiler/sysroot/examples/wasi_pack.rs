//! Stage a ready-to-install `wasm32-unknown-wasi` target pack.
//!
//! ```sh
//! cargo build --manifest-path compiler/runtime/wasi/Cargo.toml \
//!   --target wasm32-unknown-unknown --release
//! cargo run -p sysroot --example wasi_pack -- \
//!   --runtime compiler/runtime/wasi/target/wasm32-unknown-unknown/release/libruntime_native.a \
//!   --out /tmp/wasi-pack
//! rnx sysroot add --local /tmp/wasi-pack wasm32-unknown-wasi
//! ```

use std::path::{Path, PathBuf};

const TRIPLE: &str = "wasm32-unknown-wasi";

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
            "usage: wasi_pack --runtime <libruntime_native.a> [--out <dir>]\n\
             build the archive first:\n\
             cargo build --manifest-path compiler/runtime/wasi/Cargo.toml --target wasm32-unknown-unknown --release"
        );
        std::process::exit(2);
    });
    let out = argument(&args, "--out").unwrap_or_else(|| "wasi-pack".to_string());
    let out = PathBuf::from(out);
    let runtime = Path::new(&runtime);
    if !runtime.is_file() {
        eprintln!("no runtime archive at {}", runtime.display());
        std::process::exit(1);
    }

    let target = sysroot::target::Target::parse(TRIPLE).unwrap_or_else(|e| {
        eprintln!("{e}");
        std::process::exit(1);
    });
    let options = sysroot::build::BuildOptions {
        runtime: Some(runtime.to_path_buf()),
        force: true,
        ..sysroot::build::BuildOptions::default()
    };
    match sysroot::build::build(&target, &out, &options) {
        Ok(built) => println!("{} {}", built.dir.display(), built.digest),
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    }
}
