---
title: "Project and Toolchain"
description: "Project.config spec, modules, SemVer deps, testing, docs, and the rnx CLI."
section: "Toolchain and Diagnostics"
icon: "Terminal"
---

# Project and Toolchain

## Installation

One static binary — `rnx` — covers editing, running, testing, formatting, documenting, and shipping. Installation places `rnx` on `PATH`; `rnx --version` confirms the install and prints the build record (commit hash, build timestamp, install kind, target triple, backend versions) after the `rnx <version>` first line. Scaffolding creates the manifest and entry file:

Distributors building `rnx` for a managed install set two build-time variables. `RNX_INSTALLATION_MANAGED` carries the reason shown after `install: managed:` — for example how the package was installed and how to upgrade it; unset, the install reads `install: standalone`. `RNX_INSTALLATION_NO_TIMESTAMP` (any non-empty value) leaves the timestamp out, so the record reads `built: unknown`; it wins over `SOURCE_DATE_EPOCH` when both are set.

```sh
rnx init hello
cd hello
```

## Project.config

The manifest is an `.rnx` module exporting a default object. It declares package identity, entry points, and dependencies. `project` requires `name` and `version`; `engine` states the minimum toolchain requirement. An optional top-level `entries` object names the files — `main` defaults to `src/main.rnx`, `lib` names the library file, `docs` the guides folder, and `bins` maps tool names to files. The old `entry` and `edition` keys are rejected outright; there is no compatibility alias.

```rnx
export default {
    project: {
        name: "colony",
        version: "0.4.0",
        engine: ">=0.4.0"
    },
    entries: {
        main: "src/main.rnx"
    },
    registry: {
        url: "https://rasmalai.rovelstars.com",
        token_env: "RNX_TOKEN"
    },
    dependencies: {
        sqlite3: "^3.45.0",
        "@rovelstars/ui": "~1.2.0",
        physics_2d: { path: "../physics_2d" },
        physics: { git: "https://github.com/org/physics", tag: "v1.2.0" },
        zstd: { version: "1.5.5", url: "https://example.com/zstd.tar.gz", checksum: "sha256:..." },
        zlib: { native: "z", system: true }
    }
}
```

Manifests evaluate in a sandbox: top-level `const`, object and array spreads (`...deps`), ternaries, `switch` over the ambient `target` object (`target.os`, `target.arch`, `target.env`), and member access. Functions, loops, imports, and I/O are rejected with `E108`. The exact allowed-versus-forbidden list lives in [Packages and Registries](/manual/18-packages-and-registries). A conditional native dependency looks like this:

```rnx
const libs = {
    zlib: switch (target.os) {
        case "windows": { native: "zlibstatic", system: true }
        default: { native: "z", system: true }
    }
}

export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        ...libs,
        gui: target.arch == "aarch64" ? { native: "gui_arm" } : { native: "gui_x86" }
    }
}
```

## SemVer comparators

Version requirements follow SemVer 2.0: `^1.2.0` permits compatible minor and patch changes; `~1.2.0` permits patch-level changes only; a bare version pins exactly; ranges combine comparators. A git dependency pins exactly one of `rev`, `tag`, or `branch` (`rev` holding a branch name is rejected). A `path` entry is a local checkout; git checkouts cache under `.rnx-cache/cache/git/`; a `.vendor/<pkg>/` directory wins over the network, for git and registry dependencies alike (see [Packages and Registries](/manual/18-packages-and-registries)). `RNX_VENDOR_DIR` adds one extra override lookup directory. The token never lives in the manifest: `token_env` names the environment variable, and `ca_cert` pins a corporate root CA.

`rnx add <name>` resolves the latest stable version from the configured registry and records it as `^<version>`; `rnx add <name>@<range>` records the given range verbatim (`*`, `latest`, `X.Y.Z`, `^`, `~`, `>=`). The lockfile pins the resolved version digest, never the range or a tag name. `rnx add <name> --path <dir>` keeps the local flow: the directory is recorded as a `path` entry.

