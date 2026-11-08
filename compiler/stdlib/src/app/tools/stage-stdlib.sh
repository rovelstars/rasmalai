#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
app=$(dirname "$here")
repo=$(cd "$app/../../../.." && pwd)
if [ -n "${RNX_BIN:-}" ]; then
    rnx=$RNX_BIN
elif [ -x "$repo/target/release/rnx" ]; then
    rnx=$repo/target/release/rnx
else
    rnx=$repo/target/debug/rnx
fi
stage=${1:-${RNX_STAGE_DIR:-$repo/.rnx-cache/std-vendor}}

rm -rf "$stage"
mkdir -p "$stage"

"$rnx" vendor @std/prelude @std/bytes @std/env @std/os @std/fs @std/sync --dir "$stage" >/dev/null

for name in ui app; do
    package="$repo/compiler/stdlib/src/$name"
    mkdir -p "$stage/@std/$name"
    cp "$package/Project.config" "$stage/@std/$name/"
    cp "$package/lib.rnx" "$stage/@std/$name/"
    if [ -d "$package/docs" ]; then
        cp -R "$package/docs" "$stage/@std/$name/"
    fi
    if [ -d "$package/src" ]; then
        cp -R "$package/src" "$stage/@std/$name/"
    fi
done
