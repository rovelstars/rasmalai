---
title: "Project and Toolchain"
description: "Project.config spec, modules, SemVer deps, testing, docs, and the rnx CLI."
section: "Toolchain and Diagnostics"
icon: "Terminal"
---

# Project and Toolchain

## Installation

One static binary, `rnx`, covers editing, running, testing, formatting, documenting, and shipping. Installation places `rnx` on `PATH`; `rnx --version` confirms the install and prints the build record (commit hash, build timestamp, install kind, target triple, backend versions) after the `rnx <version>` first line. Scaffolding creates the manifest and entry file:

Distributors building `rnx` for a managed install set two build-time variables. `RNX_INSTALLATION_MANAGED` carries the reason shown after `install: managed:`, for example how the package was installed and how to upgrade it; unset, the install reads `install: standalone`. `RNX_INSTALLATION_NO_TIMESTAMP` (any non-empty value) leaves the timestamp out, so the record reads `built: unknown`; it wins over `SOURCE_DATE_EPOCH` when both are set.

```sh
rnx init hello
cd hello
```

## Project.config

The manifest is an `.rnx` module exporting a default object. It declares package identity, entry points, and dependencies. `project` requires `name` and `version`; `engine` states the minimum toolchain requirement. An optional top-level `entries` object names the files: `main` defaults to `src/main.rnx`, `lib` names the library file, `cabi` names the C surface file, `docs` the guides folder, and `bins` maps tool names to files. The old `entry` and `edition` keys are rejected outright; there is no compatibility alias.

```rnx
export default {
    project: {
        name: "colony",
        version: "0.4.0",
        engine: ">=0.4.0",
        targets: ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
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
        zlib: { native: "z", system: true },
        "@std/prelude": { alias: "@uefi/prelude" },
        "@uefi/prelude": { path: "../uefi-prelude" }
    },
    i18n: {
        base: "en-US",
        dir: "locales",
        format: "arb",
        missing: "fallback",
        embed: "auto"
    }
}
```

The optional `i18n` key declares the base locale, catalog directory, source
format, missing policy, and embedding mode. Locales are discovered from the
directory and filenames; there is no supported-locales array and no
import/export configuration. Tasks fetch and place catalogs, and the build
parses ARB, PO, XLIFF, ICU JSON, or CSV directly. There is no `rnx i18n`
subcommand. See `requirements/26_I18N.md` for the contract.

`entries.cabi` names the file whose `export fn` declarations form the C surface for `rnx build --lib`. Plain `fn` declarations and non-function declarations are package-visible and need no C signature; `export fn` is the C-ABI surface and is validated as such. `rnx check` reads the cabi file as a second root even when nothing imports it. Without a cabi entry, `rnx build --lib` uses `entries.lib`, then `entries.main`, and validates every export reachable from that entry, which is the behavior projects had before the cabi entry existed. See [Hardware and FFI](/manual/13-hardware-and-ffi) for the signature rules.

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

### Target constraints

`project.targets` lists the built-in triples a package is meant to build for. The array is optional and each entry is validated when the manifest loads; an unknown triple fails as `E108` with the accepted list.

```rnx
export default {
    project: {
        name: "boot",
        version: "0.1.0",
        targets: ["x86_64-unknown-uefi"]
    }
}
```

Selection follows four rules:

- With `--target`, the triple must appear in `project.targets` when the list is not empty. A triple outside the list fails as `E108` and names both the request and the allowed list.
- Without `--target`, a single listed triple becomes the default.
- Without `--target`, more than one listed triple fails as `E108` and asks for `--target`.
- An absent or empty list selects the host target.

The list is a compatibility guard, not the code generator. The built-in descriptor table still decides the object format, libc, loader, CPU baseline, and capabilities. Run `rnx target list` to see the triples, tiers, object formats, capabilities, and pack status; add `--json` for machine-readable output or `--tier 1` to filter. The descriptors are `x86_64-unknown-linux-gnu`, `aarch64-unknown-linux-gnu`, `x86_64-unknown-linux-musl`, `aarch64-unknown-linux-musl`, `x86_64-pc-windows-gnu`, `aarch64-apple-darwin`, `x86_64-apple-darwin`, `x86_64-unknown-uefi`, and `wasm32-unknown-wasi`.

### Required capabilities

A package can state which target facilities it needs with a top-level `capabilities` array. The accepted names are `alloc`, `console`, `clock`, `random`, `fs`, `net`, `threads`, `process`, `log`, and `firmware`; an unknown name fails as `E108`.

```rnx
export default {
    project: { name: "@acme/telemetry", version: "0.1.0" },
    capabilities: ["fs", "net"]
}
```

The descriptor for the active target supplies the other half of the check. Importing a package that requires a capability the target does not provide fails as `E108` naming the package, the missing capability, and the triple. For example, a package declaring `capabilities: ["fs"]` cannot be imported when `project.targets` selects `x86_64-unknown-uefi`, because UEFI provides `alloc`, `console`, `clock`, and `firmware` but not `fs`. The check runs for path, git, registry, alias, and `@std` imports. `permissions` is a separate security ceiling and does not satisfy this check.

### Patching sources and wrappers

`patch` overrides package sources and native wrapper libraries for a project. It is the
manifest equivalent of `RNX_VENDOR_DIR`, `PKG_CONFIG_PATH`, and `LD_LIBRARY_PATH`, so a
checkout or an example can point at local or remote builds without exports.

```rnx
export default {
    project: { name: "demo", version: "0.1.0" },
    entries: { main: "src/main.rnx" },
    patch: {
        vendor: { path: "../../compiler/stdlib/src" },
        wrappers: { path: "../../.rnx-cache/wrappers" }
    }
}
```

`patch.vendor` is a package tree. It accepts `path`, `git`, or `oci`:

- `path` resolves relative to the project root and accepts either
  `<root>/@std/<name>` or `<root>/<name>`.
- `git` needs exactly one of `rev`, `tag`, or `branch`. The checkout is cached
  under `.rnx-cache/patches/git/`.
- `oci` pulls an OCI artifact whose layer is a tar archive of the package tree.
  It is cached under `.rnx-cache/patches/oci/`.

`patch.wrappers` is a directory of native wrapper artifacts: `lib<name>.so` (or
`.a`) plus `<name>.pc`. It accepts `path`, `git`, or `oci`. The git form expects a
repository that already contains the built artifacts, because a wrapper is a
platform-specific binary. The OCI form is the natural remote form for prebuilt
wrappers and is selected by host OS and architecture. The generated `.pc` files
use `${pcfiledir}`, so a copied or extracted wrapper directory remains
relocatable.

`patch.vendor` wins over `RNX_VENDOR_DIR`, the project cache, the global cache,
and the registry. `patch.wrappers` adds the directory to `pkg-config` discovery
and to the AOT binary's runpath; `rnx run` also adds it to the runtime native
library search path. `RNX_VENDOR_DIR`, `PKG_CONFIG_PATH`, and
`LD_LIBRARY_PATH` remain explicit overrides.

Prefer relative paths in a committed manifest. A machine-specific absolute path
belongs in a local copy of `Project.config`, not in the published package.

## SemVer comparators

Version requirements follow SemVer 2.0: `^1.2.0` permits compatible minor and patch changes; `~1.2.0` permits patch-level changes only; a bare version pins exactly; ranges combine comparators. A git dependency pins exactly one of `rev`, `tag`, or `branch` (`rev` holding a branch name is rejected). A `path` entry is a local checkout; git checkouts cache under `.rnx-cache/cache/git/`; a `.vendor/<pkg>/` directory wins over the network, for git and registry dependencies alike (see [Packages and Registries](/manual/18-packages-and-registries)). `RNX_VENDOR_DIR` adds one extra override lookup directory. The token never lives in the manifest: `token_env` names the environment variable, and `ca_cert` pins a corporate root CA.

`rnx add <name>` resolves the latest stable version from the configured registry and records it as `^<version>`; `rnx add <name>@<range>` records the given range verbatim (`*`, `latest`, `X.Y.Z`, `^`, `~`, `>=`). The lockfile pins the resolved version digest, never the range or a tag name. `rnx add <name> --path <dir>` keeps the local flow: the directory is recorded as a `path` entry.