| Requirement | Matches | Skips |
|---|---|---|
| `^1.2.0` | `>=1.2.0`, same major | `2.0.0`, prereleases |
| `^0.2.3` | `>=0.2.3`, same minor | `0.3.0` |
| `^0.0.3` | exactly `0.0.3` | anything else |
| `~1.2.0` | `>=1.2.0`, same major and minor | `1.3.0` |
| `1.2.3` (bare) | exactly `1.2.3` | anything else |
| `*` / `latest` | any stable version | prereleases |

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    dependencies: {
        sqlite3: "^3.45.0"
    }
}
```

## Capability tiers

| Tier | Meaning | Examples |
|---|---|---|
| `pure` | no external reach | arithmetic, data layout |
| `delegated` | acts only on caller-provided values | `fs:delegated`, `net:delegated` |
| `ambient` | reaches named external resources | `fs:read:/data`, `net:http:example.com` |
| `hazard` | spawns processes, touches raw memory, or calls foreign code | `sys:exec:*`, `unsafe:ffi`, `unsafe:raw_memory` |

Adding a dependency whose surface reaches the ambient or hazard tiers requires explicit approval through `rnx add --accept-caps <list>` or `--accept-all-caps`.

Unknown packages fail as `E108`; circular package dependencies fail as `E107`, which is reserved exclusively for external package graphs and is never emitted for intra-project file imports. Unrecognized manifest sections or fields warn as `W201` and are ignored.

## Entry and exit codes

Programs start at the entry file's top-level statements — no wrapper required. An explicit `main`/`Main` still works when you want a named entry point, but define exactly one: `main` and `Main` together in the entry file is an `E108` error. `rnx run` prefers lowercase `main` when resolving a default entry, and `rnx build` compiles it by renaming the symbol clear of the C-level `main`. A top-level `return <Int>` (or the entry function's return value) becomes the process exit code; `0` means success.

```rnx
print("hello,", "world");
```

`Process.exit(code)` terminates immediately with a code, flushing output already written to stdout first; `Process.args()` reads CLI arguments as `Array<String>` — every element is a `String`, index 0 is the program path, and user arguments follow a `--` separator from index 1. `"5" + 1` concatenates to `"51"`: convert explicitly before doing arithmetic, since `Int()` on a `String` is an `E108` error. A parameterless `Main` ignores extra arguments on every backend; the JIT backends pass declared `Main` parameters as integers only. `rnx build --release` compiles through LLVM `-O3` and links a stripped native executable.

## Modules and imports

Each file is its own namespace. Between files in one project, default and `public` items import freely in any direction (`A -> B -> C -> A` included); locals shadow imports. Only `private` items are file-locked — importing one is an `E203` error.

```text
import { add } from "./math";
import { make_origin as origin } from "./shapes";
import * from "./shapes";
import math from "@std/math";
import engine, { Config } from "./engine";
import "./setup";
```

A default import binds the module namespace; `import * as ns` is the explicit spelling of the same binding; `import "./setup"` loads a file for declarations alone. Relative imports probe `<path>.rnx`, `<path>/mod.rnx`, `<path>/index.rnx` in order, and the error lists every path tried. Specifiers with a `:` are rejected as `E108` (`std:time` becomes `@std/time`). There is no `::` in the language; paths, variants, and generics use `.` and `<>`.

`@std/` imports resolve from the standard library compiled into `rnx` itself — nothing to install, no versions to pin. Seventeen modules ship with the compiler: `prelude`, `simd`, `math`, `collections`, `fs`, `bytes`, `time`, `random`, `sync`, `env`, `process`, `os`, `testing`, `web`, `json`, `net`, and `io`. `@std/net` provides the TCP, TLS, and DNS primitives the `net:*` capability domain refers to. `@std/io` owns the terminal streams and the `term:*` capability domain: `write`/`writeError` pretty-printing, `writeRaw` for exact bytes, `read`/`readLine` line input, `isTTY`/`width`/`height`/`colorProfile` queries, `setRawMode`, and TTY-guarded `clear`. Every file also sees `@std/prelude` without importing it; local declarations and imports always win, and anything found in neither scope is `E303`. Each `@std/*` module ships reference docs at `/docs/@std/<module>/overview` (overview, getting-started, API reference), and any published `@scope/<name>` package gets the same pages at `/docs/@scope/<name>/overview`.

```rnx
import { Vec2 } from "@std/math";
import { Map } from "@std/collections";

let m = new Map<String, Int>();
m.set("ore", 7);
let v = new Vec2(3.0, 4.0);
print(m.get("ore"), v.length());
```

Bare and scoped specifiers name registry or path dependencies from the nearest `Project.config`: `"pkg"` loads that package's entry, `"pkg/sub"` loads `src/sub.rnx` inside it, and `"@scope/pkg"` checks the matching scoped registry first. Transitive dependencies resolve against each package's own manifest; diamond imports parse once, deduplicated by canonical path.

## Lockfiles and workspaces

`rnx lock` traverses the transitive graph, checksums every package directory (SHA-256 over sorted paths and bytes), and writes a sorted, deterministic `Project.deplock`. From then on, `rnx run --locked` and `rnx build --locked` re-verify every checksum before executing. A `Project.config` with a `workspace` object (a `members` list supporting `/*` expansion over sibling manifests) marks a monorepo root whose members resolve to each other by name; `rnx lock` writes one consolidated root lockfile.

## Permissions and capability audits

Every package carries a statically deduced capability surface: what files, sockets, processes, environment variables, and `unsafe` operations its code can reach. Capabilities are strings in `domain:action:scope` form:

- `fs:read:<path>`, `fs:write:<path>`, `fs:delegated` (operates only on caller-provided paths)
- `net:http:<host>`, `net:ws:<host>`, `net:delegated`
- `sys:exec:<binary>`
- `env:read:<name>`, `env:dump` (also written `env:read:*`)
- `unsafe:ffi`, `unsafe:raw_memory`

Each capability sits in one of four tiers: `pure` (no external reach), `delegated` (acts only on caller-provided values), `ambient` (reaches named external resources), `hazard` (spawns processes, touches raw memory, or calls foreign code). Static attribution covers file, process, environment, `fetch`/`WebSocket`, and `unsafe` sinks; raw TCP/TLS/DNS dial sites (`TcpStream.connect`, `TcpListener.bind`, `Dns.lookup`) are not yet attributed to a capability. `rnx audit` prints the tier badge, the capability list, and the call trace behind each entry, plus lockfile drift when the scan disagrees with `Project.deplock`:

```sh
rnx audit --path ../dep        # audit another directory (defaults to current)
rnx audit --json               # full report as JSON for CI
rnx audit --export-manifest    # minimal publishing manifest as JSON
rnx audit --registry https://rnx.acme.internal  # check advisories against another registry (or RNX_REGISTRY)
```

When the directory holds a `Project.deplock`, the audit also fetches the registry-signed advisory feed (`GET /api/v0/advisories`), verifies the operator signature, and matches the locked entries — versions and `artifactDigest` pins — against it. A `critical` finding fails the audit with a security hold; a `warning` warns exactly like a yanked version; an `info` finding is a plain note. A feed whose signature does not verify fails the audit closed; a feed that cannot be reached only warns and the audit passes on the capability report alone. `RNX_ADVISORY_KEY` pins the expected feed key id or public key for registries whose key is known out of band. The audit never rewrites the lockfile: re-pinning an affected dependency stays an explicit update step.

There is no `--unsafe` flag: the default audit already prints every capability, including `unsafe:*` grants. `unsafe` is a language block, not a CLI switch, and capability checks stay enforced at compile time.

Two files bound the surface. `Project.config` takes an optional ceiling. Each entry is either a bare capability string or a `{ perm, reason }` table; the reason is shown on the package page next to the grant, and entries without one render as "no reason given":

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    permissions: [
        "net:http:example.com",
        { perm: "fs:read:/data", reason: "seed fixtures for the import job" }
    ]
}
```

Any deduced capability outside `allowed` — counting the package's own code plus every dependency — fails the build as `S102`, on `rnx check` and `rnx build` alike, with or without a `Project.deplock`. Bare `rnx run` executes without enforcing the ceiling; `rnx run --locked` enforces it. `Project.deplock` records the approved ledger: each entry stores its `tier` and `capabilities`, written by `rnx lock` and re-verified by `--locked` builds. Hand-editing the ledger does not grant anything; the next `rnx lock` overwrites it from the scan, and the next `--locked` build fails on the difference (`S101`). Adding a dependency whose surface reaches the ambient or hazard tiers requires explicit approval through `rnx add --accept-caps <list>` or `--accept-all-caps`.

## Testing and benchmarking

A `test fn` takes no parameters and returns nothing. The runner discovers every one in `src/**/*.rnx` and `tests/**/*.rnx`, times each, and reports one line apiece. `assert` takes a `Bool` condition and a `String` message; on false it prints the message and marks that test failed without aborting it. `test fn` blocks are stripped from normal `run`/`build` pipelines. Tests run on the Cranelift JIT by default; pass `--backend interpreter` or `--backend llvm` to exercise the interpreter or AOT codegen. Filter by substring, pass `--exact` for full names. Output captures per test; `--nocapture` streams live.

```rnx
test fn adds_up() {
    assert(1 + 1 == 2, "math");
}

