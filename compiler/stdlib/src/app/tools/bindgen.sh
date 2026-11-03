#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
rnx=${RNX_BIN:-rnx}

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