An alias points one package name at another package: `{ alias: "@uefi/prelude" }`. The target may be declared in the same `dependencies` table, which wins, or pinned inline with `version: "0.2.2"`, which fetches it from the registry. Without either, the locked version is used and a missing pin fails as `E115`. `alias` cannot combine with another source key. Chains are allowed up to 16 links, and a cycle fails as `E108` with the full chain. See [Packages and Registries](/manual/18-packages-and-registries) for the worked example and the lock representation.

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

Programs start at the entry file's top-level statements, with no wrapper required. An explicit `main`/`Main` still works when you want a named entry point, but define exactly one: `main` and `Main` together in the entry file is an `E108` error. `rnx run` prefers lowercase `main` when resolving a default entry, and `rnx build` compiles it by renaming the symbol clear of the C-level `main`. A top-level `return <Int>` (or the entry function's return value) becomes the process exit code; `0` means success.

```rnx
println!("hello,", "world");
```

`Process.exit(code)` terminates immediately with a code, flushing output already written to stdout first; `Process.args()` reads CLI arguments as `Array<String>`. Every element is a `String`, index 0 is the program path, and user arguments follow a `--` separator from index 1. `"5" + 1` concatenates to `"51"`: convert explicitly before doing arithmetic, since `Int()` on a `String` is an `E108` error. A parameterless `Main` ignores extra arguments on every backend; the JIT backends pass declared `Main` parameters as integers only. `rnx build --release` compiles through LLVM `-O3` and links a stripped native executable.

## Modules and imports

Each file is its own namespace. Plain `fn` and non-function declarations are package-visible across files and packages; there is no `public` keyword in the final surface. `export` marks the C-ABI surface, not ordinary visibility. Only `private` items are file-locked. Importing one is an `E203` error.

```text
import { add } from "./math";
import { make_origin as origin } from "./shapes";
import * from "./shapes";
import math from "@std/math";
import engine, { Config } from "./engine";
import "./setup";
```

A default import binds the module namespace; `import * as ns` is the explicit spelling of the same binding; `import "./setup"` loads a file for declarations alone. Relative imports probe `<path>.rnx`, `<path>/mod.rnx`, `<path>/index.rnx` in order, and the error lists every path tried. Specifiers with a `:` are rejected as `E108` (`std:time` becomes `@std/time`). There is no `::` in the language; paths, variants, and generics use `.` and `<>`.

`@std/` imports resolve from the standard library compiled into `rnx` itself, with nothing to install and no versions to pin. Twenty-two modules ship with the compiler: `prelude`, `foundation`, `async`, `simd`, `math`, `collections`, `fs`, `bytes`, `time`, `random`, `sync`, `env`, `process`, `os`, `testing`, `web`, `json`, `net`, `io`, `log`, `uefi`, and `wasi`. `@std/net` provides the TCP, TLS, and DNS primitives the `net:*` capability domain refers to. `@std/io` owns the terminal streams and the `term:*` capability domain: `write`/`writeError` pretty-printing, `writeRaw` for exact bytes, `read`/`readLine` line input, `isTTY`/`width`/`height`/`colorProfile` queries, `setRawMode`, and TTY-guarded `clear`. Every file also sees `@std/prelude` without importing it; local declarations and imports always win, and anything found in neither scope is `E303`. Each `@std/*` module ships reference docs at `/docs/@std/<module>/overview` (overview, getting-started, API reference), any published `@scope/<name>` package gets the same pages at `/docs/@scope/<name>/overview`, and unscoped packages get them at `/docs/<name>/overview`.

```rnx
import { Vec2 } from "@std/math";
import { Map } from "@std/collections";

let m = new Map<String, Int>();
m.set("ore", 7);
let v = new Vec2(3.0, 4.0);
println!(m.get("ore"), v.length());
```

Bare and scoped specifiers name registry or path dependencies from the nearest `Project.config`: `"pkg"` loads that package's entry, `"pkg/sub"` loads `src/sub.rnx` inside it, and `"@scope/pkg"` checks the matching scoped registry first. Transitive dependencies resolve against each package's own manifest; diamond imports parse once, deduplicated by canonical path.

## Lockfiles and workspaces

`rnx lock` traverses the transitive graph, checksums every package directory (SHA-256 over sorted paths and bytes), and writes a sorted, deterministic `Project.deplock`. From then on, `rnx run --locked` and `rnx build --locked` re-verify every checksum before executing. A `Project.config` with a `workspace` object (a `members` list supporting `/*` expansion over sibling manifests) marks a monorepo root whose members resolve to each other by name; `rnx lock` writes one consolidated root lockfile.

## Permissions and capability audits

Every package carries a statically deduced capability surface: what files, sockets, processes, environment variables, and `unsafe` operations its code can reach. The surface is transitive: each module is charged its own sinks plus the summaries of everything it imports, through `@std/*` wrappers and wrapper libraries alike, resolved through imports (bare, renamed, and chained calls attribute identically), so a `[permissions]` ceiling must cover the whole transitive closure and a `Project.deplock` entry records the per-package summary. Capabilities are strings in `domain:action:scope` form:

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

When the directory holds a `Project.deplock`, the audit also fetches the registry-signed advisory feed (`GET /api/v0/advisories`), verifies the operator signature, and matches the locked entries (versions and `artifactDigest` pins) against it. A `critical` finding fails the audit with a security hold; a `warning` warns exactly like a yanked version; an `info` finding is a plain note. A feed whose signature does not verify fails the audit closed; a feed that cannot be reached only warns and the audit passes on the capability report alone. `RNX_ADVISORY_KEY` pins the expected feed key id or public key for registries whose key is known out of band. The audit never rewrites the lockfile: re-pinning an affected dependency stays an explicit update step.

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

Any deduced capability outside `allowed`, counting the package's own code plus every dependency, fails the build as `S102`, on `rnx check` and `rnx build` alike, with or without a `Project.deplock`. Bare `rnx run` executes without enforcing the ceiling; `rnx run --locked` enforces it. `Project.deplock` records the approved ledger: each entry stores its `tier` and `capabilities`, written by `rnx lock` and re-verified by `--locked` builds. Hand-editing the ledger does not grant anything; the next `rnx lock` overwrites it from the scan, and the next `--locked` build fails on the difference (`S101`). Adding a dependency whose surface reaches the ambient or hazard tiers requires explicit approval through `rnx add --accept-caps <list>` or `--accept-all-caps`.

## Testing and benchmarking

For repeated local runs, build `rnx` with `cargo build --release -p cli` and use
`target/release/rnx`. A debug compiler spends most of its time in the
single-threaded front end: a stdlib test suite that takes about six seconds with
the release binary can take over two minutes in debug. The staging and CI
scripts pick the release binary automatically when it exists.

`App.setSurfaceSocket(path)` or `RNX_SURFACE_SOCKET` makes `App.show` send frames
to a surface socket instead of the Wayland window. `App.runHost` commits the
display list, frame packet, damage, and window id, then waits for the ack.

The surface socket seam is also part of `host_cabi`: `host_surface_socket_open`,
`host_surface_socket_commit`, `host_surface_socket_id`, and `host_surface_socket_close` speak a
small Unix-socket protocol carrying the display list, optional frame packet,
damage rectangles, and window id. The compositor itself is a separate project;
`App.show` still uses the Wayland window.

`entries.lib` is the legacy alias: it keeps the graph-wide surface and warns
`W205` for every emitted symbol. With neither `cabi` nor `lib`, `rnx build
--lib` emits the main entry's exports and warns `W206` once.

`entries.cabi` names the C surface file. `rnx check` loads it as a second
root, so its declarations are type-checked even when no C artifact is produced.
The surface is exactly the `export fn` declarations physically in that file: a
surface with no `export fn`, a re-export, or a non-C-representable signature is
`E108`. A manifest that names a missing file is also `E108`.

A `test fn` takes no parameters and returns nothing. The runner discovers every one in `src/**/*.rnx` and `tests/**/*.rnx`, times each, and reports one line apiece. `assert` takes a `Bool` condition and a `String` message; on false it prints the message and marks that test failed without aborting it. `test fn` blocks are stripped from normal `run`/`build` pipelines. Tests run on the Cranelift JIT by default; pass `--backend interpreter` or `--backend llvm` to exercise the interpreter or AOT codegen. Filter by substring, pass `--exact` for full names. Output captures per test; `--nocapture` streams live.

