#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
out="$root/target/probe"
rnx=${RNX_BIN:-rnx}

mkdir -p "$out"
cargo build --release --manifest-path "$root/Cargo.toml"
cp "$root/target/release/libwgpu_cabi.so" "$out/libwgpu_cabi.so"

cat > "$out/wgpu_cabi.pc" <<PC
prefix=$out
libdir=\${prefix}
includedir=\${prefix}
Name: wgpu_cabi
Description: gpu pilot
Version: 0.1.0
Libs: -L\${libdir} -lwgpu_cabi
Cflags:
PC

cd "$here"
PKG_CONFIG_PATH="$out" LD_LIBRARY_PATH="$out" exec "$rnx" run "$@"
