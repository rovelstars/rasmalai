---
title: "Registry Upload Rules"
description: "What an upload may contain, the plan-shaped caps, and what the server rejects."
section: "Registry"
order: "1"
icon: "Package"
---

# Registry Upload Rules

An upload is the source tree plus readme, docs, and license — nothing else. The client sends exactly that set as a JSON frame, and the server re-checks it on accept. Anything outside the set fails the publish.

## The allowlist

`rnx publish` ships these files and no others:

- `Project.config` itself.
- Every path named in `entries`: `entries.main`, `entries.lib`, and each `entries.bins` target.
- Every `**/*.rnx` under `src/`.
- Every `**/*.md` under the folder named by `entries.docs`.
- `README` or `README.md`, and `LICENSE` or `LICENSE.txt`, at the package root.

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    entries: {
        main: "src/main.rnx",
        lib: "src/lib.rnx",
        docs: "docs"
    }
}
```

With that manifest the upload holds `Project.config`, the two entry files, the rest of `src/**/*.rnx`, every guide under `docs/`, plus the readme and license. A file the manifest does not cover and the rules above do not include stays home.

## What never ships

`target/`, `.git/`, `.rnx/`, `.rnx-cache/`, and `tests/` are excluded even when they hold matching files. `Project.deplock` never ships either — the lockfile pins your build and stays on your machine; publishing it would pin everyone else's. There are no wasm blobs, no compiled binaries, and no generated output in uploads: if a file is not source, manifest, readme, guide, or license text, it does not belong in the upload.

## Native binaries and wasm live outside source packages

Compiled artifacts are never uploaded with source — neither per-platform native libraries nor `.wasm` modules. They ship as OCI artifacts: one index per version, per-target manifests beneath it, identical shape for bindings and binaries. A package that needs them declares the reference and the pinned digest:

```rnx
export default {
    project: { name: "codec", version: "0.3.0" },
    entries: { lib: "src/lib.rnx" },
    artifacts: {
        native: { ref: "ghcr.io/org/codec:0.3.0", digest: "sha256:<64 hex chars>" },
        wasm:   { ref: "ghcr.io/org/codec-wasm:0.3.0", digest: "sha256:<64 hex chars>" }
    }
}
```

Tags discover, digests pin: the digest is resolved once and stored, and every fetch below it addresses content by digest only — a moved tag can never affect a pinned consumer. The package manager downloads, hash-verifies, and caches artifacts globally (`~/.cache/rnx/artifacts/<hash>`, shared across projects). First fetch of a native artifact prints what is being installed from where. Linking native code voids all other sandbox guarantees for that scope — see the capability rules in the language manual. WebAssembly follows this same path with a single entry, since one module runs everywhere; execution needs a wasm runtime, which is a separate, not-yet-built phase — declarations and verification work today, running comes later. Our own blob tier speaks the same OCI protocol, so origins are interchangeable without client changes.

## LICENSE is required

Every version must include `LICENSE` or `LICENSE.txt` at the package root. An upload without one is rejected, because the registry redistributes your code and needs the grant on record.

Rejections fail the publish with `E108` and a message naming the offending file; the CLI surfaces them verbatim.

## Entries must match both ways

The entries manifest fails closed in both directions:

- Every path named in `entries.*` must exist in the upload. The client checks this before sending anything — a missing `entries.main` stops the publish with `E108`.
- The frame's file list must match the packaged tree exactly. A file that is listed but missing, or shipped but unlisted, fails the publish. The server repeats this check on accept, so a hand-built upload cannot smuggle past the client.

## Entry names are paths

Every file name in the upload becomes a path the client writes to disk, so the server checks each name on accept and refuses the whole publish when a name could escape the destination directory. A name is accepted when, after dropping at most one trailing `/`, it is not empty, holds no `\`, does not start with `/`, carries no drive prefix (`C:`), has no empty, `.` or `..` path component, holds no control character, and is at most 255 bytes long.

A publish that carries such a name is rejected with 400 and a message naming the entry. The client applies the same rule to the same names, so a manifest written elsewhere is refused at install time too.

## The caps

Source bytes are capped per plan: 16 KiB on Free, 4 MiB on Paid. The stored snapshot (readme, docs, guides) caps at 512 KiB on every tier. Oversize uploads are rejected — split the package or drop the weight before retrying. Retries are safe: repeat a publish with the same `requestId` and the server answers success with `duplicate: true` instead of writing a second row.

## What the server refuses before it reads

The upload body is checked before it is read. The server looks at the declared length first, and a body that claims more than the cap is refused with 413 without a byte of it reaching the worker; a body that arrives without a declared length is cut off at the same ceiling while it streams. A hostile upload therefore costs the server a header check instead of filling its memory.

The request body is capped at source bytes plus manifest headroom plus 2 MiB. The stored form is canonical: `.rnx` files are minified on the server, so installs carry minified bytes, and `Project.config` is stored with its version redacted to `0.0.0-published` (rehydrated to the resolved version at install).

## The chunk limit

A package is stored as chunks: one chunk per file, and more for a file over 128 KiB, which is split further. The server counts the chunks before it hashes anything and refuses a package that would need more than the plan's new-chunk limit (32 Free, 512 Paid), with 413 and a message naming the count. No version holds more than 4096 chunks.

The number is not arbitrary. Every chunk is one stored object and one call to the object store, and the platform allows 50 such calls per request on the free plan. A package that needs more chunks than that cannot be stored on the free plan at all, so the server says so instead of failing halfway through the upload. A package that stays under the limit is stored in one batch, whatever its size.

