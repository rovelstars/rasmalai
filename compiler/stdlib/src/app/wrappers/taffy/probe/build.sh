#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
root=$(dirname "$here")
out="$root/target/probe"
app=$(cd "$root/../.." && pwd)
repo=$(cd "$app/../../../.." && pwd)
if [ -x "$repo/target/release/rnx" ]; then
    rnx=$repo/target/release/rnx
elif [ -x "$repo/target/debug/rnx" ]; then
    rnx=$repo/target/debug/rnx
else
    echo "no rnx build found; run: cargo build --release -p cli" >&2
    exit 1
fi

mkdir -p "$out"
cargo build --release --manifest-path "$root/Cargo.toml"
cp "$root/target/release/libtaffy_cabi.so" "$out/libtaffy_cabi.so"

cat > "$out/taffy_cabi.pc" <<PC
prefix=$out
libdir=\${prefix}
includedir=\${prefix}
Name: taffy_cabi
Description: taffy pilot
Version: 0.1.0
Libs: -L\${libdir} -ltaffy_cabi
Cflags:
PC

cd "$here"
PKG_CONFIG_PATH="$out" LD_LIBRARY_PATH="$out" exec "$rnx" run "$@"
