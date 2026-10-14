---
title: "Registry Upload Rules"
description: "What an upload may contain, the 512KB cap, and what the server rejects."
section: "Registry"
order: "1"
icon: "Package"
---

# Registry Upload Rules

An upload is source, manifest, readme, and license — nothing else. The client packs exactly that set, and the server re-checks it on accept. Anything outside the set fails the publish.

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

`target/`, `.git/`, `.rnx/`, `.rnx-cache/`, and `tests/` are excluded even when they hold matching files. `Project.deplock` never ships either — the lockfile pins your build and stays on your machine; publishing it would pin everyone else's. There are no wasm blobs, no compiled binaries, and no generated output in uploads: if a file is not source, manifest, readme, guide, or license text, it does not belong in the archive.

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

TODO(owner): pin down the exact rejection status code for allowlist and LICENSE failures so clients can match on it.

## Entries must match both ways

The entries manifest fails closed in both directions:

- Every path named in `entries.*` must exist in the archive. The client checks this before sending anything — a missing `entries.main` stops the publish with `E108`.
- The file list in the `X-RNX-Meta` header must match the archive exactly. A file that is listed but missing, or shipped but unlisted, fails the publish. The server repeats this check on accept, so a hand-built upload cannot smuggle past the client.

## The 512KB cap

Each version caps at 512KB of stored snapshot bytes. Oversize uploads are rejected no matter the tier — split the package or drop the weight before retrying. Retries are safe: repeat a publish with the same `X-RNX-Request-ID` and the server answers success with `duplicate: true` instead of writing a second row.