test fn starts_empty() {
    let items: Array<Int> = [];
    assert(items.length == 0, "fresh array");
}
```

`bench` blocks report timings instead of pass/fail; `blackBox` from `@std/testing` wraps values the optimizer must not fold away. `rnx bench` runs them (LLVM release by default, filterable, backend-selectable).

```rnx
import { Vec4f } from "@std/simd";
import { blackBox } from "@std/testing";

bench "vec add" {
    let v = new Vec4f(1.0, 2.0, 3.0, 4.0) + new Vec4f(1.0, 1.0, 1.0, 1.0);
    blackBox(Int(v.x()));
}
```

## Documentation

A `/** */` block documents the item that follows it: first paragraph of description, then one tag per line — `@param <name>`, `@returns`, `@throws`, `@example`, `@see`. `//!` lines document the module itself. Plain `//` remarks never attach to items. `pub` items without `/** */` docs warn as `L004` under `rnx lint`.

```rnx
/**
* Scale a quote by demand pressure.
*
* @param base price per unit.
* @returns scaled quote.
*/
fn quote(base: Float): Float {
    return base;
}

print(quote(2.0));
```

`rnx doc` collects documented items into a static site under `target/doc` (`pub` only; `--all` adds internals; `--open` opens a browser; `--json` writes `api.json`). Every `@example` in the standard library executes as a documentation test and fails the build when broken.

