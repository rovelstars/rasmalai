#!/usr/bin/env bash
set -euo pipefail

stage="${1:?usage: farm-push-ttl.sh <stage-dir> <index-dir> <repo> <ttl-tag>}"
index_dir="${2:?usage: farm-push-ttl.sh <stage-dir> <index-dir> <repo> <ttl-tag>}"
repo="${3:?usage: farm-push-ttl.sh <stage-dir> <index-dir> <repo> <ttl-tag>}"
tag="${4:?usage: farm-push-ttl.sh <stage-dir> <index-dir> <repo> <ttl-tag>}"
base="https://ttl.sh"

fail() {
    echo "farm-push-ttl: $1" >&2
    exit 1
}

push_blob() {
    local file="$1"
    local hex="$2"
    local digest="sha256:$hex"
    local headers code location resume
    headers="$(mktemp)"
    code="$(curl -sS -D "$headers" -o /dev/null -w '%{http_code}' -X POST --data-binary "@$file" -H 'Content-Type: application/octet-stream' "$base/v2/$repo/blobs/uploads/?digest=$digest")"
    case "$code" in
        201)
            rm -f "$headers"
            return 0
            ;;
        202)
            ;;
        *)
            rm -f "$headers"
            fail "blob $digest upload answered HTTP $code"
            ;;
    esac
    location="$(grep -i '^location:' "$headers" | tr -d '\r' | sed 's/^[Ll]ocation: *//' || true)"
    rm -f "$headers"
    [ -n "$location" ] || fail "blob $digest upload gave 202 with no location"
    case "$location" in
        *digest=*) resume="$location" ;;
        *) resume="$location&digest=$digest" ;;
    esac
    case "$resume" in
        http*) ;;
        *) resume="$base$resume" ;;
    esac
    code="$(curl -sS -o /dev/null -w '%{http_code}' -X PUT --data-binary "@$file" -H 'Content-Type: application/octet-stream' "$resume")"
    case "$code" in
        2*) return 0 ;;
        *) fail "blob $digest resume answered HTTP $code" ;;
    esac
}

put_manifest() {
    local file="$1"
    local reference="$2"
    local media="$3"
    local code
    code="$(curl -sS -o /dev/null -w '%{http_code}' -X PUT --data-binary "@$file" -H "Content-Type: $media" "$base/v2/$repo/manifests/$reference")"
    case "$code" in
        2*) return 0 ;;
        *) fail "manifest $reference push answered HTTP $code" ;;
    esac
}

digest="$(cat "$index_dir/digest.txt")"
case "$digest" in
    sha256:*) ;;
    *) fail "bad digest $digest" ;;
esac

for meta in "$stage"/*.meta.json; do
    rel="$(jq -r '.blob' "$meta")"
    [ -n "$rel" ] && [ "$rel" != "null" ] || fail "$meta carries no blob"
    file="$stage/$rel"
    [ -f "$file" ] || fail "missing blob $rel"
    push_blob "$file" "$(sha256sum "$file" | cut -d' ' -f1)"
done
for blob in "$index_dir"/blobs/*; do
    [ -f "$blob" ] || continue
    push_blob "$blob" "$(basename "$blob")"
done

for manifest in "$index_dir"/manifests/*.json; do
    hex="$(basename "$manifest" .json)"
    put_manifest "$manifest" "sha256:$hex" "application/vnd.oci.image.manifest.v1+json"
done
put_manifest "$index_dir/index.json" "$tag" "application/vnd.oci.image.index.v1+json"

tmp="$(mktemp)"
trap 'rm -f "$tmp"' EXIT
curl -sS -H 'Accept: application/vnd.oci.image.index.v1+json' "$base/v2/$repo/manifests/$digest" -o "$tmp"
[ "$(sha256sum "$tmp" | cut -d' ' -f1)" = "${digest#sha256:}" ] || fail "anonymous read-back digest mismatch"
echo "farm-push-ttl: pushed $repo:$tag index $digest, anonymous read-back matches"
echo "farm-push-ttl: note: ttl.sh tags expire $tag after push (canary-grade origin; the uploaded farm-index artifact remains the evidence)"
