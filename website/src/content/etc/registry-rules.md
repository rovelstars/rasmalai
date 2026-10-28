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

With that manifest the upload holds `Project.config`, the two entry files, the rest of `src/**/*.rnx`, every guide under `docs/`, plus the readme and license. A file the manifest does not cover and the rules below do not include stays home.

## Custom guides ride the docs folder

Every `*.md` directly under `entries.docs`, plus one nesting level down, becomes one guide in the publish frame, sorted by slug. The frame carries raw markdown source only — slug, title, source, description, section, order. The client never sends rendered html; the server renders the markdown and sanitizes it on accept, and the package page injects that stored html. The `/{version}/guides` routes serve the index, one guide as html, or sanitized source with `?format=source`.

- Slug: the filename without `.md`, lowercased, with each run of other characters folded to one dash and leading and trailing dashes dropped. It must match `^[a-z0-9][a-z0-9-]{0,63}$`. A file that maps to nothing usable, two files that map to the same slug, or a file nested deeper than one level fails the publish with `E108` naming the file.
- Title: the first `# ` heading, else the frontmatter `title:`, else the filename with dashes and underscores turned into spaced, capitalized words.
- Optional frontmatter uses the same `---` block shape as site content, and only `title:`, `description:`, `section:`, and `order:` are read. Unknown keys are ignored, and a leading `---` line always opens frontmatter, so start the file with a heading when the guide itself opens with a rule. Blank section and order values are stored as null.

## What never ships

`target/`, `.git/`, `.rnx/`, `.rnx-cache/`, and `tests/` are excluded even when they hold matching files. Directories declared as bindgen `outputs` are excluded the same way — generated bindings travel as OCI artifacts, and a package entry that points inside one is rejected at pack time. `Project.deplock` never ships either — the lockfile pins your build and stays on your machine; publishing it would pin everyone else's. There are no wasm blobs, no compiled binaries, and no generated output in uploads: if a file is not source, manifest, readme, guide, or license text, it does not belong in the upload.

## Native binaries and wasm live outside source packages

Compiled artifacts are never uploaded with source — neither per-platform native libraries nor `.wasm` modules. They ship as OCI artifacts: the registry stores no artifact bytes, only a pin. One image per package version; composition is the image author's job (an image bundles everything its version needs). A package that needs artifacts declares one bare repo ref, no tags, no digests:

```rnx
export default {
    project: { name: "codec", version: "0.3.0" },
    entries: { lib: "src/lib.rnx" },
    artifacts: "ghcr.io/org/codec"
}
```

The index is the file list — config never duplicates it, so a digest-bearing or keyed `artifacts` table is rejected. At publish time the author asserts the index digest out-of-band (`rnx publish --artifact sha256:<hex>`, one digest for the one image). The server resolves the declared ref fresh, fetches the index (capped at 1 MiB), hashes it, and rejects the publish fail-closed on mismatch; declaration and assertion must agree in both directions. Every fetch below the index addresses content by digest: consumers verify the index digest first, then per-blob hashes from the verified index, pick role (`runtime`, `bindgen`, `wasm`, `media`) and platform from the descriptors, and resolve each native entry's `from` title per role into the content-addressed caches (project `.rnx-cache/artifacts/`, durable global `~/.cache/rnx/artifacts/<hash>`, hardlink with copy fallback); a dev checkout shadows the cached entry with a notice. A future operator mirror would serve identical hashes under a reserved prefix outside the chunk store; until then there is no registry blob storage to meter or delete.

Tags discover, digests pin: refs are bare repos (an explicit tag is a discovery hint only), and the digest is resolved once and stored as version metadata. WebAssembly follows this same path with one manifest per flavor (no arch suffix; simd/threads/memory64 ship as separate manifests in the same index when needed). First fetch of a native artifact prints what is being installed from where. Native code is unaudited foreign bytes: declaring `artifacts` records the dependency and its pin, but no analysis passes over the blobs — see the capability rules in the language manual. WebAssembly declarations and verification work today; execution needs a wasm runtime, which is a separate, not-yet-built phase.

## Farm-built images cover two Linux targets

The `Farm` workflow builds one multi-target image per change to the native, bindgen, or OCI paths: `x86_64-unknown-linux-gnu` (emitted, statically linked, and executed — the binary must exit 42) plus `aarch64-unknown-linux-gnu` (object emission and ELF machine check only, since the linker backend targets x86_64 Linux). Each leg stages its blob with its target, role, title, generator, upstream source hash, and platform; the assemble job validates every descriptor against the annotation grammar, writes one sorted index, and verifies it pointer-style (index digest, then manifest hashes, then blob hashes). The index and blobs upload as workflow artifacts. A canary copy also pushes to `ttl.sh` under a `24h` tag and reads back anonymously by digest — `ttl.sh` origin data rots on expiry, so the artifact row is the durable evidence, and a durable operator origin is still an open follow-up. macOS and Windows legs wait on linker backend support; adding runners without a backend that can link for them would prove nothing.

## LICENSE is required

Every version must include `LICENSE` or `LICENSE.txt` at the package root. An upload without one is rejected, because the registry redistributes your code and needs the grant on record.

Rejections fail the publish with `E108` and a message naming the offending file; the CLI surfaces them verbatim.

## Stored packages never pin the registry

A `Project.config` that carries top-level `registry` or `registries` keys fails the publish with `E108` naming the keys. The keys would otherwise be stored verbatim and served at install, and registry choice stays with the installer, never with stored bytes. A rejection also means a hand-built frame. Fix: delete both keys from `Project.config` and republish the next version (versions are immutable, so the fix ships as the next number).

The publish gate and the admin scan share one predicate over the whole-tree analysis verdict, so the scan count reads zero by construction: anything the scan would count, the gate rejects first.

Stored violators — packages published before the gate — get `critical` advisories naming the cause and the remedy; `rnx audit` fails on them. An advisory informs, it never yanks: removal still runs the normal yank ladder.

## Runtime imports are rejected, not counted

A non-`@std` package that names a runtime internal is refused outright, before anything is sent. Naming one bare (`__rnx_foo()` with no declaration) is a scope error (`E303`) pointing at the declared form; declaring one with `import { fn name(params): Ret } from runtime "mod"` outside registry-verified `@std/*` is an `E108` naming the owning `@std/*` wrapper. The `rnx publish` pre-check applies the same rule to every file in the upload, so a hand-built frame around the CLI cannot smuggle runtime access in. Only `@std/*`-named frames are exempt, and only for bytes that arrived through registry-verified `@std` provision; a `@std/`-prefixed file smuggled into any other frame is still rejected. Declared `from native` aliases are unaffected, and `__ffi_*` names never counted. Fix: call the owning `@std/*` wrapper instead of the `__rnx_*` internal, and republish — versions are immutable, so the fix ships as the next number.

The pre-check runs each file on its own, so it reports only what one file can prove: parse errors, bare `__rnx_*` uses, and bad `from runtime` declarations. Anything that needs the merged picture — protocol conformance, methods extended onto builtins from another file, unresolved imports — is the server's whole-tree analysis to judge, with the dependency closure the client never holds. The two predicates match by construction: the client never rejects what the server would accept, and whatever the client lets through still faces the server, which fails closed.

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

