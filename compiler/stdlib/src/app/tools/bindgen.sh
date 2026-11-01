#!/bin/sh
set -eu
here=$(cd "$(dirname "$0")/.." && pwd)
rnx=${RNX_BIN:-rnx}

for dir in "$here"/wrappers/*/; do
    for header in "$dir"include/*.h; do
        [ -f "$header" ] || continue
        lib=$(basename "$header" .h)
        mkdir -p "$dir"generated
        "$rnx" bindgen \
            --header "$header" \
            --native-lib "$lib" \
            --pkg-name "std-app" \
            --out "$dir"generated \
            --case camel
    done
done
