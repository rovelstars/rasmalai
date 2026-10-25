#!/usr/bin/env bash
# Print the nextest target list for the crates affected by the current change.
#
# Usage: affected-tests.sh [base-ref]
#
# Outputs one line on stdout:
#   FULL                  nothing mappable, an error, or a non-crate change:
#                         run the whole workspace suite.
#   -p cli -p frontend   only these workspace packages (plus everything that
#                         depends on them) are affected: pass the line straight
#                         to `cargo nextest run --profile ci`.
#
# Fails open by design: every error path prints FULL and exits 0, so CI can
# never silently skip the suite. Unknown file mappings (website/, .github/,
# root manifests, lockfile) also print FULL.
set -u

ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
BASE="${1:-origin/main}"

fail_open() {
    echo "FULL"
    exit 0
}

cd "$ROOT" || fail_open
git rev-parse --verify --quiet "$BASE" >/dev/null 2>&1 || fail_open
MERGE_BASE="$(git merge-base "$BASE" HEAD 2>/dev/null)" || fail_open
[ -n "$MERGE_BASE" ] || fail_open

CHANGED="$(git diff --name-only "$MERGE_BASE" HEAD -- 2>/dev/null)" || fail_open
[ -n "$CHANGED" ] || fail_open

export RAS_AFFECTED_CHANGED="$CHANGED"

python3 - "$ROOT" <<'EOF' || echo "FULL"
import json
import os
import subprocess
import sys

root = sys.argv[1]
changed = os.environ.get("RAS_AFFECTED_CHANGED", "").splitlines()


def run_metadata(no_deps):
    cmd = ["cargo", "metadata", "--format-version", "1", "--manifest-path",
           os.path.join(root, "Cargo.toml")]
    if no_deps:
        cmd.append("--no-deps")
    out = subprocess.run(cmd, capture_output=True, text=True, cwd=root)
    if out.returncode != 0:
        raise RuntimeError("cargo metadata failed: " + out.stderr[-500:])
    return json.loads(out.stdout)


try:
    meta = run_metadata(no_deps=True)
except Exception as e:
    sys.stderr.write("affected-tests: %s\n" % e)
    sys.exit(1)

members = {}  # name -> rel dir
for pkg in meta.get("packages", []):
    manifest = pkg["manifest_path"]
    pkg_dir = os.path.dirname(manifest)
    rel = os.path.relpath(pkg_dir, root)
    members[pkg["name"]] = "" if rel == "." else rel

ROOT_FILES = ("Cargo.toml", "Cargo.lock")

touched = set()
for path in changed:
    path = path.strip()
    if not path:
        continue
    if path in ROOT_FILES or path.startswith(".github/") or path.startswith("website/"):
        sys.exit(2)  # conservative: non-crate change runs everything
    owner = None
    for name, rel in members.items():
        if rel and (path == rel or path.startswith(rel + "/")):
            if owner is None or len(rel) > len(members[owner]):
                owner = name
    if owner is None:
        sys.exit(2)  # unknown mapping runs everything
    touched.add(owner)

if not touched:
    sys.exit(2)

try:
    full = run_metadata(no_deps=False)
except Exception as e:
    sys.stderr.write("affected-tests: %s\n" % e)
    sys.exit(1)

by_id = {}
for pkg in full.get("packages", []):
    by_id[pkg["id"]] = pkg["name"]

resolve = (full.get("resolve") or {}).get("nodes", [])
forward = {}  # member -> workspace members it depends on
for node in resolve:
    name = by_id.get(node["id"])
    if name not in members:
        continue
    deps = set()
    for dep in node.get("deps", []):
        dep_name = by_id.get(dep.get("pkg"))
        if dep_name in members and dep_name != name:
            deps.add(dep_name)
    forward[name] = deps


def reaches(member, target, seen):
    if member in seen:
        return False
    seen.add(member)
    direct = forward.get(member, set())
    if target in direct:
        return True
    return any(reaches(d, target, seen) for d in direct)


affected = set(touched)
for member in members:
    if member in affected:
        continue
    for t in touched:
        if reaches(member, t, set()):
            affected.add(member)
            break

print(" ".join("-p %s" % name for name in sorted(affected)))
EOF
