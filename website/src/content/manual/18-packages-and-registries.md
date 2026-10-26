---
title: "Packages and Registries"
description: "Names, versions, yank rules, the registry API, import sources, and the Project.config subset."
section: "Toolchain and Diagnostics"
icon: "Package"
---

# Packages and Registries

A package is a directory with a `Project.config` manifest. The manifest names the package, lists its dependencies, and optionally points at registries. This page covers how names work, what the registry guarantees, the five ways a dependency can name its source, the exact subset of rnx the manifest may use, and a full sqlite3 example.

## Names and scopes

Names come in two shapes: unscoped (`sqlite3`) and scoped (`@acme/ui`). Both parts are lowercase letters, digits, and hyphens, starting with a letter or digit. An unscoped name holds at most 64 characters; a scope holds at most 32 and the name after the slash at most 64. Anything else fails validation — the CLI rejects it as `E108` and the server answers 400 `invalid package name`. The `std` scope is reserved for the standard library.

The first publish of a name claims its owner. A later publish of the same package by anyone else fails with 403 `package <name> is owned by someone else`; for a scoped package the check is scope membership (`scope @<scope> is not yours`). The org token skips the owner check as operator break-glass.

## Versions are immutable

A version looks like `1.2.3` with an optional prerelease tail (`1.2.3-beta.1`). Publishing an already-used `name@version` fails with 409 and the message `version <v> already published (versions are immutable)`. There is no re-mint: not by the owner, not after a yank, not after a takedown, not after a scope transfer. The server blocks reuse against live rows, tombstones, and the audit log together, so even a deleted-then-republished version keeps its number burned.

Retries are safe. Publish takes a client-generated idempotency key — the `requestId` frame field, or the `X-RNX-Request-ID` header when the frame leaves it empty — and repeating a publish with the same key returns success with `duplicate: true` instead of a second row. Yank, takedown, and unpublish take the same key as a JSON body field or that header.

## Yanked versus taken down

Yank is reversible-looking but one-way, and takedown is forever. Both keep the number burned per the rule above.

`POST /api/v0/packages/<name>@<version>/yank` flips a live version to `yanked`. The metadata still serves, now with `"status": "yanked"`. Pinned requirements keep resolving — yanked versions stay in the candidate set — but the latest pointer and the catalog skip them, so new floating ranges move past. Yanking a version that is not live fails with 409 (`version <v> is not live`).

`POST /api/v0/packages/<name>@<version>/takedown` needs the admin token. It writes a tombstone row carrying the reason and every later read of that version answers 410 `{ code: "withdrawn", reason }` with a one-year immutable cache. The number never comes back.

Only the `@std/*` scope has a further carve-out, `DELETE /api/scopes/{scope}/{name}@{version}`, which needs the org admin plus a second approval. It has no CLI path.

## Security advisories

Package and artifact advisories ride a registry-signed feed. Only the operator speaks: advisory objects name package versions and artifact pins, and no code path renders package content as a notice, so a forged notice is structurally impossible. Upstream CVE feeds are an input to advisories, not a fetch path — nothing fetches third-party feeds at build time; the operator files each advisory by hand through the route below.

An advisory names what it covers and how bad it is:

```json
{
  "id": "ADV-2026-0007",
  "package": "@acme/widget",
  "affected": "^1.0.0",
  "artifacts": ["sha256:ab…"],
  "severity": "critical",
  "title": "signature bypass in the handshake",
  "detail": "versions in ^1.0.0 accept …"
}
```

`affected` uses the same range grammar as dependency requirements (`*`, `latest`, `X.Y.Z`, `^`, `~`, `>=`). `artifacts` is optional: when present, the advisory applies only to locked entries whose `artifactDigest` matches one of the pins exactly. Severity maps onto the notice tiers: `critical` is a security hold and fails operations, `warning` warns exactly like a yank, `info` is a plain notice.

`GET /api/v0/advisories` serves the active advisories as one JSON document plus the operator's Ed25519 signature over the canonical byte form of that list:

```json
{ "advisories": [ … ], "keyId": "9f2c…", "publicKey": "ab…", "signature": "cd…" }
```

