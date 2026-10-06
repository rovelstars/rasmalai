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

Retries are safe. Publish accepts an `X-RNX-Request-ID` idempotency key; repeating a publish with the same key returns success with `duplicate: true` instead of a second row.

## Yanked versus taken down

Yank is reversible-looking but one-way, and takedown is forever. Both keep the number burned per the rule above.

`POST /api/packages/<name>@<version>/yank` flips a live version to `yanked`. The metadata still serves, now with `"status": "yanked"`. Pinned requirements keep resolving — yanked versions stay in the candidate set — but the latest pointer and the catalog skip them, so new floating ranges move past. Yanking a version that is not live fails with 409 (`version <v> is not live`).

`POST /api/packages/<name>@<version>/takedown` needs the admin token. It writes a tombstone row carrying the reason and every later read of that version answers 410 `{ code: "withdrawn", reason }` with a one-year immutable cache. The number never comes back.

Only the `@std/*` scope has a further carve-out, `DELETE /api/scopes/{scope}/{name}@{version}`, which needs the org admin plus a second approval. It has no CLI path.

## Reading from the registry

Reads take no credentials. Every response carries an `rnx-registry-spec: 1` header. `GET /api/version` reports the contract in one place:

```json
{ "spec": 1, "capabilities": ["tombstones", "guides", "jsdoc", "yank", "transfer"], "registry": "rasmalai.rovelstars.com" }
```

The fixed paths are:

| Request | Result |
|---|---|
| `GET /api/packages` | catalog of package summaries |
| `GET /api/packages/<name>` (or `@<scope>/<name>`) | 302 to the latest versioned URL, never content |
| `GET .../<name>@<version>` | pinned version; older `/<name>/<version>` path form 308-redirects here |
| `GET .../<name>@<version>` | metadata: status, checksums, engine range, api/manifest/chunks/guides links |
| `GET .../<version>/api` | the jsdoc snapshot JSON |
| `GET .../<version>/manifest` | the stored dependency manifest (deps, homepage) |
| `GET .../<version>/chunks` | chunk entry manifest (file names, sizes, content hashes) for browser-side reassembly |
| `GET .../<version>/chunk/<sha256>` | one content chunk by hash, immutable — browsers fetch these in parallel and unpack locally |
| `GET .../<name>@<version>/guides[/<slug>]` | guide index or one guide, 404 when the package ships none |

Version metadata and content chunks cache as immutable for a year; a missing name or version answers 404 JSON and a tombstoned version answers 410.

## Status code reference

| Status | Meaning | Body |
|---|---|---|
| `200` / `201` | version is live | metadata or tarball bytes |
| `302` | name without version | redirect to the latest versioned URL, never content |
| `400` | bad input | `invalid package name`, `bad-range` |
| `403` | not the owner | `package <name> is owned by someone else`, `scope @<scope> is not yours` |
| `404` | missing name, version, or guide | JSON error |
| `409` | version reuse or bad yank | `version <v> already published (versions are immutable)`, `version <v> is not live` |
| `410` | tombstoned version | `{ code: "withdrawn", reason }`, cached immutably for a year |

Every response carries an `rnx-registry-spec: 1` header. A client meeting a spec it does not support is a hard error naming both numbers, and calls needing a capability the server lacks fail closed naming it.

## Resolving a dependency graph

`POST /api/resolve` takes one request with a `requirements` map and a `have` list of already-fetched versions:

```json
{
  "requirements": { "sqlite3": "^3.45.0", "@acme/ui": "~1.2.0" },
  "have": [{ "full": "sqlite3", "version": "3.45.0", "integrity": "sha256:..." }]
}
```

The server picks the highest satisfying version for each requirement, walks manifests transitively, and returns `resolved` plus depth-leveled `levels`. Each node carries its chunk-manifest `path`, `integrity` digest, engine range, and a `yanked` flag. Clients fetch the manifest, pull content chunks in parallel, reassemble the tarball bytes locally, and verify against `integrity`. Entries in `have` whose integrity still matches skip dependency expansion, so repeated resolves only fetch the delta.

Ranges use a small grammar: `*`, `latest`, an exact `X.Y.Z`, `^X.Y.Z`, `~X.Y.Z`, or `>=X.Y.Z`. Anything else is a 400 `bad-range`. The semantics:

