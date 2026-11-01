#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
rnx=${RNX_BIN:-rnx}

cargo build --release --manifest-path "$root/Cargo.toml"
cp "$root/target/release/libgpu_sys.so" "$here/libgpu_sys.so"

cat > "$here/gpu_sys.pc" <<PC
prefix=$here
libdir=\${prefix}
includedir=\${prefix}
Name: gpu_sys
Description: gpu pilot
Version: 0.1.0
Libs: -L\${libdir} -lgpu_sys
Cflags:
PC

cd "$here"
PKG_CONFIG_PATH="$here" LD_LIBRARY_PATH="$here" exec "$rnx" run "$@"