## CLI command reference

| Command | Purpose | Key flags |
|---|---|---|
| `rnx check` | rapid lexer/parser/module/typecheck, no codegen | `[paths...]`, `-p`, `--json` |
| `rnx dev` | watch sources, hot-swap function bodies without restart | `[path]`, `--entry F`, `--no-rerun`, `--mcp` |
| `rnx repl` | interactive JIT shell | none |
| `rnx run` | execute program (Cranelift JIT by default) | `--entry F`, `--backend <interpreter\|cranelift\|llvm>`, `-p`, `--locked`, `-O`, `-- <args>` |
| `rnx build` | link native binary into `.rnx-cache/build/{dev,release}/<name>` | `--release`, `--lib`, `--emit-obj`, `--target`, `--entry F`, `-p`, `--locked`, `-O`, `-g/--debug`, `--deny-perf-lints[=codes]`, `--show-perf-notes`, `--fix[--force]`, `--fix-keep-backup` |
| `rnx test` | run `test fn` blocks (Cranelift JIT by default) | `[filter]`, `-p`, `--backend`, `-O`, `--exact` |
| `rnx bench` | time `bench` blocks (LLVM release by default) | `--filter`, `-p`, `--backend`, `--release/--no-release` |
| `rnx doc` | docs to `target/doc` (HTML or `--json`) | `-p`, `--open`, `--no-deps`, `--all`/`--private`, `--json`, `--stdlib`, `--out-dir` |
| `rnx lint` | static checks over sources | `[paths...]`, `-p`, `--sarif`, `--json`, `--deny-warnings`, `--fix` |
| `rnx fmt` | format `.rnx` sources | `[paths...]`, `--check`, `--diff` |
| `rnx add` | add a registry (`<name>[@range]`) or local path dependency with capability approval | `<package>[@range]`, `--path <dir>`, `--accept-caps <list>`, `--accept-all-caps` |
| `rnx publish` | publish the current project to the registry (pushes the OCI image when `RNX_OCI_PUSH_URL` is set) | `--registry <url>`, `--token <token>`, `--artifact <digest>` |
| `rnx lsp` | Language Server Protocol over stdio | none |
| `rnx mcp` | Model Context Protocol server over stdio | none |
| `rnx explain` | diagnostic text plus fix | `<code>` |
| `rnx audit` | report capability and tier surface per package, plus advisory check against the lockfile | `--path <dir>`, `--json`, `--export-manifest`, `--licenses`, `--deny-license <spdx>`, `--registry <url>` |
| `rnx bindgen` | generate `bindgen.rnx` plus OCI annotations from C headers | `--header <file>`, `--native-lib <lib>`, `--pkg-name <name>`, `--out <dir>`, `--target`, `--prefix`, `--skip`, `--include`, `--case keep\|camel\|pascal`, `--rename old=new`, `--check` |
| `rnx task` | run an author-local task from `Project.config` (`bindgen` replays the driver table) | `[name]` (lists tasks when omitted) |
| `rnx completions` | shell completion scripts | `bash`, `zsh`, `fish`, `powershell`, `elvish` |
| `rnx pack` | deterministic ustar plus SHA-256 | `-p`, `--out-dir`, `--gzip/-z` |
| `rnx unpack` | verify and extract archive | `--out-dir <dir>` |
| `rnx init` | scaffold a new project | `[name]` |
| `rnx setup` | configure editor LSP and highlighting | `<vscode\|zed\|helix\|neovim>` |
| `rnx lock/fetch/vendor` | lockfiles, git-dependency cache, dep precompilation, and local source staging | `[packages]`, `--dir <path>` |
| `rnx download` | seed the registry cache with libraries (`@std/*` when no names are given) | `[packages]`, `--local`, `--registry <url>` |
| `rnx doctor` | check the stdlib cache pin and contents, plus dep product status | `--repair-std`, `--registry <url>` |