```rnx
test fn addsUp() {
    assert(1 + 1 == 2, "math");
}

test fn startsEmpty() {
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

A `/** */` block documents the item that follows it: first paragraph of description, then one tag per line: `@param <name>`, `@returns`, `@throws`, `@example`, `@see`. `//!` lines document the module itself. Plain `//` remarks never attach to items. `pub` items without `/** */` docs warn as `L004` under `rnx lint`.

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

println!(quote(2.0));
```

`rnx doc` collects documented items into a static site under `target/doc` (`pub` only; `--all` adds internals; `--open` opens a browser; `--json` writes `api.json`). Every `@example` in the standard library executes as a documentation test and fails the build when broken.

## CLI command reference

| Command | Purpose | Key flags |
|---|---|---|
| `rnx check` | rapid lexer/parser/module/typecheck, no codegen | `[paths...]`, `-p`, `--json` |
| `rnx dev` | watch sources, hot-swap function bodies without restart | `[path]`, `--entry F`, `--no-rerun`, `--mcp` |
| `rnx repl` | interactive JIT shell | none |
| `rnx run` | execute program (Cranelift JIT by default) | `--entry F`, `--backend <interpreter\|cranelift\|llvm>`, `-p`, `--context <dir>`, `--locked`, `-O`, `-- <args>` |
| `rnx build` | link native binary into `.rnx-cache/build/{dev,release}/<name>` | `--release`, `--lib`, `--emit-obj`, `--target`, `--entry F`, `-p`, `--context <dir>`, `--locked`, `-O`, `-g/--debug`, `--deny-perf-lints[=codes]`, `--show-perf-notes`, `--fix[--force]`, `--fix-keep-backup` |
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
| `rnx task` | run a local task or an exported task from a direct dependency (`bindgen` replays the driver table) | `[<ref>]` (lists local tasks when omitted), `--all`, `--explain <ref>`, `--allow <pkg#task>`, `--force`, `--watch`, `-p <package>` |
| `rnx completions` | shell completion scripts | `bash`, `zsh`, `fish`, `powershell`, `elvish` |
| `rnx pack` | deterministic ustar plus SHA-256 | `-p`, `--out-dir`, `--gzip/-z` |
| `rnx unpack` | verify and extract archive | `--out-dir <dir>` |
| `rnx target list` | show built-in target triples, tiers, object formats, capabilities, and pack status | `--json`, `--tier <1\|2\|3>` |
| `rnx sysroot build` | seal a target pack from a source sysroot and a runtime archive | `<triple>`, `--from <dir>`, `--runtime <archive>`, `--out <dir>`, `--builtins <archive>`, `--force` |
| `rnx sysroot publish` | push a sealed pack to an OCI registry | `[<dir>]`, `--repo <bare>`, `--tag <tag>`, `--version <version>`, `--push-url <url>` |
| `rnx sysroot add` | install a target pack from OCI or from a local directory | `<triple>[@<version>]`, `--local <dir>`, `--digest <sha256>` |
| `rnx sysroot list/path/verify` | list installed packs, print the current directory, or rehash a pack | `[--json]`, `<triple>` |
| `rnx sysroot remove/prune` | remove one triple's packs or every non-current digest | `<triple>`, `--keep <n>` |
| `rnx init` | scaffold a new project | `[name]` |
| `rnx setup` | configure editor LSP and highlighting | `<vscode\|zed\|helix\|neovim>` |
| `rnx lock/fetch/vendor` | lockfiles, git-dependency cache, dep precompilation, and local source staging | `[packages]`, `--dir <path>` |
| `rnx install` | resolve, download, and precompile every dependency (lock then fetch) | `-p <package>` |
| `rnx x` | download and run a CLI tool from a package | `<package>[@version]`, `[bin]`, `--yes`, `-- <args>` |
| `rnx download` | seed the registry cache with libraries (`@std/*` when no names are given) | `[packages]`, `--local`, `--registry <url>` |
| `rnx doctor` | check the stdlib cache pin and contents, plus dep product status | `--repair-std`, `--registry <url>` |

Global flags: `--no-color` strips ANSI escapes, `-v` enables pipeline logging, `-q` suppresses banners, `--offline` disables automatic target pack fetch, and `--sysroot <dir>` selects a custom root. `--prefer-system` resolves every native dependency from the host library and `--prefer-bundled` from bundled OCI blobs; the two conflict, and per-dependency `native` pins override either.

`rnx download` with no package names resolves every `@std/<module>` at `*` through the registry in a single round-trip and pins the exact versions (plus the seeding date) to a pin file next to the global cache (`RNX_CACHE_HOME` is honored). With names, it downloads `name` or `name@range` libraries (the same SemVer grammar as dependency requirements, with the engine-range check after resolve) into the global cache. With `--local`, it targets the project-local cache (`.rnx-cache/registry/`) instead, with the pin recorded at `.rnx-cache/std-pin.json`. Global downloads land in the content-addressed store (`store/` under the cache root: chunk blobs plus one verified tree per manifest digest; see [Packages and Registries](/manual/18-packages-and-registries)); `--local` keeps the older per-version project layout instead. The installer runs `rnx download` automatically and keeps going with a warning when offline. `rnx doctor` reports a missing pin file or missing packages; `rnx doctor --repair-std` re-runs the seeding and reports how many packages it repaired. `download` and `doctor` accept `--registry <url>` (or `RNX_REGISTRY`) to point at a registry other than the default, which is how a self-hosted registry seeds its own standard library. With an empty cache and no network, `@std/*` imports fail as `E108` naming the registry instead of falling back silently. There is no embedded fallback: the binary carries no stdlib bytes. To seed or hack on the standard library without a registry instead, run `rnx vendor @std/... --dir <DIR>` from a checkout (see below), then point `RNX_VENDOR_DIR` at the staged directory: `@std/*` imports resolve straight from that tree with zero network, the same local-source override that covers every registry package (see [Packages and Registries](/manual/18-packages-and-registries)). Distributors bundle a pre-seeded cache directory and set `RNX_CACHE_HOME` at it. The stdlib sources in this repository are a workspace of packages, one manifest per module, so `RNX_VENDOR_DIR=<repo>/compiler/stdlib/src` resolves every `@std/*` import from the checkout. Repository development needs no export: the root `.cargo/config.toml` sets `RNX_VENDOR_DIR` to `compiler/stdlib/src` as a relative path, so `cargo run`, `cargo test`, and `cargo nextest` resolve `@std/*` from the checkout. The value is not forced, so an explicit `RNX_VENDOR_DIR` in the environment wins. A `rnx` binary installed from the registry carries no checkout path and resolves `@std/*` from the registry cache; the checkout override applies only to repository development and its tests. With a vendor directory, `@std/<name>` accepts either `<dir>/@std/<name>` or `<dir>/<name>`. No second mechanism exists.

Build products key on what the selected stdlib tree contains. With no pin file, the digest hashes each resolved `@std` module's files, so editing a vendored source invalidates the products built from it instead of reusing a stale one. Build output inside a vendored module (`.rnx-cache/`, `target/`) stays out of that hash, so a warm tree does not re-key on its own artifacts.

`rnx install` runs `lock` and then `fetch`, so a fresh checkout gets a lockfile and a warm dependency cache in one step. `rnx run` and `rnx check` already download missing registry packages on demand, but they never write the lock.

`rnx x <package> [bin] [-- args]` resolves and downloads a package, picks a bin from `entries.bins` (or `entries.main` when no bin is declared), prints the package's declared capabilities, asks once, and runs the entry in your current directory. `--yes` approves in CI. It does not add the package to your project or write a lock.

`rnx fetch` downloads every dependency of the current scope and then precompiles the scope: it loads each member's entry module so the dependency resolver records its products. Each product is mirrored from the project cache into the global store at `products/<registry-host>/<package>/<version>/<toolchain>/` (git dependencies under `products/git/<name>/<rev>/<toolchain>/`), next to a `manifest` file holding the content hash, dep set, and toolchain. A product is reused only when the exact pinned version, the toolchain hash, and the content digest all match; floating ranges never populate, and a missing or corrupt entry falls back to the normal path. `rnx build` also keeps whole-program codegen units in a content-keyed global `objects/` pool, so rebuilding the same program with an empty project cache skips LLVM emission. `rnx doctor` lists each pinned dependency as `cached` or `absent` (floating ranges are reported as unpinned), and `rnx cache status` reports the content-addressed store usage alongside the other pools, and `rnx cache prune` reclaims the pools plus deletes the old `registry/` tree left by previous versions wholesale (everything in it re-derives via `rnx download`); the store itself is retained.

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