- `*` and `latest` match any stable version, never a prerelease.
- An exact version matches exactly it; a prerelease matches only when the range names a prerelease with the same major, minor, and patch.
- `^1.2.3` means at least `1.2.3` with the same major version. `^0.2.3` pins the minor as well, and `^0.0.3` pins the patch.
- `~1.2.3` means at least `1.2.3` with the same major and minor.
- `>=1.2.3` means that version or newer.

Conflicting ranges for one package fail the whole resolve as `unsatisfiable`; a dependency cycle fails as `cycle`.

Every resolve error response carries a `requestId` (a UUID v4) alongside `code` and `message`. The server logs the failure under `[resolve <id>]`, so paste the id when reporting a 500 `resolve-failed`.

## Publishing

`rnx pack` builds the deterministic archive (ustar plus SHA-256, `--gzip` for the compressed form). `rnx publish` ships it:

```sh
rnx pack --gzip
rnx publish ./colony-0.4.0.tar.gz --registry https://rasmalai.rovelstars.com/api/packages --token <token>
```

Omit the tarball and `rnx publish` packs the current project into a temporary directory first. `--registry` defaults to `https://rasmalai.rovelstars.com/api/packages` and `--token` falls back to the `RNX_TOKEN` environment variable. Without either, the command stops with `E501` and the hint `pass --token <token> or set RNX_TOKEN in your environment`.

The CLI posts the raw bytes with exactly these headers:

- `Authorization: Bearer <token>`
- `Content-Type: application/octet-stream`
- `X-RNX-Package-Name`, `X-RNX-Package-Version`, `X-RNX-Checksum`

The server also honors `X-RNX-Meta` (readme, docs, guides, manifest JSON) and `X-RNX-Request-ID` for idempotent retries. A 200 or 201 means the version is live; anything else prints the rejection status and message with the hint to check the token and the version.

Publishing minifies on the client before upload. `rnx publish` first refuses unformatted trees with `E108` naming the files (run `rnx fmt` and retry), then extracts the API docs so doc comments survive in `X-RNX-Meta` (`metaVersion` 3), then ships minified `.rnx` sources — comments stripped, `Project.config` sent formatted as-is — and fails closed if the minified output does not re-parse to the same token stream. Checksums cover the minified bytes.

Server-side auth is an exact match against the configured org token. A local server without one accepts `preview-` tokens and logs a warning — a deliberate hole for local development, not for deployment. Browser sessions can publish too: then the username owns the package and scope membership is checked per scope.

## Third-party registries

A manifest can name its own registry and per-scope overrides:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    registry: {
        url: "https://rasmalai.rovelstars.com/api/packages",
        token_env: "RNX_TOKEN"
    },
    registries: {
        "@acme": { url: "https://rnx.acme.internal/api/packages" }
    }
}
```

Each table needs `url` and accepts `token_env` (the environment variable holding that registry's token) and `ca_cert` (a custom root CA for a corporate proxy). The token never lives in the manifest — only the variable name does.

These tables parse and validate today, but no compiler path fetches through them yet: a semver or tarball dependency fails the build with `needs a registry fetch (not implemented yet)`. The server contract they will plug into is fixed by spec: the base is a domain with no path prefix, every compliant server answers the same `/api` paths, and `GET /api/version` plus the `rnx-registry-spec` header drive discovery. A client meeting a spec it does not support is a hard error naming both numbers, and calls needing a capability the server lacks fail closed naming it. The CLI sends no spec header yet, so treat third-party hosting as server-ready and client-pending.

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

It resolves through the range rules above once registry fetching lands; today it fails the build as not implemented.

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

A native dependency names a system library and never mixes with `git`, `version`, or `url`:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        zlib: { native: "z", system: true }
    }
}
```

`native` is the library name, `system: true` means the host provides it, and an optional `path` vendors it instead. At `rnx build` time the linker appends `-l<lib>` automatically; at `rnx run` time the JIT resolves each symbol from the host loader. Importing a native entry as a module is an error — call it with `from native "lib"` (see [Hardware and FFI](/manual/13-hardware-and-ffi)).

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
