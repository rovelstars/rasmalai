#!/usr/bin/env bash
set -euo pipefail

index_dir="${1:?usage: farm-verify.sh <index-dir> <stage-dir>}"
stage="${2:?usage: farm-verify.sh <index-dir> <stage-dir>}"
python3 - "$index_dir" "$stage" <<'EOF'
import hashlib
import json
import os
import sys

index_dir, stage = sys.argv[1], sys.argv[2]

ASCII = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")


def fail(msg):
    sys.stderr.write("farm-verify: %s\n" % msg)
    sys.exit(1)


def is_digest(value):
    return (len(value) == 71 and value.startswith("sha256:")
            and all(c in "0123456789abcdef" for c in value[7:]))


def valid_name(value):
    return len(value) > 0 and all(c in ASCII or c in "._-" for c in value)


def valid_target(value):
    parts = value.split("-")
    return len(parts) in (3, 4) and all(
        len(p) > 0 and all(c in ASCII or c == "_" for c in p) for p in parts)


def canonical_arch(arch):
    return {"x86_64": "amd64", "aarch64": "arm64"}.get(arch, arch)


with open(os.path.join(index_dir, "index.json"), "rb") as fh:
    index_bytes = fh.read()
with open(os.path.join(index_dir, "digest.txt")) as fh:
    asserted = fh.read().strip()
served = "sha256:" + hashlib.sha256(index_bytes).hexdigest()
if served != asserted:
    fail("digest mismatch (asserted %s, index hashes to %s)" % (asserted, served))
if not is_digest(asserted):
    fail("bad digest spelling %s" % asserted)
index = json.loads(index_bytes)
manifests = index.get("manifests")
if not isinstance(manifests, list) or not manifests:
    fail("index carries no manifests")
if index.get("schemaVersion") != 2:
    fail("index schemaVersion is not 2")

staged = {}
for root, _, files in os.walk(stage):
    for name in files:
        if name.endswith(".meta.json"):
            continue
        path = os.path.join(root, name)
        with open(path, "rb") as fh:
            staged["sha256:" + hashlib.sha256(fh.read()).hexdigest()] = path

targets = []
for item in manifests:
    digest = item.get("digest", "")
    if not is_digest(digest):
        fail("bad descriptor digest %s" % digest[:24])
    ann = item.get("annotations")
    if not isinstance(ann, dict):
        fail("descriptor %s carries no annotations" % digest)
    for key in ("run.rasmalai.kind", "run.rasmalai.role",
                "run.rasmalai.target", "run.rasmalai.generator",
                "run.rasmalai.upstream", "org.opencontainers.image.title"):
        if not ann.get(key):
            fail("descriptor %s misses %s" % (digest, key))
    target = ann["run.rasmalai.target"]
    if not valid_target(target):
        fail("descriptor %s has bad target %s" % (digest, target))
    name, sep, udigest = ann["run.rasmalai.upstream"].partition("@")
    if not sep or not valid_name(name) or not is_digest(udigest):
        fail("descriptor %s has bad upstream" % digest)
    plat = item.get("platform")
    if (not isinstance(plat, dict)
            or not plat.get("os") or not plat.get("architecture")):
        fail("descriptor %s has a bad platform" % digest)
    targets.append((target, plat["os"], plat["architecture"]))
    man_path = os.path.join(index_dir, "manifests", digest[7:] + ".json")
    if not os.path.isfile(man_path):
        fail("manifest file missing for %s" % digest)
    with open(man_path, "rb") as fh:
        man_bytes = fh.read()
    if "sha256:" + hashlib.sha256(man_bytes).hexdigest() != digest:
        fail("manifest bytes do not match %s" % digest)
    man = json.loads(man_bytes)
    cfg = man.get("config", {})
    cdigest = cfg.get("digest", "")
    if not is_digest(cdigest):
        fail("manifest %s carries a bad config digest" % digest)
    cfg_path = os.path.join(index_dir, "blobs", cdigest[7:])
    if not os.path.isfile(cfg_path):
        fail("config blob missing for %s" % digest)
    with open(cfg_path, "rb") as fh:
        cbytes = fh.read()
    if "sha256:" + hashlib.sha256(cbytes).hexdigest() != cdigest:
        fail("config bytes do not match %s" % cdigest)
    layers = man.get("layers")
    if not isinstance(layers, list) or not layers:
        fail("manifest %s carries no layers" % digest)
    for layer in layers:
        ldigest = layer.get("digest", "")
        if not is_digest(ldigest):
            fail("manifest %s carries a bad layer digest" % digest)
        if ldigest not in staged:
            fail("layer %s of %s matches no staged blob" % (ldigest, digest))

for host_arch, want_arch in (("x86_64", "amd64"), ("aarch64", "arm64")):
    matched = [t for (t, o, a) in targets
               if o == "linux" and canonical_arch(host_arch) == a
               and canonical_arch(t.split("-")[0]) == want_arch]
    if len(matched) != 1:
        fail("host linux/%s matches %d descriptors, want 1" % (host_arch, len(matched)))
print("farm-verify: index %s ok (%d descriptors, linux/amd64 + linux/arm64 select)"
      % (asserted, len(manifests)))
EOF