Global flags: `--no-color` strips ANSI escapes, `-v` enables pipeline logging, `-q` suppresses banners. `--prefer-system` resolves every native dependency from the host library and `--prefer-bundled` from bundled OCI blobs; the two conflict, and per-dependency `native` pins override either.

`rnx download` with no package names resolves every `@std/<module>` at `*` through the registry in a single round-trip and pins the exact versions (plus the seeding date) to a pin file next to the global cache (`RNX_CACHE_HOME` is honored). With names, it downloads `name` or `name@range` libraries (the same SemVer grammar as dependency requirements, with the engine-range check after resolve) into the global cache — `--local` targets the project-local cache (`.rnx-cache/registry/`) instead, with the pin recorded at `.rnx-cache/std-pin.json`. Global downloads land in the content-addressed store (`store/` under the cache root: chunk blobs plus one verified tree per manifest digest — see [Packages and Registries](/manual/18-packages-and-registries)); `--local` keeps the older per-version project layout instead. The installer runs `rnx download` automatically and keeps going with a warning when offline. `rnx doctor` reports a missing pin file or missing packages; `rnx doctor --repair-std` re-runs the seeding and reports how many packages it repaired. `download` and `doctor` accept `--registry <url>` (or `RNX_REGISTRY`) to point at a registry other than the default, which is how a self-hosted registry seeds its own standard library. With an empty cache and no network, `@std/*` imports fail as `E108` naming the registry instead of falling back silently. There is no embedded fallback: the binary carries no stdlib bytes. To seed or hack on the standard library without a registry instead, run `rnx vendor @std/... --dir <DIR>` from a checkout (see below), then point `RNX_VENDOR_DIR` at the staged directory: `@std/*` imports resolve straight from that tree with zero network, the same local-source override that covers every registry package (see [Packages and Registries](/manual/18-packages-and-registries)). Distributors bundle a pre-seeded cache directory and set `RNX_CACHE_HOME` at it — no second mechanism exists.

`rnx fetch` downloads every dependency of the current scope and then precompiles the scope: it loads each member's entry module so the dependency resolver records its products. Each product is mirrored from the project cache into the global store at `products/<registry-host>/<package>/<version>/<toolchain>/` (git dependencies under `products/git/<name>/<rev>/<toolchain>/`), next to a `manifest` file holding the content hash, dep set, and toolchain. A product is reused only when the exact pinned version, the toolchain hash, and the content digest all match; floating ranges never populate, and a missing or corrupt entry falls back to the normal path. `rnx build` also keeps whole-program codegen units in a content-keyed global `objects/` pool, so rebuilding the same program with an empty project cache skips LLVM emission. `rnx doctor` lists each pinned dependency as `cached` or `absent` (floating ranges are reported as unpinned), and `rnx cache status`/`prune` covers the new pools plus the store/legacy cache split: `status` shows store versus old-tree usage and `prune` reclaims old `registry/` trees while the content-addressed store is retained.

`rnx fmt` rewrites sources with the canonical style (4-space indents, Egyptian braces, single spaces around binary operators, at most one blank line, no trailing whitespace); comments and string contents are never altered and formatting is idempotent. `rnx lint` checks `L001` (unused variable), `L002` (unused parameter), `L003` (unreachable code), `L004` (missing doc comment), and `L005` (empty block).

## Native matrix and author tasks

Native dependencies choose their source per entry: a host system library or a bundled OCI blob. The declaration carries the choice:

```rnx
export default {
    project: { name: "colony", version: "0.4.0" },
    artifacts: "ghcr.io/org/colony",
    dependencies: {
        zlib: { native: "z", system: true, version: ">=1.2.0" },
        codec: { native: "codec", from: "libcodec.so" }
    }
}
```

