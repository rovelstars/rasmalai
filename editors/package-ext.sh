#!/bin/sh
set -eu

out=${1:?usage: package-ext.sh <out-dir> <version> <source-commit>}
version=${2:?usage: package-ext.sh <out-dir> <version> <source-commit>}
commit=${3:?usage: package-ext.sh <out-dir> <version> <source-commit>}
root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)

rm -rf "$out"
mkdir -p "$out/grammars/rasmalai"
cp -R "$root/editors/zed/." "$out/"
rm -rf "$out/target" "$out/extension.wasm" "$out/grammars"
cp -R "$root/editors/tree-sitter-rasmalai/." "$out/grammars/rasmalai/"
rm -f "$out/grammars/rasmalai/tree-sitter-rasmalai.wasm"
cp "$root/LICENSE" "$out/LICENSE"

if [ ! -f "$out/grammars/rasmalai/src/parser.c" ]; then
    echo "package-ext: grammars/rasmalai/src/parser.c is missing; run tree-sitter generate first" >&2
    exit 1
fi

cat > "$out/SOURCE" <<EOF
source_repository=https://github.com/rovelstars/rasmalai
source_commit=$commit
source_version=$version
EOF

cat > "$out/README.md" <<EOF
# rasmalai-ext

Generated editor support for Rasmalai. Do not edit this repository by hand.

Source: https://github.com/rovelstars/rasmalai
Version: $version
Commit: $commit

Contents:

- the Zed extension at the repository root
- the tree-sitter grammar under \`grammars/rasmalai\`

Report issues at https://github.com/rovelstars/rasmalai/issues.
EOF

cat > "$out/.gitignore" <<'EOF'
/target
/extension.wasm
/grammars/rasmalai/*.wasm
EOF
