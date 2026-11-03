#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
app=$(dirname "$here")
repo=$(cd "$app/../../../.." && pwd)
rnx=${RNX_BIN:-$repo/target/debug/rnx}
stage=${1:-${RNX_STAGE_DIR:-$repo/.rnx-cache/std-vendor}}

rm -rf "$stage"
mkdir -p "$stage"

"$rnx" vendor @std/prelude @std/bytes @std/env --dir "$stage" >/dev/null

for name in ui app; do
    package="$repo/compiler/stdlib/src/$name"
    mkdir -p "$stage/@std/$name"
    cp "$package/Project.config" "$stage/@std/$name/"
    cp "$package/lib.rnx" "$stage/@std/$name/"
    if [ -d "$package/docs" ]; then
        cp -R "$package/docs" "$stage/@std/$name/"
    fi
done