`version` constrains the source: the installed host library for `system: true`, the declaring package's own version for bundled sources. Ranges use the same grammar as SemVer requirements (`*`, `^`, `~`, `>=`, exact). `from` selects a blob by its OCI title from the package's own `artifacts` image and implies `system: false`; a `from` entry without a declared `artifacts` image is an `E108` error. `system: true` consumes the host library only after verifying the version: the check runs `pkg-config --modversion <lib>`, falling back to `ldconfig -p` for presence. A mismatch or a missing library fails the lock closed and names the mismatch — there is no silent fallback. `path` vendors a local checkout instead and cannot combine with `system: true`, `version`, or `from`.

Every dependency accepts a `link` key with `static`, `dynamic`, or `auto` (default `auto`). Only native entries provide shared objects, so `dynamic` on a path, git, tarball, or registry entry is an `E108` error. For native entries `auto` resolves to `dynamic` when a shared object exists (a host library or a `from` title) and to `static` otherwise; an explicit `static` or `dynamic` is recorded as-is. The lockfile stores the effective choice per entry in a `link` field (`static` or `dynamic`), and changing it requires `rnx lock` — a verify against a lock with a different `link` fails naming the mismatch.

Each package also declares its grant with a top-level `license` field holding one SPDX id (`MIT`, `Apache-2.0`, `LGPL-3.0-only`, and the rest of the known set). The id must be known or the manifest fails with `E108`. The lockfile carries each locked package's license alongside its `link`, so `rnx audit --licenses` can report the graph without network access.

The library author declares the default, but the final consumer re-pins any transitive native dependency from the root `Project.config`:

```rnx
export default {
    project: { name: "app", version: "1.0.0" },
    dependencies: {
        sqlite3: "^3.45.0"
    },
    native: {
        sqlite3: { system: true, version: ">=3.40.0" }
    }
}
```

The `native` table is root-only; member and dependency tables are ignored. The `--prefer-system` and `--prefer-bundled` global flags set the default for every entry without an explicit pin (or `RNX_PREFER_NATIVE=system|bundled` when the flags are absent; the flags win). Explicit pins override both. A flag that flips a declared source keeps the declared `version` constraint and checks it against the new source — a host-oriented range that the flipped source cannot satisfy fails the lock with the mismatch named, and an explicit pin without `version` clears it. The lockfile records the effective choice per entry — `native:<lib>+system` with the installed version, `native:<lib>#<title>` with the owning image digest, plain `native:<lib>` for the automatic role-plus-platform match — so locked builds resolve offline and identically. Re-lock after changing a pin or a flag; a locked verify with different preferences fails as a source mismatch, which is the point.

Author-local automation lives in two more tables. `bindgen` drives the generator without flags:

```rnx
export default {
    project: { name: "demo", version: "0.1.0" },
    bindgen: {
        headers: ["include/foo.h"],
        include: ["include"],
        prefix: ["foo_"],
        skip: ["foo_debug"],
        case: "camel",
        rename: ["foo_open=openDb"],
        native_lib: "foo",
        outputs: {
            "src/gen": { title: "foo.rnx" }
        }
    }
}
```

`rnx task bindgen` replays the driver for every output directory, writing `<dir>/<title>` plus `<dir>/bindgen.oci.json`. (`bindgen` the task name is reserved: a `tasks.bindgen` entry is rejected.) The `tasks` table holds the rest:

```rnx
export default {
    project: { name: "demo", version: "0.1.0" },
    tasks: {
        check: ["rnx check", "rnx lint"],
        build: {
            steps: [{ run: target.os == "windows" ? "rnx build --target x86_64-pc-windows-msvc" : "rnx build" }],
            matrix: ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
        },
        all: { tasks: ["check", "build"], strategy: "parallel", "on-failure": "continue" }
    }
}
```

`rnx task <name>` runs steps with the project root as the working directory, streaming their output; `rnx task` alone lists them. A step is a shell command string, a `{ run = ... }` table, or a `{ task = ... }` reference to another task. A metatask names other tasks with `strategy: serial | parallel` and `"on-failure": stop | continue` — note the quotes, since a bare `on-failure` key is not valid record syntax. A `matrix` list of target triples evaluates every step once per triple (each leg sees its own `target`), so one table fans out across platforms; the manifest sandbox — `const`, spreads, ternaries, `switch` on `target`, member access, no interpolation — is the whole expression language, and a driver that needs more is rejected as non-declarative. Task cycles and unknown names fail closed with `E108`.

Tasks are containment-load-bearing: they run only through `rnx task` on the author's machine. They never ship — the publish frame drops `tasks` and `bindgen`, only generated files travel — they never run on consumers, and no other command reads them. There are no lifecycle hooks, and none will ever exist.

