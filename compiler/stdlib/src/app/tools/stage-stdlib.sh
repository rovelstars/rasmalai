#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")" && pwd)
app=$(dirname "$here")
repo=$(cd "$app/../../../.." && pwd)
if [ -x "$repo/target/release/rnx" ]; then
    rnx=$repo/target/release/rnx
elif [ -x "$repo/target/debug/rnx" ]; then
    rnx=$repo/target/debug/rnx
else
    echo "no rnx build found; run: cargo build --release -p cli" >&2
    exit 1
fi
stage=${1:-${RNX_STAGE_DIR:-$repo/.rnx-cache/std-vendor}}

modules=""
for package in "$repo"/compiler/stdlib/src/*; do
    [ -f "$package/Project.config" ] || continue
    modules="$modules @std/$(basename "$package")"
done

rm -rf "$stage"
mkdir -p "$stage"
RNX_VENDOR_DIR="$repo/compiler/stdlib/src" "$rnx" vendor $modules --dir "$stage" >/dev/null