Clients verify the signature before matching anything, and fail closed when it does not verify. `RNX_ADVISORY_KEY` pins the expected key id or public key hex; a feed signed by any other key is rejected. Withdrawn advisories leave the feed but stay in the audit log, so the feed only ever carries what currently applies.

Filing and withdrawing are operator-only. Both need the admin token and `X-RNX-Protocol: 0`:

- `POST /api/v0/advisories` with the advisory object files it (a duplicate id fails with 409).
- `POST /api/v0/advisories/<id>/withdraw` marks it withdrawn. Withdrawing an unknown or already-withdrawn id answers 404.

`rnx audit` matches the locked `Project.deplock` — versions and `artifactDigest` pins — against this feed and reports. A `critical` finding fails the audit; everything else warns or notes. The re-pin of affected dependencies is a separate, explicit update step; the audit names the finding, it never rewrites the lockfile.

`rnx audit --licenses` reports the license graph instead of the capability surface: one line per locked package with its SPDX id, effective `link` (`static` or `dynamic` from the lockfile), and obligation class (`none`, `notice`, `file-copyleft`, `relink-object-files`, `relink-corresponding-source`, `whole-work-copyleft`), followed by conflict notes (Apache-2.0 against GPL-2.0, static copyleft pulls, static relink obligations). `--deny-license <spdx>` (comma-separated, repeatable) fails the audit when a locked package carries a denied id; unknown ids in the flag fail with `E108`. A lock with conflicts or denied hits exits `1`, so the flag gates CI.

## Reading from the registry

Reads take no credentials. Every response carries an `rnx-registry-spec: 0` header. `GET /api/version` reports the contract in one place:

```json
{ "spec": 0, "capabilities": ["tombstones", "guides", "jsdoc", "yank", "transfer", "advisories"], "registry": "rasmalai.rovelstars.com" }
```

The fixed paths are:

| Request | Result |
|---|---|
| `GET /api/v0/packages` | catalog of package summaries |
| `GET /api/v0/packages/<name>` (or `@<scope>/<name>`) | 302 to the latest versioned URL, never content |
| `GET .../<name>@<version>` | pinned version; older `/<name>/<version>` path form 308-redirects here |
| `GET .../<name>@<version>` | metadata: status, checksums, engine range, api/manifest/chunks/guides links |
| `GET .../<version>/api` | the jsdoc snapshot JSON |
| `GET .../<version>/manifest` | the stored dependency manifest (deps, homepage) |
| `GET .../<version>/chunks` | chunk entry manifest (file names, sizes, content hashes) describing canonical bytes; 404 `no-file-index` when the version ships no file index |
| `GET .../<version>/chunk/<sha256>` | one content chunk by hash, immutable — clients fetch these in parallel and write files straight to disk; the digest is accepted in either case |
| `GET .../<name>@<version>/guides[/<slug>]` | guide index or one guide, 404 when the package ships none |
| `GET /api/v0/advisories` | registry-signed advisory feed: active advisories plus `keyId`, `publicKey`, and `signature` |

Version metadata and content chunks cache as immutable for a year; a missing name or version answers 404 JSON and a tombstoned version answers 410. The status of a version is the one part of that metadata that changes after the publish, so the server purges the version URLs when a yank or a takedown lands, and then reads the edge back: the purge counts as done only when the edge answers with the new state instead of the old copy. Without the purge credentials (`CF_ZONE_ID` and `CF_PURGE_TOKEN`) nothing is purged, and the one-year lifetime of the metadata route is the bound. Tooling that must see one consistent generation (stdlib seeding, stdlib docs, the playground) reads transport payloads under a fixed `?origin-direct=1` namespace instead of the bare URL; the server answers that namespace `no-store`, so it always reflects origin live truth and can never hold a stale generation.

The chunk manifest is served compressed to a client that asks for it, and the route answers with `Vary: accept-encoding` so the cache never hands one encoding to a client that asked for the other. A chunk digest is accepted in either case: an upper case digest is the same digest, not a missing chunk.

The command line client speaks HTTP/2 to the registry. One TLS session per host carries the whole download, so the chunk reads of a cold seed run as parallel streams on that one session instead of opening a session each. A server that only speaks HTTP/1.1 still works: the client falls back to it and keeps the connection alive between reads.

