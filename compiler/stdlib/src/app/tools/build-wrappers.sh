#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
app=$(dirname "$here")
repo=$(cd "$app/../../../.." && pwd)
prefix=${WRAPPER_PREFIX:-$repo/.rnx-cache/wrappers}
target=${CARGO_TARGET_DIR:-$repo/.rnx-cache/wrapper-target}

export CARGO_TARGET_DIR="$target"

mkdir -p "$prefix"

for w in wgpu parley taffy scene vello cpu host; do
    cargo build --release --manifest-path "$app/wrappers/$w/Cargo.toml"
done

install_lib() {
    lib=$1
    cp "$target/release/lib$lib.so" "$prefix/lib$lib.so"
    cat > "$prefix/$lib.pc" <<PC
prefix=$prefix
libdir=\${prefix}
includedir=\${prefix}
Name: $lib
Description: runix app wrapper
Version: 0.1.0
Libs: -L\${libdir} -l$lib
Cflags:
PC
}

for lib in wgpu_cabi parley_cabi taffy_cabi vello_cabi cpu_cabi host_cabi; do
    install_lib "$lib"
done

echo "wrappers installed in $prefix"
