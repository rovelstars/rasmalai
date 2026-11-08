#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
repo=$(cd "$here/../../../.." && pwd)
if [ -x "$repo/target/release/rnx" ]; then
    rnx=$repo/target/release/rnx
elif [ -x "$repo/target/debug/rnx" ]; then
    rnx=$repo/target/debug/rnx
else
    echo "no rnx build found; run: cargo build --release -p cli" >&2
    exit 1
fi

for dir in "$here"/wrappers/*/; do
    for header in "$dir"include/*.h; do
        [ -f "$header" ] || continue
        lib=$(basename "$header" .h)
        out="$here/.rnx-cache/generated/$lib"
        mkdir -p "$out"
        "$rnx" bindgen \
            --header "$header" \
            --native-lib "$lib" \
            --pkg-name "std-app" \
            --out "$out" \
            --case camel
    done
done