### Entry names in a chunk manifest

Every entry name in a chunk manifest is a path the client writes to disk, so the server checks the names at publish and refuses the publish when a name could escape the destination directory. A name is accepted when, after dropping at most one trailing `/`, all of the following hold:

- it is not empty;
- it holds no `\` (a separator on Windows, where joining it onto the destination directory would leave that directory);
- it does not start with `/`;
- it carries no drive prefix (`C:`);
- every `/`-separated component is non-empty and is neither `.` nor `..`;
- it holds no control character, NUL included;
- it is at most 255 bytes long.

A publish that carries such a name is rejected with 400 and a message naming the entry. The client applies the same rule to the same names, so a manifest that came from somewhere else is refused at install time as well.

A version that ships no file index at all answers 404 `no-file-index` on the chunks route, and the client reports that as a package with no file index, never as a broken download.

### Playground stdlib prefetch

The web playground carries no standard library inside its wasm engine either: engine download pre-resolves the pinned `@std` set at the engine version, preloads `@std/prelude`, then resolves each compilation in two phases — the engine reports the missing modules (`prelude` plus the transitive closure of the user's `@std/*` imports) and the page fetches them from the same site's registry before checking. Chunk bytes verify per-hash in the browser and persist in IndexedDB beside the engine bytes; versioned package URLs are immutable, so entries cache forever and only fall away when the engine version moves on. User code keeps the plain `@std/<module>` form throughout — no URL imports exist, all fetching is toolchain-initiated against the same domain.

## Status code reference

| Status | Meaning | Body |
|---|---|---|
| `200` / `201` | version is live | metadata or stored manifest digest |
| `302` | name without version | redirect to the latest versioned URL, never content |
| `400` | bad input | `invalid package name`, `bad-range` |
| `403` | not the owner | `package <name> is owned by someone else`, `scope @<scope> is not yours` |
| `404` | missing name, version, guide, or file index | JSON error |
| `409` | version reuse or bad yank | `version <v> already published (versions are immutable)`, `version <v> is not live` |
| `410` | tombstoned version | `{ code: "withdrawn", reason }`, cached immutably for a year |
| `413` | the request is larger than the server accepts | `payload-too-large` |
| `503` | the registry changed while the answer was computed | `resolution raced with a mutation, retry` |

Every response carries an `rnx-registry-spec: 0` header. A client meeting a spec it does not support is a hard error naming both numbers, and calls needing a capability the server lacks fail closed naming it.

## Resolving a dependency graph

`POST /api/v0/resolve` takes one request with a `requirements` map and a `have` list of already-fetched versions:

```json
{
  "requirements": { "sqlite3": "^3.45.0", "@acme/ui": "~1.2.0" },
  "have": [{ "full": "sqlite3", "version": "3.45.0", "integrity": "sha256:..." }]
}
```

The server picks the highest satisfying version for each requirement, walks manifests transitively, and returns `resolved` plus depth-leveled `levels`. Each node carries its chunk-manifest `path`, `integrity` digest, engine range, and a `yanked` flag. Clients fetch the manifest, pull content chunks in parallel, write files straight to disk, verify the manifest digest, and rehydrate `Project.config` to the resolved version. Installed `.rnx` bytes are canonical minified form, not author formatting. Entries in `have` whose integrity still matches skip dependency expansion, so repeated resolves only fetch the delta.

A `have` entry never hides a status change. A version that was yanked after the client cached it is still reported with `yanked` set, and a version that was taken down is not served at all, not even when the client's cache still names it.

### What a resolve answer means

A resolve answer is a snapshot of the registry taken while the request runs. The server keeps a short-lived memo of the candidate versions of each package so that a repeat resolve does not read them again, and every memo entry carries the mutation count of the package it was built from. Before the answer is written, that count is checked against the row the server reads for the picked version. If it moved, another publish, takedown or transfer changed the package, and the candidate list is read again instead of being trusted.

The rules that follow from that:

- The pick is the highest version in the range that exists right now. A version another server published a moment ago is picked up by the next resolve.
- A taken down version is never resolved. The answer is the remaining version, or `package <name> does not exist` when no version is left.
- A yanked version may still be resolved, and the node carries `"yanked": true`.
- A package that does not exist, and a range that no version satisfies, are answered from the current state of the registry, never from a cached list.
- A registry that changes while the answer is being computed answers 503 `unavailable` with `resolution raced with a mutation, retry`. The client retries and never receives a wrong answer.

Ranges use a small grammar: `*`, `latest`, an exact `X.Y.Z`, `^X.Y.Z`, `~X.Y.Z`, or `>=X.Y.Z`. Anything else is a 400 `bad-range`. The semantics:

- `*` and `latest` match any stable version, never a prerelease.
- An exact version matches exactly it; a prerelease matches only when the range names a prerelease with the same major, minor, and patch.
- `^1.2.3` means at least `1.2.3` with the same major version. `^0.2.3` pins the minor as well, and `^0.0.3` pins the patch.
- `~1.2.3` means at least `1.2.3` with the same major and minor.
- `>=1.2.3` means that version or newer.

Conflicting ranges for one package fail the whole resolve as `unsatisfiable`; a dependency cycle fails as `cycle`.

## Inline chunk lists

A client that sees the `resolve-inline-chunks` capability in `GET /api/version` can add `"chunks": true` to the resolve request. The server then inlines each node's chunk-entry list into the node and names the mode it chose in a top-level `chunks` field:

```json
{
  "resolved": { "@std/fs": "0.1.0" },
  "base": "",
  "levels": [[{
    "full": "@std/fs",
    "version": "0.1.0",
    "path": "@std/fs@0.1.0/chunks",
    "integrity": "e554…e148",
    "engineRange": "",
    "deps": [],
    "yanked": false,
    "entries": [{ "name": "src/main.rnx", "size": 4096, "dir": false, "chunks": ["14ed…2d2"] }]
  }]],
  "chunks": "inline"
}
```

That removes one `GET .../{version}/chunks` per package from a cold seed. `entries` is the same array the chunks endpoint serves, and the client still verifies chunk hashes and the manifest digest, so an inlined list is used *in addition to* verification, never instead of it.

Three rules govern it:

- **Opt-in, additive.** A client that sends no `chunks` field gets the byte-for-byte response it got before, with no `entries` and no `chunks` field. Nothing breaks if the server never gains the capability.
- **`"chunks": "links"` means the server declined.** No node carries `entries` and clients read the chunks endpoint as usual. One global switch keeps the response self-consistent, so a client never has to mix the two paths inside one resolve. The refusal is named in a top-level `chunksDeclined` field, either `size-cap-exceeded` when the inline byte budget (256 KiB) was exceeded, or `closure-too-large` when the closure holds too many packages to be worth inlining. The budget is measured on the resolve body itself, byte for byte, with the inline lists in it, so the number means what it says: the whole response, not only the file indexes. The server measures the file indexes before reading them, so a refused request never pays for an index it would have to drop.
- **A generation error still retries the manifest.** If an inlined list does not verify, the client falls back to reading the chunk manifest origin-direct, exactly as it would without the capability.

Every resolve error response carries a `requestId` (a UUID v4) alongside `code` and `message`. The server logs the failure under `[resolve <id>]`, so paste the id when reporting a 500 `resolve-failed`.

A resolve failure answers with a status the client can act on: 400 `bad-request` or `bad-range`, 404 `not-found`, 413 `payload-too-large`, 422 `budget-exceeded`, `cycle` or `unsatisfiable`, 503 `unavailable` while the store is briefly down or while a mutation races the answer, and 500 `resolve-failed` for anything else. A client that meets a status it does not know prints that status and says its own version is out of date, instead of failing with a parse error.

### The shape of a resolve request

The resolve endpoint is anonymous, so it caps what a request may cost before it reads it:

| Limit | Value | Refusal |
|---|---|---|
| Request body | 64 KiB | 413 `payload-too-large` |
| Entries in `requirements` | 512 | 413 `payload-too-large` |
| Entries in `have` | 256 | 413 `payload-too-large` |

An oversized body is refused on its declared length, before a byte of it is read, and the stream is cut off at the same ceiling for a body that arrives without one. The counts are checked before the names are parsed, so a request with a hundred thousand requirements never pays for validating them.

A resolve also has a runtime budget: the traversal stops at 64 levels, 512 packages, 512 packages in one level and 4096 candidate versions, and the request may spend 50 calls to the store. A closure that needs more answers 422 `budget-exceeded` with the reason in the message. The last limit is the platform's own: one request may make 50 external calls on the free plan, so the resolver counts them and refuses itself rather than being cut off mid-answer.

## Lockfile and rnx update

`rnx lock` writes `Project.deplock`, pinning every dependency to the exact version the registry just served: the version number plus the manifest digest as `checksum`, and the OCI index digest as `artifactDigest` when the version ships artifacts. A build run with a lockfile never auto-upgrades: pinned versions resolve from cache without network access, and byte mismatches abort the build. The registry never stores deplocks — a published deplock would pin everyone else's tree.

Moving a dependency takes an explicit update:

```sh
rnx update
rnx update @acme/ui sqlite3
rnx update @acme/ui@latest
rnx update sqlite3 --major
```

With no names, every registry range re-resolves against the live registry and the lockfile is rewritten with the new digests. With names, only those dependencies float; every other direct registry dependency stays pinned to its locked version while the named ones re-resolve. Each move prints as `updated <name> <old> -> <new>`; when nothing moves the command prints `Already up to date.` and leaves the lockfile byte-identical.

Three rules govern what an update may pick:

- Selectors never reach the manifest. A `name@range` selector is a one-shot override for that update only — the `Project.config` range is untouched, and the lockfile pins the resolved digest, never the range or tag text. `latest` resolves through the registry's computed latest pointer and pins the resulting digest. Any other tag is rejected: named dist-tags need the server `dist-tags` capability, which the registry does not advertise yet.
- Major versions need opt-in. An update that would cross a major version holds that dependency at its locked version and prints `held <name> at <old> (major move to <new> needs --major)`. Passing `--major` allows the crossing. An explicit `name@range` selector always opts that dependency in.
- The server stays the source of truth. A candidate whose engine range the running toolchain does not satisfy fails the update instead of being pinned, and consumer native choices (the root `native` table, `--prefer-system` / `--prefer-bundled`) are preserved across the rewrite. Every byte still verifies against the manifest digest and chunk hashes at install time.

## Publishing

`rnx publish` ships the current project as a source tree. `rnx pack` stays for local archives but plays no part in publishing: the server derives the stored bytes itself.

```sh
rnx publish --registry https://rasmalai.rovelstars.com --token <token>
```

`--registry` defaults to `https://rasmalai.rovelstars.com` and `--token` falls back to the `RNX_TOKEN` environment variable. Without a token, the command stops with `E501` and the hint `pass --token <token> or set RNX_TOKEN in your environment`. The project must be formatted (`rnx fmt`); unformatted files refuse the publish before anything is sent.

The CLI posts one JSON frame with exactly these headers:

- `Authorization: Bearer <token>`
- `Content-Type: application/json`
- `X-RNX-Protocol: 0`

The frame carries the package name, the requested version, every file as text, and the readme, docs, engine range, keywords, and license. The license is the manifest's `license` field when present (a known SPDX id), otherwise the SPDX inferred from the LICENSE text, falling back to `MIT`. The server reads the package identity out of `Project.config` itself, checks the frame against it, minifies what it stores, redacts the published version, and analyses the whole tree before storing. A 201 means the version is live and names the stored manifest digest; anything else prints the rejection status and message with the hint to check the token and the version. Unknown SPDX ids fail the publish with 400 on the server and `E108` on the client before anything is sent.

The client validates the license before posting: the package root must hold a non-empty `LICENSE` (or `LICENSE.txt`) file or the publish stops with `E108`, and a declared `license` must read as that grant or the publish stops naming the mismatch. The server enforces the same floor: no LICENSE file, or an unknown license string, rejects the publish.

Publishing canonicalizes on the server. `rnx publish` first refuses unformatted trees with `E108` naming the files (run `rnx fmt` and retry; the gate is UX only, the server trusts nothing), then extracts the API docs so doc comments survive in the payload, then ships the source tree as text. The server minifies `.rnx` sources itself — comments stripped — stores `Project.config` with its version redacted, and fails the publish closed if anything does not parse. The manifest digest covers the canonical bytes.

The server stores the submitted API snapshot after checking it: the payload must parse as docgen JSON (`{"modules":[...]}` with typed entries per module), fit the same 1 MiB cap as storage, and carry no pre-rendered HTML — unknown fields are dropped before the row is written, and anything misshapen fails with 400 (oversize with 413). The stored snapshot is what `GET .../{version}/api` serves, so the package page and the registry API always agree.

Declarations are checked against the shipped code. The server analyses the whole canonical tree with the real capability analyzer (cross-file call-graph walk, entry-anchored) plus a textual sink scan over files no entry root reaches, and rejects the publish with 400 when the code uses a domain head the manifest does not declare — the message names the symbol, the file, and the `Project.config` permission entry that would cover it. Declaring a wider ceiling than the code uses is fine; only undeclared use is rejected. Analysis failures (unparseable files, unresolvable imports) reject the publish too: never silently unscanned.

The entry names of the tree are checked too. A name that could escape the destination directory on the client (`../`, a Windows separator, a drive prefix, an absolute path) fails the publish with 400 and a message naming the entry, before the version is written. The rule is the one in "Entry names in a chunk manifest" above.

Server-side auth is an exact match against the configured org token. A local server without one accepts `preview-` tokens and logs a warning — a deliberate hole for local development, not for deployment. Browser sessions can publish too: then the username owns the package and scope membership is checked per scope.

## Third-party registries

A manifest can name its own registry and per-scope overrides:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    registry: {
        url: "https://rasmalai.rovelstars.com",
        token_env: "RNX_TOKEN"
    },
    registries: {
        "@acme": { url: "https://rnx.acme.internal" }
    }
}
```

Each table needs `url` and accepts `token_env` (the environment variable holding that registry's token) and `ca_cert` (a custom root CA for a corporate proxy). The token never lives in the manifest — only the variable name does.

These tables parse and validate today, but no compiler path fetches through them yet: a semver or tarball dependency fails the build with `needs a registry fetch (not implemented yet)`. The server contract they will plug into is fixed by spec: the base is a domain with no path prefix, every compliant server answers the same `/api` paths, and `GET /api/version` plus the `rnx-registry-spec` header drive discovery. A client meeting a spec it does not support is a hard error naming both numbers, and calls needing a capability the server lacks fail closed naming it. The CLI sends no spec header yet, so treat third-party hosting as server-ready and client-pending.

## Artifacts are pinned OCI images

A package that ships platform binaries or generated bindings declares one bare OCI repo ref — intent only, no tags, no digests:

```rnx
export default {
    project: { name: "codec", version: "0.3.0" },
    entries: { lib: "src/lib.rnx" },
    artifacts: "ghcr.io/org/codec"
}
```

A digest-bearing or keyed `artifacts` table is rejected with a named error. The index digest travels separately, asserted at publish time:

```sh
rnx publish --artifact sha256:<64 hex chars>
```

The assertion rides the publish frame as `artifactDigest` (`null` when the package declares no `artifacts`) and is never written to stored config. The server resolves the declared ref fresh over TLS, fetches the index (capped at 1 MiB), hashes it, and fails the publish closed on mismatch; a declared `artifacts` with a null frame digest, or a frame digest with no declared `artifacts`, both reject. The resolved digest is stored as version metadata and echoed back as `artifactDigest` in the publish response.

Locked builds pin the artifact like the version: `Project.deplock` records an `artifactDigest` on each locked entry, resolve serves the pin with the version metadata, and the client verifies the index digest first, then the per-blob hashes from the verified index, selecting role and platform from the descriptors. Each native entry's `from` title resolves per role into the content-addressed caches (project `.rnx-cache/artifacts/`, durable global `~/.cache/rnx/artifacts/<hash>`, hardlink with copy fallback); a dev checkout shadows the cached entry with a notice. A mismatch at any leg aborts the build.

## Import sources

`dependencies` maps a package name to one of five source shapes. The key must be the dependency's own `project.name`.

A registry requirement is a range string:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        sqlite3: "^3.45.0",
        "@acme/ui": "~1.2.0"
    }
}
```

It resolves through the range rules above: `rnx lock` writes the picked versions to `Project.deplock`, and `rnx update` moves them (see Lockfile and rnx update).

A path dependency is a string starting with `.` or containing `/`, or an object with `path`:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        physics_2d: { path: "../physics_2d" }
    }
}
```

The directory must contain its own `Project.config`. Members of the same workspace resolve by name without a `dependencies` line.

A git dependency pins exactly one of `rev`, `tag`, or `branch`:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        physics: { git: "https://github.com/org/physics", tag: "v1.2.0" }
    }
}
```

`tag` and `branch` store as the pinned ref; `rev` holding `main`, `master`, or `HEAD` is rejected with instructions to use `branch` for a moving target. Checkouts cache under `<workspace-root>/.rnx-cache/cache/git/`: a shallow `--branch <rev> --depth 1` clone first, falling back to a full clone plus checkout, stamped with a `.rnx-fetch` marker. A `vendor/<name>/` directory containing a manifest wins over the network without a flag.

A tarball dependency pairs a version with a URL and an optional checksum:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        zstd: { version: "1.5.5", url: "https://example.com/zstd.tar.gz", checksum: "sha256:..." }
    }
}
```

A native dependency names a system library and never mixes with `git`, `version`, or `url` (a `link` key may ride alongside any shape: `static`, `dynamic`, or `auto`, with `dynamic` reserved for native entries):

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        zlib: { native: "z", system: true, link: "dynamic" }
    }
}
```

`native` is the library name, `system: true` means the host provides it, and an optional `path` vendors it instead. `version` constrains the source (installed host library for `system: true`, declaring-package version otherwise), and `from` selects a blob by title from the package's own `artifacts` image, implying `system: false`. The combinations are exclusive: `system: true` with `from`, or `path` with any of `system: true`, `version`, or `from`, is an `E108` error — as is `from` without a declared `artifacts` image. `link: "dynamic"` on a path, git, tarball, or registry entry is an `E108` error; on native entries `auto` picks `dynamic` when a shared object exists and `static` otherwise. A `system: true` entry with a `version` is verified at lock time through `pkg-config`, failing closed on mismatch. At `rnx build` time the linker follows the effective choice recorded for each native entry: a system entry links the host shared object (`-l<lib>`), while a bundled entry links the staged `from` blob as a static archive. A bundled entry whose blob is missing fails the build as `E108` naming the title and the remedy instead of falling back to the host library; at `rnx run` time the JIT resolves each symbol from the host loader. Importing a native entry as a module is an error — call it with `from native "lib"` (see [Hardware and FFI](/manual/13-hardware-and-ffi)). The final consumer re-pins any transitive native source from the root `native` table or the `--prefer-system` / `--prefer-bundled` flags; see [Project and Toolchain](/manual/16-project-and-toolchain).

Bare versus subpath imports resolve against the dependency's `entries`. A bare `"pkg"` loads `entries.main` (`src/main.rnx` by default) and falls back to `entries.lib` when main is missing. `"pkg/sub"` probes `src/sub.rnx`, `src/sub/mod.rnx`, then `src/sub/index.rnx` under the package root, and any path escaping the package fails. A scoped `"@scope/pkg"` splits the same way after the second segment.

## Project.config is a subset of rnx

The manifest is rnx syntax but not normal rnx. The module may hold only `const` declarations and exactly one `export default`; the default must evaluate to an object. Everything else is `E108`, phrased as `<thing> is not permitted in configuration files`. The full reference is:

```rnx
const lib = target.os == "windows" ? "zlibstatic" : "z";

export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        zlib: { native: lib, system: true }
    }
}
```

Allowed: strings, integers, and booleans; records (and map literals) and arrays; `==`, `!=`, `&&`, `||` over booleans; unary `!` over booleans and unary `-` over integers; ternaries with a boolean condition; `switch` with literal or wildcard patterns and expression arms, no guards, no block arms; `target.os`, `target.arch`, and `target.env`; field access on objects; array and record spread of the matching shape; references to earlier `const` bindings.

Forbidden, each with its own `E108`: function calls, closures, and `fn` declarations; indexing and casts; float literals; string interpolation (plain strings are fine); `import`, re-export, and export-list statements; type declarations (`struct`, `class`, `enum`, and the rest); `let` bindings (use `const`); loops and any other statement; member access on non-objects; `target` properties outside `os`, `arch`, and `env`; spreading a non-array into an array or a non-object into a record; duplicate `const` names and duplicate `export default`. Arithmetic operators other than the comparisons above are rejected as arithmetic.

## End to end: sqlite3 via rnx-bindgen

`tools/rnx-bindgen` turns a C header into a Rasmalai package by dumping the clang AST as JSON, keeping whitelisted functions, transparent structs, and integer defines, and emitting `src/lib.rnx` plus `src/types.rnx`. The full command, flags exactly as the tool defines them:

```sh
rnx run --backend cranelift tools/rnx-bindgen/src/main.rnx -- \
  --header /usr/include/sqlite3.h --native-lib sqlite3 --pkg-name sqlite3 \
  --pkg-version 3.53.4 --prefix sqlite3_ --out packages/sqlite3 \
  --skip sqlite3_carray_bind --skip sqlite3_win32_set_directory
```

`--prefix` keeps matching names (repeatable; empty keeps the legacy allowlist) and `--skip` drops exact function names that the header declares but the shipped library does not export — native backends resolve foreign symbols eagerly, so one missing symbol fails the whole package. Optional `--include <dir>` (repeatable) and `--clang-flags "<flags>"` extend the compiler search path and flags.

For day-to-day use, declare the driver once in `Project.config` and replay it with `rnx task bindgen` instead of repeating flags; the `bindgen` table (`headers`, `include`, `prefix`, `skip`, `native_lib`, per-output titles) is documented under [Project and Toolchain](/manual/16-project-and-toolchain).

The tool writes `<out>/src/lib.rnx` (and `src/types.rnx` when structs or constants exist) directly, and prints the manifest between `---BEGIN Project.config---` markers because runtime writes to any `Project.config` path abort with `S301`. Extract it with:

```sh
sed -n '/---BEGIN Project.config---/,/---END Project.config---/p' run.log \
  | sed '1d;$d' > packages/sqlite3/Project.config
```

The generated layout is `src/lib.rnx` (foreign signatures ending in `} from native "sqlite3"`), `src/types.rnx` (structs and constants), and a manifest of this shape:

```rnx
export default {
    project: { name: "sqlite3", version: "3.53.4" },
    entries: {
        main: "src/lib.rnx"
    },
    dependencies: {
        sqlite3: {
            native: "sqlite3",
            system: true
        }
    },
    permissions: ["unsafe:ffi"]
}
```

Depend on it by path while iterating:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        sqlite3: { path: "../sqlite3" }
    }
}
```

```rnx
import { sqlite3_open } from "sqlite3";

print(sqlite3_open);
```

Publish with `rnx pack` then `rnx publish` as above; once registry fetching lands, the same name works as a `"^3.53.4"` range without the path line. The `unsafe:ffi` permission stays in the manifest: foreign calls need it, and `rnx audit` reports it. When a grant needs justification, declare it as `{ perm: "unsafe:ffi", reason: "calls into libsqlite3" }` instead of the bare string; the registry stores the reason and the package page lists it beside the grant.

## Dist-tags roadmap

`latest` today is computed as the highest stable version, not stored — there are no named tags yet. The plan is npm-style `tags` per package where `latest` auto-advances on stable publishes but never on prereleases, `rnx publish --tag beta` moves one tag without touching `latest`, requirement positions accept tag names, and servers advertise the behavior under a `dist-tags` capability that older clients ignore.

## Summary

- Names are unscoped or `@scope/name`, lowercase, first publisher owns the name, versions never repeat.
- Yank keeps a version installable-but-deprecated; takedown answers 410 forever; both burn the number.
- Reads are anonymous under fixed `/api` paths; resolve takes requirements plus a have-list and returns integrity inline.
- `rnx publish` posts bytes with five exact headers and `--token`/`RNX_TOKEN` auth.
- Dependencies come from registries, paths, git, tarballs, or native libraries; bare imports use `entries.main` with an `entries.lib` fallback.
- `Project.config` allows only `const` plus one `export default` over values, comparisons, ternaries, `switch`, and `target`; everything else is `E108`.
