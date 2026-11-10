//! Stage a ready-to-install `x86_64-unknown-uefi` target pack.
//!
//! ```sh
//! cargo build --manifest-path compiler/runtime/uefi/Cargo.toml \
//!   --target x86_64-unknown-uefi --release
//! cargo run -p sysroot --example uefi_pack -- \
//!   --runtime compiler/runtime/uefi/target/x86_64-unknown-uefi/release/libruntime_native.a \
//!   --out /tmp/uefi-pack
//! rnx sysroot add --local /tmp/uefi-pack
//! ```

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
