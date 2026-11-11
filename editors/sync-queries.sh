#!/bin/sh
set -eu

root=$(CDPATH= cd -- "$(dirname "$0")/.." && pwd)
cp "$root/editors/tree-sitter-rasmalai/queries/highlights.scm" \
   "$root/editors/zed/languages/rasmalai/highlights.scm"