`version` constrains the source: the installed host library for `system: true`, the declaring package's own version for bundled sources. Ranges use the same grammar as SemVer requirements (`*`, `^`, `~`, `>=`, exact). `from` selects a blob by its OCI title from the package's own `artifacts` image and implies `system: false`; a `from` entry without a declared `artifacts` image is an `E108` error. `system: true` consumes the host library only after verifying the version: the check runs `pkg-config --modversion <lib>`, falling back to `ldconfig -p` for presence. A mismatch or a missing library fails the lock closed and names the mismatch, with no silent fallback. `path` vendors a local checkout instead and cannot combine with `system: true`, `version`, or `from`.

Every dependency accepts a `link` key with `static`, `dynamic`, or `auto` (default `auto`). Only native entries provide shared objects, so `dynamic` on a path, git, tarball, or registry entry is an `E108` error. For native entries `auto` resolves to `dynamic` when a shared object exists (a host library or a `from` title) and to `static` otherwise; an explicit `static` or `dynamic` is recorded as-is. The lockfile stores the effective choice per entry in a `link` field (`static` or `dynamic`), and changing it requires `rnx lock`. A verify against a lock with a different `link` fails naming the mismatch.

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

The `native` table is root-only; member and dependency tables are ignored. The `--prefer-system` and `--prefer-bundled` global flags set the default for every entry without an explicit pin (or `RNX_PREFER_NATIVE=system|bundled` when the flags are absent; the flags win). Explicit pins override both. A flag that flips a declared source keeps the declared `version` constraint and checks it against the new source. A host-oriented range that the flipped source cannot satisfy fails the lock with the mismatch named, and an explicit pin without `version` clears it. The lockfile records the effective choice per entry (`native:<lib>+system` with the installed version, `native:<lib>#<title>` with the owning image digest, plain `native:<lib>` for the automatic role-plus-platform match), so locked builds resolve offline and identically. Re-lock after changing a pin or a flag; a locked verify with different preferences fails as a source mismatch, which is the point.

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
        icons: {
            file: "tools/icons.rnx",
            export: true,
            description: "Render the app icon set",
            runs_in: "project",
            inputs: ["assets/icons"],
            outputs: ["build/icons/**"]
        },
        build: {
            steps: [{ shell: target.os == "windows" ? "rnx build --target x86_64-pc-windows-msvc" : "rnx build" }],
            matrix: ["x86_64-unknown-linux-gnu", "aarch64-apple-darwin"]
        },
        release: {
            tasks: ["icons", "build"],
            strategy: "parallel",
            on_failure: "continue"
        },
        dist: ["icons", "build"]
    }
}
```

Each task holds exactly one body. `{ file: "tools/icons.rnx" }` names a package-relative rnx file with `fn Main(): Int`, which the runner starts as a program. `{ steps: [{ shell: "..." }] }` runs external commands, and a step may call another task with `{ task: "<ref>" }` in place of `shell`. `{ tasks: ["icons", "build"] }` is a group, and a bare array is the short form of one. Declaring two bodies fails with `E108`. A string in a group or a `{ task: ... }` step is always a task reference, never a command, so shell is written out as an object every time.

Every body accepts `export`, `runs_in`, `description`, `inputs`, and `outputs`. `runs_in` is `"package"` (the default, the defining package's directory) or `"project"` (the invoking project's directory). `export: true` requires a `description`. `matrix` belongs to a `steps` task and runs every step once per target triple, each leg seeing its own `target`. `strategy: "serial" | "parallel"` and `on_failure: "stop" | "continue"` belong to a group. `on-failure` still parses as the older spelling; write `on_failure`. The manifest sandbox (`const`, spreads, ternaries, `switch` on `target`, member access, no interpolation) is the whole expression language, so work that needs more belongs in a file body.

A reference with no `#` is local. `<package>#<task>` names an exported task in a direct dependency or a workspace member:

```sh
rnx task build                # local task
rnx task @std/app#bundle      # exported task in a direct dependency
rnx task core#build           # workspace member
```

`#icons` and `self#icons` are errors, since the bare name already means the current package. Task names are lowercase letters, digits, and hyphens, so a hyphenated key needs quotes. Task cycles and unknown names fail closed with `E108`.

`rnx task` lists the local tasks, and `rnx task --all` adds the imported exported tasks with the package each one came from. `rnx task <ref>` runs one, `rnx task --explain <ref>` prints the resolved graph without running it, and `rnx task -p <package> <ref>` works in a workspace member's context.

A task from a workspace member is part of the same project and runs without approval. A task from an external dependency is resolved down to its leaves, a file or a shell script, before anything runs. The graph is printed, and the approval is stored in `.rnx/tasks.consent` keyed by content hash, so a changed file or script prompts again. `rnx task --allow <pkg#task>` approves a graph in CI. Local tasks need no approval.

A task that declares `inputs` or `outputs` is skipped when the recorded input hash still matches and every output exists. `rnx task --force` runs it anyway. A task with neither field always runs. `rnx task --watch` reruns when the selected package's files change, after the first run finishes.

`rnx run` and `rnx build` accept `--context <dir>`, which reads the manifest, lock, and capability ceiling from that project while running the given file. `rnx task` passes it for file bodies, so a task from a dependency runs with the invoking project's grants instead of the library's.

No command other than `rnx task` runs a task, and there are no lifecycle hooks. Exported tasks are the ones that travel: publishing keeps them in the stored `Project.config` with their `description`, `runs_in`, `inputs`, and `outputs`, and drops private tasks along with `bindgen`. An exported task's text is public, so name an environment variable and never paste the token.

