#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
repo=$(cd "$here/../.." && pwd)
prefix="$repo/.rnx-cache/wrappers"
rnx=${RNX_BIN:-$repo/target/debug/rnx}

if [ ! -f "$prefix/host_cabi.pc" ]; then
    sh "$repo/compiler/stdlib/src/app/tools/build-wrappers.sh"
fi

if [ -z "${RNX_VENDOR_DIR:-}" ]; then
    stage=${RNX_STAGE_DIR:-$repo/.rnx-cache/std-vendor}
    sh "$repo/compiler/stdlib/src/app/tools/stage-stdlib.sh" "$stage"
    export RNX_VENDOR_DIR="$stage"
fi
export PKG_CONFIG_PATH="$prefix${PKG_CONFIG_PATH:+:$PKG_CONFIG_PATH}"
export LD_LIBRARY_PATH="$prefix${LD_LIBRARY_PATH:+:$LD_LIBRARY_PATH}"
exec "$rnx" run --backend "${RNX_BACKEND:-cranelift}" "$@"