Publishing a version that declares `artifacts` can push the image itself. Stage per-target blobs under `.rnx-cache/publish/<target>/<role>/<title>` (roles `runtime`, `wasm`, `bindgen`, `media`; the author's matrix tasks produce these), point at an endpoint, and publish:

```sh
export RNX_OCI_PUSH_URL=https://ghcr.io
rnx publish
```

The pipeline replays `rnx task bindgen`, assembles one manifest per target plus the bindings manifests into a single index, cross-checks every staged file against the index before asserting, pushes blobs, manifests, and the index, then captures the index digest into the frame's `artifactDigest` automatically — no hand-copied hashes. A `--artifact` flag that disagrees with the pushed digest fails closed. Without the push URL, publish keeps the old behavior: `--artifact sha256:<hex>` asserts a prebuilt image, fetched by tag, verified by digest, and cross-checked against the local project cache (`.rnx-cache/artifacts/`, seeded from the global `~/.cache/rnx/artifacts/` store; cache hits print a notice and skip the download). The push URL always comes from the environment — never from `Project.config`, since the registry endpoint is environment, not package identity.

Every new failure here reports `E108` with the cause in the message; no new diagnostic codes were added.

## Optimization levels, profiling, and linking

`rnx run`, `rnx build`, and `rnx test` accept `-O/--opt-level <0|1>` (default 1). Level 1 folds integer constants, prunes constant branches and unreachable blocks, compacts jump trampolines, and removes dead side-effect-free assignments; level 0 leaves the LIR intact.

Profiling and debug flags (`build`, `run`, `bench`): `--time-passes` prints a pass timing table to stderr, `--trace <path>` writes a Perfetto trace JSON file, and `--perf-map` writes `/tmp/perf-<pid>.map` for JIT backends. `build -g|--debug` emits DWARF line tables (`--release` strips them).

`rnx build` emits an LLVM object for the host target, links it against the embedded native runtime archive with mold first, then LLD, then the system linker, and marks the output executable. The program's `Main` return value becomes the process exit code. `--release` forces the LIR pipeline to level 1, runs the standard LLVM `default<O3>` pipeline over per-function `.text.<name>` (and `.rodata.<name>`) sections, and links with `-Wl,--gc-sections -s` to drop dead runtime code and the symbol table; rebuilding identical sources with identical flags yields byte-identical binaries.

Exit code is `0` on success, `1` on compile errors, thrown errors, or fatals. An uncaught throw prints as `Uncaught exception: <value>` on stderr, followed by a `Stack trace:` listing when the stop site is known; `print()` output goes to stdout on every backend (interpreter, Cranelift dev, LLVM release) through the shared `rnx_print_*` runtime symbols. `rnx run` prints only program output; the `Main` return value is not echoed. Performance warnings (`P201`, `P202`, `P203`) print to stderr after a successful build; performance notes (`P101`/`P102`) print only with `--show-perf-notes` (or `RNX_SHOW_PERF_NOTES=1`). `--deny-perf-lints` exits `1` when a denied warning is present without changing the built binary, and unknown codes exit `2`. `rnx build --fix` attempts the proven automatic rewrite for fixable `P201` ZeroInit cases (preview diff, `.bak` backup, re-parse plus rebuild plus output comparison, byte-for-byte rollback on any failure); anything else is refused with `no autofix available`.

## Editors and language servers

`rnx lsp` launches a Language Server Protocol server on stdio using zero-dependency in-tree JSON-RPC framing (`Content-Length` headers). It serves one open document set: `textDocument/didOpen` and `textDocument/didChange` re-run the typechecker and linter and push `textDocument/publishDiagnostics` (compiler errors at severity 1, lint warnings at severity 2, 0-based ranges, `E`/`L` codes under source `"rasmalai"`); `didClose` clears with an empty array. `initialize` advertises full-sync text documents (`textDocumentSync: 1`); `shutdown` plus `exit` ends the loop (exit 0 when shutdown was requested, 1 otherwise). Internal log lines go to stderr so stdout stays clean JSON-RPC.

The recommended way to wire an editor to this server is `rnx setup <vscode|zed|helix|neovim>`: it installs the VS Code extension when a local `.vsix` is present, prints dev-extension steps for Zed, and merges the shipped presets for Helix and Neovim idempotently. The full per-editor walkthrough lives in [Editor Setup](/guide/09-editor-setup).

Neovim: copy `editors/neovim/rasmalai.lua` into your config (it sets the `rasmalai` filetype for `*.rnx` and starts `rnx lsp` with `Project.config` root detection):

```lua
vim.filetype.add({ extension = { rnx = "rasmalai" } })
vim.api.nvim_create_autocmd("FileType", {
  pattern = "rasmalai",
  callback = function()
    vim.lsp.start({
      name = "rasmalai",
      cmd = { "rnx", "lsp" },
      root_dir = vim.fs.root(0, { "Project.config", ".git" }),
    })
  end,
})
```

Helix: merge `editors/helix/languages.toml` into yours (or point `HELIX_RUNTIME` at it):

```toml
[[language]]
name = "rasmalai"
scope = "source.rnx"
file-types = ["rnx"]
roots = ["Project.config", ".git"]
language-servers = ["rnx-lsp"]

[language-server.rnx-lsp]
command = "rnx"
args = ["lsp"]
```

`editors/vscode/` is the packaged extension (manifest, TextMate grammar covering `source.rnx` plus `*.rnx`, language configuration, stdio LSP client in `src/extension.ts`). It activates on `*.rnx`, starts `rnx lsp` (configurable via `rasmalai.serverPath`), and reads `rasmalai.trace.server` for protocol tracing. Helix users take the `languages.toml` stanza above; Zed users take `editors/zed/languages/rasmalai/config.toml` plus the `rnx-lsp` settings snippet commented at its bottom:

```json
{
  "lsp": { "rnx-lsp": { "binary": { "path": "rnx", "args": ["lsp"] } } },
  "languages": { "Rasmalai": { "language_servers": ["rnx-lsp"] } }
}
```

`editors/tree-sitter-rasmalai/` holds the Tree-sitter grammar (`grammar.js`, scope `source.rnx`, `*.rnx`) with `queries/highlights.scm`, `queries/locals.scm`, `queries/folds.scm`, `queries/outline.scm`, and `queries/indents.scm`. The generated `src/parser.c`, `src/grammar.json`, and `src/node-types.json` are not committed: CI rebuilds them from `grammar.js` with `tree-sitter generate`, checks them with `tree-sitter test`, and publishes them as build artifacts (see `.github/workflows/tree-sitter.yml`). Check query captures locally with `tree-sitter query`.

All preset files are validated by `crates/cli/tests/editor_configs_test.rs`, `grammar_test.rs` (TextMate JSON syntax, root keys, keyword coverage over real `.rnx` sources), and `tree_sitter_test.rs` (file presence, S-expression balance, keyword consistency against the compiler lexer).

## MCP server

`rnx mcp` serves the toolchain over the Model Context Protocol on stdio. Stdout carries protocol frames only; log lines go to stderr. Ten tools operate on self-contained snippets (`source` text or a `path` to one `.rnx` file): `check` (typecheck), `run` (interpreter, 30s cap, abandoning past the limit), `fmt`, `explain`, `rasmalai_lookup_symbol` (`@std/*` signature search), `version`, `inspect_package_capabilities` (package tier classification, required capability flags, and source-to-sink provenance chains), `eval_code` (persistent JIT session), `get_diagnostics` (structured JSON diagnostics), and `hot_reload` (reports an error without an attached `rnx dev --mcp` watcher). Four resources expose the language specification (`rasmalai://spec/grammar`, `rasmalai://spec/architecture`, `rasmalai://spec/manifest`, `rasmalai://stdlib/api.json`); two prompts bootstrap agents (`rasmalai-expert`, `convert-to-rasmalai`). Harness setup and protocol notes are specified in [AI Assistants](/guide/10-ai-assistants). The server is local-trust software: it executes received code with user privileges.

## Summary

- `Project.config` (`project`, `registry`, `dependencies`) with SemVer 2.0 requirements; `Project.deplock` plus `--locked` for repeatable builds; `workspace` for monorepos.
- `main` returns the exit code; `run`/`check`/`build`/`test` form the daily loop.
- One namespace per file; `@std/` served from the seeded cache (offline-first, `rnx download` to seed); `E203` guards `private`.
- `test fn` plus `assert`; `bench` plus `blackBox`; `/** */` plus `rnx doc`.
- `-O/--opt-level` plus `--time-passes`/`--trace`/`--perf-map`; mold then LLD then system linker; `--release` is byte-identical.
- `rnx lsp` plus Neovim/Helix/VS Code/Zed presets plus the Tree-sitter grammar; tests validate every preset file.
- Every diagnostic has a code, a span, and a fix hint; the registry is specified in [Diagnostics Directory](/manual/17-diagnostics-directory).
