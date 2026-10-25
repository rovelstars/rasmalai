#!/usr/bin/env bash
set -euo pipefail

stage="${1:?usage: farm-assemble.sh <stage-dir> <out-dir>}"
out="${2:?usage: farm-assemble.sh <stage-dir> <out-dir>}"
mkdir -p "$out/manifests"
python3 - "$stage" "$out" <<'EOF'
import hashlib
import json
import os
import sys

stage, out = sys.argv[1], sys.argv[2]

ASCII = set("abcdefghijklmnopqrstuvwxyzABCDEFGHIJKLMNOPQRSTUVWXYZ0123456789")


def fail(msg):
    sys.stderr.write("farm-assemble: %s\n" % msg)
    sys.exit(1)


def is_hex64(value):
    return len(value) == 64 and all(c in "0123456789abcdef" for c in value)


def valid_name(value):
    return len(value) > 0 and all(c in ASCII or c in "._-" for c in value)


def valid_target(value):
    parts = value.split("-")
    return len(parts) in (3, 4) and all(
        len(p) > 0 and all(c in ASCII or c == "_" for c in p) for p in parts)


def valid_generator(value):
    name, sep, version = value.partition(" ")
    if not sep or not valid_name(name):
        return False
    core = version.split("-", 1)[0].split("+", 1)[0]
    nums = core.split(".")
    return len(nums) == 3 and all(n.isdigit() and len(n) > 0 for n in nums)


def valid_upstream(value):
    name, sep, digest = value.partition("@")
    if not sep or not valid_name(name):
        return False
    return digest.startswith("sha256:") and is_hex64(digest[7:])


def valid_title(value):
    return (len(value) > 0 and value not in (".", "..")
            and "/" not in value and "\\" not in value)


def role_allowed(kind, role):
    return ((kind == "binary" and role in ("runtime", "wasm"))
            or (kind == "bindings" and role == "bindgen")
            or (kind == "media" and role == "media"))


metas = sorted(f for f in os.listdir(stage) if f.endswith(".meta.json"))
if not metas:
    fail("no leg metadata in %s" % stage)

config = b"{}"
config_digest = "sha256:" + hashlib.sha256(config).hexdigest()
seen_targets = set()
descriptors = []
manifest_files = {}
for meta_name in metas:
    with open(os.path.join(stage, meta_name), "rb") as fh:
        meta = json.load(fh)
    for key in ("target", "role", "title", "kind", "generator",
                "upstream", "os", "architecture", "blob"):
        if not meta.get(key):
            fail("%s: missing %s" % (meta_name, key))
    target = meta["target"]
    if target in seen_targets:
        fail("duplicate target %s" % target)
    seen_targets.add(target)
    if not valid_target(target):
        fail("%s: bad target %s" % (meta_name, target))
    if not role_allowed(meta["kind"], meta["role"]):
        fail("%s: role %s not admitted by kind %s"
             % (meta_name, meta["role"], meta["kind"]))
    if not valid_generator(meta["generator"]):
        fail("%s: bad generator %s" % (meta_name, meta["generator"]))
    if not valid_upstream(meta["upstream"]):
        fail("%s: bad upstream %s" % (meta_name, meta["upstream"]))
    if not valid_title(meta["title"]):
        fail("%s: bad title %s" % (meta_name, meta["title"]))
    blob_path = os.path.join(stage, meta["blob"])
    if not os.path.isfile(blob_path):
        fail("missing blob %s" % meta["blob"])
    with open(blob_path, "rb") as fh:
        blob_bytes = fh.read()
    blob_digest = "sha256:" + hashlib.sha256(blob_bytes).hexdigest()
    manifest = {"schemaVersion": 2,
                "mediaType": "application/vnd.oci.image.manifest.v1+json",
                "config": {"mediaType": "application/vnd.oci.image.config.v1+json",
                           "digest": config_digest, "size": len(config)},
                "layers": [{"mediaType": "application/vnd.oci.image.layer.v1.tar",
                            "digest": blob_digest, "size": len(blob_bytes)}]}
    manifest_bytes = json.dumps(manifest, sort_keys=True,
                                separators=(",", ": ")).encode()
    manifest_digest = "sha256:" + hashlib.sha256(manifest_bytes).hexdigest()
    manifest_files[manifest_digest[7:]] = manifest_bytes
    annotations = {"org.opencontainers.image.title": meta["title"],
                   "run.rasmalai.generator": meta["generator"],
                   "run.rasmalai.kind": meta["kind"],
                   "run.rasmalai.role": meta["role"],
                   "run.rasmalai.target": target,
                   "run.rasmalai.upstream": meta["upstream"]}
    descriptors.append({"mediaType": "application/vnd.oci.image.manifest.v1+json",
                        "digest": manifest_digest,
                        "size": len(manifest_bytes),
                        "platform": {"os": meta["os"],
                                     "architecture": meta["architecture"]},
                        "annotations": annotations})

descriptors.sort(key=lambda d: d["digest"])
index = {"schemaVersion": 2, "manifests": descriptors}
index_bytes = json.dumps(index, sort_keys=True,
                         separators=(",", ": ")).encode()
digest = "sha256:" + hashlib.sha256(index_bytes).hexdigest()
os.makedirs(os.path.join(out, "blobs"), exist_ok=True)
with open(os.path.join(out, "blobs", config_digest[7:]), "wb") as fh:
    fh.write(config)
with open(os.path.join(out, "index.json"), "wb") as fh:
    fh.write(index_bytes)
for hexpart, body in manifest_files.items():
    with open(os.path.join(out, "manifests", hexpart + ".json"), "wb") as fh:
        fh.write(body)
with open(os.path.join(out, "digest.txt"), "w") as fh:
    fh.write(digest + "\n")
print("farm-assemble: index %s (%d manifests)" % (digest, len(descriptors)))
EOF