Publishing a version that declares `artifacts` can push the image itself. Stage per-target blobs under `.rnx-cache/publish/<target>/<role>/<title>` (roles `runtime`, `wasm`, `bindgen`, `media`; the author's matrix tasks produce these), point at an endpoint, and publish:

```sh
export RNX_OCI_PUSH_URL=https://ghcr.io
rnx publish
```

The pipeline replays `rnx task bindgen`, assembles one manifest per target plus the bindings manifests into a single index, cross-checks every staged file against the index before asserting, pushes blobs, manifests, and the index, then captures the index digest into the frame's `artifactDigest` automatically, with no hand-copied hashes. A `--artifact` flag that disagrees with the pushed digest fails closed. Without the push URL, publish keeps the old behavior: `--artifact sha256:<hex>` asserts a prebuilt image, fetched by tag, verified by digest, and cross-checked against the local project cache (`.rnx-cache/artifacts/`, seeded from the global `~/.cache/rnx/artifacts/` store; cache hits print a notice and skip the download). The push URL always comes from the environment, never from `Project.config`, since the registry endpoint is environment, not package identity.

Every new failure here reports `E108` with the cause in the message; no new diagnostic codes were added.

## Optimization levels, profiling, and linking

`rnx run`, `rnx build`, and `rnx test` accept `-O/--opt-level <0|1>` (default 1). Level 1 folds integer constants, prunes constant branches and unreachable blocks, compacts jump trampolines, and removes dead side-effect-free assignments; level 0 leaves the LIR intact.

Profiling and debug flags (`build`, `run`, `bench`): `--time-passes` prints a pass timing table to stderr, `--trace <path>` writes a Perfetto trace JSON file, and `--perf-map` writes `/tmp/perf-<pid>.map` for JIT backends. `build -g|--debug` emits DWARF line tables (`--release` strips them).

`rnx build` emits an LLVM object for the host target, links it against the embedded native runtime archive with mold first, then LLD, then the system linker, and marks the output executable. The program's `Main` return value becomes the process exit code. `--release` forces the LIR pipeline to level 1, runs the standard LLVM `default<O3>` pipeline over per-function `.text.<name>` (and `.rodata.<name>`) sections, and links with `-Wl,--gc-sections -s` to drop dead runtime code and the symbol table; rebuilding identical sources with identical flags yields byte-identical binaries. Mach-O targets use `__TEXT,__text` and `__TEXT,__const` section names and the in-house linker described under macOS below.

Exit code is `0` on success, `1` on compile errors, thrown errors, or fatals. An uncaught throw prints as `Uncaught exception: <value>` on stderr, followed by a `Stack trace:` listing when the stop site is known; `println!()` output goes to stdout on every backend (interpreter, Cranelift dev, LLVM release) through the shared `rnx_print_*` runtime symbols. `rnx run` prints only program output; the `Main` return value is not echoed. Performance warnings (`P201`, `P202`, `P203`) print to stderr after a successful build; performance notes (`P101`/`P102`) print only with `--show-perf-notes` (or `RNX_SHOW_PERF_NOTES=1`). `--deny-perf-lints` exits `1` when a denied warning is present without changing the built binary, and unknown codes exit `2`. `rnx build --fix` attempts the proven automatic rewrite for fixable `P201` ZeroInit cases (preview diff, `.bak` backup, re-parse plus rebuild plus output comparison, byte-for-byte rollback on any failure); anything else is refused with `no autofix available`.

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

## Sysroots and target packs

A sysroot carries the link inputs and the target runtime for one triple. The host toolchain uses the host directly. A cross target needs a target pack: a content-addressed directory with a version 2 `sysroot.json` and the per-target library tree.

```sh
$SYSROOT/
  sysroot.json
  lib/rustlib/<triple>/lib/    # crt1.o, crti.o, crtn.o, crtbegin/end, libc.a, libm.a, ...
```

Packs carry no C headers and no `@std` source. Link map entries in `sysroot.json` are normalized relative names resolved against the declared search directories. Absolute paths, `..`, symlinks, device files, and FIFOs are rejected. `integrity.digest` is SHA-256 over the canonical manifest and every file except the manifest itself, in sorted path order, so the identity never depends on mtimes or other file metadata. A corrupt byte, a missing file, or a manifest that disagrees with the target descriptor fails `E124` and names the pack directory and the failing path.

Installed packs live in one store:

```sh
$RNX_CACHE_HOME/sysroots/     # ~/.cache/rnx/sysroots by default
  <triple>/
    current                   # text file holding one digest
    sha256:<hex>/
      sysroot.json
      lib/rustlib/<triple>/lib/...
  tmp/                        # download and validation staging
```

Install and inspect with `rnx sysroot`:

```sh
rnx sysroot add aarch64-unknown-linux-musl
rnx sysroot add aarch64-unknown-linux-musl@0.2.2
rnx sysroot add --local ./my-pack
rnx sysroot list
rnx sysroot list --json
rnx sysroot path aarch64-unknown-linux-musl
rnx sysroot verify aarch64-unknown-linux-musl
rnx sysroot remove aarch64-unknown-linux-musl
rnx sysroot prune --keep 2
```

`add` fetches `ghcr.io/rovelstars/rnx-sysroot:rnx-<toolchain-version>-<triple>` and verifies the index digest, the manifest digest, the blob digest, the `sysroot` kind and role annotations, the target annotation, and the pack `integrity.digest` before installing. A version suffix uses that version in the tag. `--local <dir>` installs a hand-built pack after the same validation, and `--digest sha256:<hex>` asserts the expected digest. `RNX_SYSROOT_OCI` overrides the repository for a mirror. `list` shows the triple, digest, tier, and current marker; `prune` never removes the digest named by `current`.

### Building a target pack

`rnx sysroot build` turns a source sysroot and a runtime archive into a sealed pack. It discovers the CRT objects, C library archives, compiler-rt builtins, and runtime archive from any number of `--from` roots, writes the version 2 manifest from the target descriptor, copies only the selected files into `lib/rustlib/<triple>/lib`, computes `integrity.digest` with the same code that verifies an installed pack, and reloads the result before it reports success.

```sh
rnx sysroot build x86_64-unknown-linux-musl \
  --from /usr/lib/x86_64-linux-musl \
  --from /usr/lib/gcc/x86_64-linux-gnu/13 \
  --runtime compiler/runtime/bundle/target/x86_64-unknown-linux-musl/release/libruntime_native.a \
  --builtins /usr/lib/llvm-22/lib/clang/22/lib/linux/libclang_rt.builtins-x86_64.a \
  --out /tmp/musl-pack
rnx sysroot add --local /tmp/musl-pack
rnx sysroot verify x86_64-unknown-linux-musl
```

The source root can be a distro sysroot (`/usr`), a cross toolchain root, a GCC directory that carries `crtbeginT.o` and `crtend.o`, or an already staged `lib/rustlib/<triple>/lib` tree. The builder looks in the conventional `lib`, `lib64`, `usr/lib`, `usr/lib/<triple>`, `usr/lib/<arch>-linux-<env>`, MinGW, and Clang directories under each root. A runtime-only target such as UEFI or WASI needs only `--runtime`. `--builtins`, `--loader`, `--kernel-min`, `--glibc-baseline`, `--prelude`, and `--abi` override the descriptor when a toolchain needs it. The output directory is replaced only with `--force`, and two different inputs that want the same pack filename fail instead of overwriting each other.

A pack is published with the same digest it was sealed with:

```sh
RNX_OCI_PUSH_USERNAME="$GITHUB_ACTOR" \
RNX_OCI_PUSH_TOKEN="$GITHUB_TOKEN" \
rnx sysroot publish /tmp/musl-pack
```

The default repository is `ghcr.io/rovelstars/rnx-sysroot` and the default tag is `rnx-<toolchain-version>-<triple>`. `--repo`, `--tag`, `--version`, and `--push-url` override them. The pushed index and manifest carry `org.opencontainers.image.source=https://github.com/rovelstars/rasmalai`, so GHCR links the package to this repository. `rnx sysroot add` uses the same bearer-token exchange, which means a private package works when `RNX_OCI_PUSH_USERNAME` and `RNX_OCI_PUSH_TOKEN` are set.

`rnx lock` pins the installed pack for every triple in `project.targets` and for an explicit `--target`. The lockfile is version 3 and carries a sorted `sysroots` map:

```rnx
export default {
    version: 3,
    packages: [...],
    sysroots: {
        "aarch64-unknown-linux-musl": "sha256:..."
    }
}
```

Version 1 and 2 locks still load; a version 2 lock has no `sysroots` entry. `rnx build --locked` and `rnx lock` verification rehash the installed pack for each pinned triple and fail `E124` on a missing pack or a digest mismatch. A cross build for a triple with no pin is allowed and uses the current pack; the next `rnx lock` records it.

`--offline` and a non-empty, non-zero `CI` disable automatic pack fetch. A missing pack then fails `E124` with a hint to run `rnx sysroot add <triple>`. `--sysroot <dir>` and `RNX_SYSROOT` still override the store for local development; the custom root is validated but is not pinned. `rnx target list` reports a `PACK` column, either `installed sha256:<prefix>` or `missing`, next to each built-in descriptor.

The host target needs no pack. Linux targets select the C library by triple, never by host probing:

| Triple | C library | Link mode |
|---|---|---|
| `x86_64-unknown-linux-gnu` | glibc | dynamic dev, static release (as before) |
| `x86_64-unknown-linux-musl` | musl | always fully static |
| `aarch64-unknown-linux-gnu` | glibc | `--emit-obj` only (linking runs on the host) |

Cross builds select and validate the pack for the requested triple. The pack's link map supplies the link inputs: `crt` and `crt_end` for each mode (`static`, `dynpie`, `staticpie`), `libs` for `c`, `m`, `pthread`, `dl`, `rt`, and `gcc_s` (a null value omits the library), and `builtins` for the compiler-rt archive when the pack ships one. Bare names resolve against `link.search`, then the pack's `lib/rustlib/<triple>/lib`, then the host search path. A declared file that is missing fails `E124` naming the pack and the file, and a non-empty link map that omits the requested CRT mode fails the same way.

The runtime archive also comes from the pack. `rnx` reads `runtime.file`, verifies the recorded `runtime.sha256`, and requires `runtime.abi` to equal the compiler's runtime ABI (1). A mismatch is `E124` naming the target and both ABI values. The embedded archive is the host fast path. A host build ignores an installed host pack unless the lock pins that triple in `sysroots` or `--sysroot`/`RNX_SYSROOT` names a v2 pack. The pack digest, the runtime archive sha256, and the runtime ABI all feed the build key, so replacing a selected pack rebuilds the artifact.

The musl target needs musl libraries packed at `lib/rustlib/x86_64-unknown-linux-musl/lib` (or reachable via `RNX_LIB_DIRS`); without them the link fails closed naming the expected directory instead of silently falling back to glibc. The baked loader path follows the triple: `/lib64/ld-linux-x86-64.so.2` for glibc, `/lib/ld-musl-x86_64.so.1` for musl. The `cc`-driver fallback path refuses musl targets outright. Musl links fully static through the native backend. `rnx target list` is the authoritative list of accepted triples.

AArch64 Linux is Tier 2. The in-house linker handles AArch64 ELF objects, static and static-pie executables, and dynamic glibc executables: absolute and pc-relative data, `adrp`/`add`/`ldr` page addressing, branches, GOT loads, PLT calls, local-exec and initial-exec TLS, and `R_AARCH64_RELATIVE` static-pie relocations. `rnx build --target aarch64-unknown-linux-gnu` links through the native backend, and `--emit-obj` emits an AArch64 object without one. A full linked executable additionally needs the target pack's AArch64 `libruntime_native.a`, which the pack build job supplies; without a real archive the link fails at the archive input, and the runtime ABI check still applies. The linker tests assemble AArch64 fixtures with `aarch64-linux-gnu-as`, link them with the in-house backend, and run the result with `qemu-aarch64-static`:

```sh
aarch64-linux-gnu-as -o main.o main.s
qemu-aarch64-static -L /usr/aarch64-linux-gnu ./main
```

The backend rejects general-dynamic TLS instead of emitting a wrong sequence, and reports misaligned or out-of-range relocations as linker errors. Two links of the same inputs are byte-identical.

`x86_64-pc-windows-gnu` is Tier 2. The in-house PE backend reads x86_64 COFF objects and archives, including COMDAT and weak externals, resolves imports from MinGW-style import libraries, and writes PE32+ images: console subsystem by default, `.text`/`.rdata`/`.data`/`.bss`/`.pdata`/`.xdata` sections, an import directory with ILT/IAT, and a `.reloc` directory for absolute pointers. `rnx build --target x86_64-pc-windows-gnu` uses the pack's `crt2.o`, import libraries, builtins, and runtime archive; `--emit-obj` still emits a COFF object without linking. A missing pack is `E124`.

Windows execution is not validated on Linux: this environment has no MinGW toolchain and no Wine. The CI job must provide the Wine gate before Tier 1. Structural tests assemble fixtures with `llvm-mc`, build import libraries with `llvm-dlltool`, inspect output with `llvm-readobj`, and compare machine, subsystem, entry, section permissions, imports, and relocation kinds against `lld-link`.

`aarch64-apple-darwin` and `x86_64-apple-darwin` are Tier 2 and console-only. `rnx build --target aarch64-apple-darwin` or `x86_64-apple-darwin` links through the in-house Mach-O backend: 64-bit Mach-O objects, BSD archives, and `.tbd` stubs from the pack become an `MH_EXECUTE` image with `__PAGEZERO`, `__TEXT`, `__DATA_CONST` when present, `__DATA`, and `__LINKEDIT`, followed by `LC_MAIN`, `/usr/lib/dyld`, `LC_LOAD_DYLIB`, `LC_BUILD_VERSION`, `LC_DYLD_INFO_ONLY`, and an ad-hoc `LC_CODE_SIGNATURE`. The linker covers the arm64 and x86_64 relocations the compiler and `libSystem` stubs use, including GOT loads, subtractor pairs, addends, and thread-local descriptors. `--emit-obj` still emits a Mach-O object.

The pack supplies the startup objects, `libSystem.tbd`, optional builtins, and `libruntime_native.a`. A missing pack is `E124`, and a declared pack file that cannot be read is also `E124`. Frameworks, Objective-C, universal or fat binaries, and chained fixups are out of scope.

macOS execution is not tested on this Linux host: there is no macOS runner and no Apple SDK. Structural tests assemble fixtures with `llvm-mc`, inspect the image with `llvm-objdump`, check the ad-hoc signature blob and thread-local descriptor offsets, verify two links are byte-identical, and compare structure against `ld64.lld`. The CI job must run the linked executables on macOS before the target leaves Tier 2.

`x86_64-unknown-uefi` is Tier 3 and experimental. `rnx build --target x86_64-unknown-uefi` links through the same PE writer with subsystem 10 (`IMAGE_SUBSYSTEM_EFI_APPLICATION`) and entry `efi_main`, so the output is a firmware application, not a console program. The image imports nothing, carries no `.idata` section, and needs no C runtime. The pack ships only `libruntime_native.a` and `sysroot.json`; `--emit-obj` still emits a COFF object and needs no pack. A missing pack is `E124` with a hint to run `rnx sysroot add x86_64-unknown-uefi`.

The target runtime lives in `compiler/runtime/uefi`, a detached `no_std` crate built for the triple:

```sh
cargo build --manifest-path compiler/runtime/uefi/Cargo.toml \
  --target x86_64-unknown-uefi --release
```

It supplies `efi_main`, a Boot Services allocator, strings, arrays, `Any` boxing, integer and float rendering, a ConOut writer, a COM1 writer for captures, and a panic handler. `@std/uefi` is the prelude and console package for the triple: `println!` renders its arguments and writes one line to ConOut and COM1, and the package re-exports the foundation catalog and `Date`.

The boot gate runs the image under qemu with OVMF:

```sh
qemu-system-x86_64 -machine q35 -m 256 -bios /usr/share/edk2/x64/OVMF.4m.fd \
  -drive format=raw,file=fat:rw:esp -display none -serial stdio -no-reboot
```

Place the PE at `EFI/BOOT/BOOTX64.EFI` inside the `esp` directory. Stage a pack from a locally built archive with `rnx sysroot build x86_64-unknown-uefi --runtime <archive> --out <dir>`, then install it with `rnx sysroot add --local <dir>`. `compiler/cli/tests/uefi_boot.rs` builds the runtime archive, stages and installs a pack, checks the PE headers with `llvm-readobj`, and asserts the serial capture contains the expected line. The test skips when `qemu-system-x86_64`, OVMF, or the Rust target is absent.

Limits: maps, sets, JSON, files, sockets, threads, and the event loop are not implemented by the UEFI runtime, and a program that uses them fails at link with the missing symbol. `println!` renders `Int`, `Bool`, `Float`, `String`, and `null`; arrays and objects render as placeholders. The entry must be named `Main`.

`wasm32-unknown-wasi` is Tier 3 and experimental. `rnx build --target wasm32-unknown-wasi` links a WASI command module with the in-house wasm linker, and `--emit-obj` emits the relocatable wasm object without a pack, like the COFF and Mach-O paths.

The link model is wasm-specific. The linker reads LLVM relocatable objects: type, import, function, table, memory, global, export, element, code, data, and data-count sections, plus the `linking` custom section with its symbol table and `reloc.CODE` / `reloc.DATA` relocation sections. Archive members are pulled on demand, so the runtime archive contributes only the members the program references. Symbols resolve across objects, function, global, and table indices are renumbered, data segments are laid out, and the relocations the compiler and `rustc` emit are applied: `FUNCTION_INDEX_LEB`, `TABLE_INDEX_SLEB`, `TABLE_INDEX_I32`, `MEMORY_ADDR_LEB`, `MEMORY_ADDR_SLEB`, `MEMORY_ADDR_I32`, `MEMORY_ADDR_REL_SLEB`, `TYPE_INDEX_LEB`, `GLOBAL_INDEX_LEB`, `GLOBAL_INDEX_I32`, `FUNCTION_OFFSET_I32`, `SECTION_OFFSET_I32`, `FUNCTION_INDEX_I32`, and `TABLE_NUMBER_LEB`. An unknown relocation type fails as `E108` instead of producing a module that traps later.

The layout is fixed and deterministic: static data starts at 1024, one mebibyte of stack follows it, `__stack_pointer` starts at the stack top, `__heap_base` points at the first byte the runtime allocator owns, and the initial memory covers both plus one page. Function table slot 0 stays empty so a null function reference stays distinguishable; every function whose address is taken gets the next slot. Two links of the same inputs are byte-identical. Sections appear in one fixed order: type, import, function, table, memory, global, export, element, data count, code, data. The module exports `_start` and `memory` and imports only `wasi_snapshot_preview1` `fd_write` and `proc_exit`.

The pack ships only `libruntime_native.a` and `sysroot.json`: no headers, no `@std` source, and no wasi-libc. The runtime archive is a detached `no_std` crate built for `wasm32-unknown-unknown`:

```sh
cargo build --manifest-path compiler/runtime/wasi/Cargo.toml \
  --target wasm32-unknown-unknown --release
```

The target is `wasm32-unknown-unknown` with explicit preview 1 imports, not `wasm32-wasip2`, because the wasip2 standard library emits preview 2 component imports that a preview 1 host cannot satisfy. The crate supplies `_start`, a linear-memory allocator with a first-fit free list, strings, arrays, `Any` boxing, integer and float rendering, `fd_write` console output on descriptors 1 and 2, `proc_exit`, and a panic handler. `@std/wasi` is the prelude for the triple: it re-exports the foundation catalog and `Date` and provides `println!`, `wasi_println!`, `eprintln!`, `printLine`, `printText`, `printErrorLine`, and `printErrorText`. `@std/io`, `@std/log`, `@std/fs`, `@std/process`, `@std/net`, and `@std/sync` are not in that prelude.

Stage a pack from a locally built archive, install it, and build:

```sh
rnx sysroot build wasm32-unknown-wasi \
  --runtime compiler/runtime/wasi/target/wasm32-unknown-unknown/release/libruntime_native.a \
  --out /tmp/wasi-pack
rnx sysroot add --local /tmp/wasi-pack wasm32-unknown-wasi
rnx build --target wasm32-unknown-wasi --release
```

Node 26 no longer runs a `.wasm` file as a WASI program, so both hosts go through the same ten-line preview 1 runner:

```js
import { WASI } from "node:wasi";
import { readFile } from "node:fs/promises";
const wasi = new WASI({ version: "preview1", args: [], env: {} });
const bytes = await readFile(process.argv[2]);
const module = await WebAssembly.compile(bytes);
const instance = await WebAssembly.instantiate(module, wasi.getImportObject());
process.exit(wasi.start(instance));
```

```sh
node runner.mjs build/release/<name>.wasm
deno run --allow-read runner.mjs build/release/<name>.wasm
```

The program below prints one line and exits 0. A `return 3` in `Main` becomes `proc_exit(3)`.

```rnx
fn Main(): Int {
    println!("hello wasi");
    return 0;
}
```

`compiler/cli/tests/wasi_run.rs` builds the runtime archive, stages and installs a pack, links the fixture, and asserts the exact line on stdout and the exit status under Node and, when present, Deno. It also checks that `--emit-obj` works with no pack installed and that the object carries the `linking` and `reloc.CODE` sections. A differential test links the same object and archive with `wasm-ld`, compares import and export names and the observed output, and requires the in-house module to cover at least the reference memory size. The tests skip with a printed reason when the Rust target or the hosts are absent.

Limits: the linker does not yet drop unreachable code, so a module carries every function and data segment of the archive members it pulls and its initial memory is larger than a `wasm-ld` link of the same program. It drops custom sections from the inputs, so a `-g` build carries no DWARF, no name section, and no producers record. `--lib` is refused for this target. `println!` renders `Int`, `Bool`, `Float`, `String`, and `null`; arrays, maps, and class instances render as placeholders. `@std/fs`, `@std/process`, `@std/net`, and `@std/sync` are not ported, though the descriptor declares the `fs`, `log`, and `random` capabilities for a future runtime. The browser playground keeps its own engine path and does not use this linker.

A custom sysroot supplies link inputs only. It does not replace `@std` sources; those resolve through the package graph, the project and global caches, and the registry in that order. A barebones platform ships its own prelude as an ordinary package and selects it through one of the prelude rules below.

The compiler always imports `@std/prelude`. The specifier resolves in three levels, first match wins:

1. `Project.config.dependencies["@std/prelude"]`, including an `alias:` chain.
2. The active target descriptor's `prelude` field.
3. The stock package `@std/prelude`.

A project that wants a local prelude aliases the specifier and declares the package:

```rnx
export default {
    project: { name: "app", version: "0.1.0" },
    dependencies: {
        "@std/prelude": { alias: "@myorg/prelude" },
        "@myorg/prelude": { path: "../myorg-prelude" },
    },
}
```

The selected prelude must declare the foundation names the compiler expects: `Int`, `Float`, `FastFloat`, `Bool`, `String`, `Char`, `Array`, `Map`, `Set`, `Date`, `Error`, `Any`, `Void`, `Null`, `GenRef`, `Result`, and `Iterator`. `Null` is a language builtin and needs no declaration. A missing name fails `E108` with the package and the missing name. Start from the stock prelude and extend it, then keep the package in the lock file like any other dependency.

The stock prelude is a facade. It re-exports `Date` from `@std/time`, `Promise`, `PromiseWithResolvers`, and `PromiseResult` from `@std/async`, and `RnxHost` from `@std/env`. It also declares `IO`, `Thread`, and `ThreadPool` as catalog stubs whose methods the compiler lowers. Those three names and `Promise` are prelude-driven: a prelude that omits `Thread` makes `Thread` unavailable, and a prelude that omits `Promise` makes `async`/`await` fail with `E108` naming the prelude and `Promise`. `assert` and `typeOf` stay compiler intrinsics; the print family is macros re-exported from `@std/io`, and the log family is macros re-exported from `@std/log`.

The foundation declarations live in `@std/foundation`, not in the stock prelude. The package declares `Int`, `Float`, `FastFloat`, `Bool`, `String`, `Char`, `Array`, `Map`, `Set`, `Error`, `Any`, `Void`, `GenRef`, `Result`, `Iterator`, `Iterable`, and `ArrayIter`, plus the Array and String extension methods. It imports only the native `runtime "bytes"` and `runtime "core"` shims, never `@std/io` or `@std/log`. The stock prelude re-exports those names and adds `Date`, the promise types, `RnxHost`, the print and log macros, and the `IO`, `Thread`, and `ThreadPool` stubs.

A target prelude can therefore provide the catalog without the host console and logging surface. The repository ships `@uefi/prelude` as the worked example: it re-exports the foundation names, `Date`, and the `@std/uefi` console surface. Select it with an alias:

```rnx
export default {
    project: {
        name: "uefi-probe",
        version: "0.1.0",
        targets: ["x86_64-unknown-uefi"]
    },
    dependencies: {
        "@std/prelude": { alias: "@uefi/prelude" },
        "@uefi/prelude": { path: "../../compiler/stdlib/uefi_prelude" },
    },
}
```

A program under that prelude can use `Int`, `Result`, `Array`, `String`, `Date`, and `println!`. `println!` writes through the UEFI console, not a host stdout descriptor. `IO`, `Thread`, `ThreadPool`, the log macros, and every `@std` package that needs an operating system are not in scope. Because `@std/uefi` declares the `console` and `firmware` capabilities, the same project fails as `E108` if it is built for a host target.

```rnx
fn Main(): Int {
    println!("hello from uefi");
    return 0;
}
```

Built with `rnx build --target x86_64-unknown-uefi --release`, that program becomes a PE32+ image that boots under OVMF. See [Sysroots and target packs](#sysroots-and-target-packs) above for the pack, the runtime archive, and the qemu command.

All sysroot lookups sort directory entries and hash bytes, never mtimes.

All preset files are validated by `crates/cli/tests/editor_configs_test.rs`, `grammar_test.rs` (TextMate JSON syntax, root keys, keyword coverage over real `.rnx` sources), and `tree_sitter_test.rs` (file presence, S-expression balance, keyword consistency against the compiler lexer).

## Environment variables

Flags win when a variable and a flag set the same option. The tables list the
variables the toolchain and the app host read. Internal and test-only names are
listed at the end and are not part of the public contract.

### Toolchain, registry, and linking

| Variable | Default | Effect |
| --- | --- | --- |
| `RNX_CACHE_HOME` | `$XDG_CACHE_HOME/rnx` or `~/.cache/rnx` (`%LOCALAPPDATA%\rnx` on Windows) | Global cache root for packages and downloads. |
| `RNX_REGISTRY` | built-in registry | Registry endpoint for resolve, download, audit, and publish. `--registry` wins. |
| `RNX_VENDOR_DIR` | unset | Extra local source tree consulted before caches and the registry. |
| `RNX_ADVISORY_KEY` | unset | Expected advisory-feed key id or public key. |
| `RNX_SYSROOT` | host probe | Custom sysroot for the linker. `--sysroot` wins. The root is validated but not pinned. |
| `RNX_SYSROOT_OCI` | `ghcr.io/rovelstars/rnx-sysroot` | OCI repository used by `rnx sysroot add` and `rnx sysroot publish`. |
| `RNX_OCI_PUSH_TOKEN` | unset | Registry token for `rnx sysroot publish`, and for pulling a private pack. |
| `RNX_OCI_PUSH_USERNAME` | unset | Registry username for the bearer-token exchange. GitHub Actions uses `${{ github.actor }}`. |
| `RNX_OCI_PUSH_URL` | repository scheme and host | Push base URL. `https://ghcr.io` for GHCR. |
| `RNX_OFFLINE` | unset | Any non-empty, non-zero value disables automatic target pack fetch. `--offline` sets it. |
| `CI` | unset | A non-empty, non-zero value also disables automatic target pack fetch. |
| `RNX_LIB_DIRS` | unset | Colon-separated extra link library directories. |
| `RNX_NATIVE_LINK` | automatic on x86_64 Linux | `1` forces the in-house linker, `0` forces the C driver. |
| `RNX_PREFER_NATIVE` | manifest pins | `system` or `bundled` default for native dependencies. The `--prefer-*` flags win. |
| `RNX_CC` | `CC`, then `cc` | C compiler driver for linking and sysroot probes. |
| `RNX_TARGET_ENV` | host ABI | Target environment (`gnu`, `msvc`, and so on) used when computing the host triple. |
| `RNX_JOBS` | available parallelism | Parallel worker count. |
| `RNX_MEMORY_CAP` | min(50% RAM, 8 GB) | Build memory cap. Accepts bytes or `KB`/`MB`/`GB` suffixes. |
| `RNX_SHOW_PERF_NOTES` | off | `1` shows P101/P102 performance notes. |
| `RNX_DENY_PERF_LINTS` | unset | Comma list of P-codes to promote to errors. `--deny-perf-lints` wins. |
| `RNX_LINK_TIME` | off | Any non-empty value prints linker phase timings to stderr. |
| `RNX_INSTALLATION_MANAGED` | unset | Build-time reason shown after `install: managed:`. Distributors only. |
| `RNX_INSTALLATION_NO_TIMESTAMP` | unset | Build-time: leave the timestamp at `built: unknown`. |
| `RNX_OCI_PUSH_URL` | unset | OCI registry endpoint used by `rnx publish`. |
| `RNX_OCI_PUSH_TOKEN` | unset | Token for the OCI endpoint. |

`token_env` in `Project.config` names the variable that carries a registry
token. `RNX_TOKEN` is the convention, not a reserved name.

### App and UI host

| Variable | Default | Effect |
| --- | --- | --- |
| `RNX_TRACE` | off | Host trace lines for present damage and timing, input, scroll, accessibility, clipboard, and gamepads. |
| `RNX_INSPECT` | off | `1` enables the inspector overlay. |
| `RNX_UI_SCRIPT` | unset | Semicolon-separated headless UI script. Steps include `press`, `focus`, `set`, `scroll`, `drag`, `wait`, `shot`, and `assert`. |
| `RNX_SURFACE_SOCKET` | unset | Send frames to a surface socket instead of opening a Wayland window. `runixOS` provides the other end. |
| `RNX_HEADLESS` | unset | Keep the media picker from opening the XDG portal. |
| `RNX_PICK_FILE` | unset | Picker result used in headless runs. |
| `RNX_CAMERA` | real device | `stub` generates moving frames without V4L2. |
| `RNX_AUDIO` | real sink | `fast` drains the audio queue without pacing; `null` discards it. |
| `RNX_TEST_VIDEO` | unset | Video file used by media tests. |

### MCP server

| Variable | Default | Effect |
| --- | --- | --- |
| `RNX_MCP_RUN_TIMEOUT_SECS` | 30 | Timeout for one `run` call. |
| `RNX_MCP_JOB_TIMEOUT_SECS` | 180 | Timeout for a background MCP job. |

### Installer

| Variable | Default | Effect |
| --- | --- | --- |
| `RNX_PREFIX` | `$HOME/.local` (XDG and Windows equivalents apply) | Install prefix. `--prefix` wins. |
| `RNX_VERSION` | `latest` | Release version to install. A positional version wins. |

### Internal and test-only

These names are not public. They exist for the repository's own scripts and
tests and can change or disappear.

- `RNX_STAGE_DIR`, `RNX_BACKEND`: CI wrapper scripts.
- `RNX_DEP_CACHE`, `RNX_STDLIB_API`: repository and MCP internals.
- `RNX_TASK`, `RNX_TARGET`, `RNX_TASK_DIR`, `RNX_TASK_ORIGIN`,
  `RNX_TASK_PACKAGE_DIR`, `RNX_ALLOW_TASKS`: set by `rnx task` for its child
  steps.
- `RNX_CPU_TIER`, `RNX_TASK_STALL_MS`, `RNX_TEST_TLS_CA_DER`, `RNX_BENCH_SCALE`,
  `RNX_TOUR`, `RNX_DEBUG_LIR`: test, benchmark, or regression sentinels.

## MCP server

`rnx mcp` serves the toolchain over the Model Context Protocol on stdio. Stdout carries protocol frames only; log lines go to stderr. Eighteen tools: `check` (project-aware typecheck, structured JSON by default, `pretty` opt-in), `run` (interpreter, 30s cap, structured output; a timed-out worker is abandoned and blocks further runs until it exits), `fmt` (plus read-only `check`/`diff` modes), `explain`, `rasmalai_lookup_symbol` (`@std/*` signature search, cheaper than reading the catalog), `version`, `inspect_package_capabilities` (package tier classification, capability flags, source-to-sink provenance chains, plus lockfile drift, licenses, and fail-soft advisory verdicts), `eval_code` (persistent JIT session with `:type` and `reset`), `get_diagnostics` (structured JSON diagnostics), `hot_reload` (reports an error without an attached `rnx dev --mcp` watcher), `build` (LLVM object kept in memory, release opt-in), `test` (project test runner, capped output), `lint` (read-only), `doc` (`--json` API docs), `doctor` (read-only cache health), `cache_status` (read-only pool sizes), and `package_resolve` (offline cache verdicts), and `ui_drive` (run a semicolon UI script against an app, assert controls, drag/fling/scroll through real controls, return a `shot:` PNG as an inline image block, and compare it against a `golden` baseline with a tolerance and diff ratio). Four spec resources expose the language (`rasmalai://spec/grammar`, `rasmalai://spec/architecture`, `rasmalai://spec/manifest`, `rasmalai://spec/packages`), plus per-module `rasmalai://stdlib/<module>.json` catalogs and the full `rasmalai://stdlib/api.json` for opt-in reads; two prompts bootstrap agents (`rasmalai-expert`, `convert-to-rasmalai`). Harness setup and protocol notes are specified in [AI Assistants](/guide/10-ai-assistants). The server is local-trust software: it executes received code with user privileges.

## Summary

- `Project.config` (`project`, `registry`, `dependencies`) with SemVer 2.0 requirements; `Project.deplock` plus `--locked` for repeatable builds; `workspace` for monorepos.
- `main` returns the exit code; `run`/`check`/`build`/`test` form the daily loop.
- One namespace per file; `@std/` served from the seeded cache (offline-first, `rnx download` to seed); `E203` guards `private`.
- `test fn` plus `assert`; `bench` plus `blackBox`; `/** */` plus `rnx doc`.
- `-O/--opt-level` plus `--time-passes`/`--trace`/`--perf-map`; mold then LLD then system linker; `--release` is byte-identical.
- `rnx lsp` plus Neovim/Helix/VS Code/Zed presets plus the Tree-sitter grammar; tests validate every preset file.
- Every diagnostic has a code, a span, and a fix hint; the registry is specified in [Diagnostics Directory](/manual/17-diagnostics-directory).
