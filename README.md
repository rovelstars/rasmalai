<p align="center">
  <img src="website/static/favicon.svg" alt="Rasmalai logo" width="120" />
</p>

<h1 align="center">Rasmalai (<code>rnx</code>)</h1>

<p align="center">
  <a href="https://github.com/rovelstars/rasmalai/actions/workflows/ci.yml"><img src="https://github.com/rovelstars/rasmalai/actions/workflows/ci.yml/badge.svg" alt="CI status" /></a>
  <a href="https://github.com/rovelstars/rasmalai/stargazers"><img src="https://img.shields.io/github/stars/rovelstars/rasmalai" alt="GitHub stars" /></a>
  <a href="http://discord.rovelstars.com/server"><img src="https://img.shields.io/badge/chat-discord-5865F2?logo=discord&logoColor=white" alt="Discord server" /></a>
</p>

<p align="center">
  <a href="https://rasmalai.rovelstars.com/manual/01-lexicon-and-structure">Documentation</a>
  ·
  <a href="https://github.com/rovelstars/rasmalai/issues">Issues</a>
  ·
  <a href="https://rasmalai.rovelstars.com">Website</a>
  ·
  <a href="https://rasmalai.rovelstars.com/packages">View packages</a>
</p>

## Stability: 0.x means breaking changes

Rasmalai is pre-1.0. While the major version is 0, expect breaking
changes in any release with no deprecation period and no backward
compatibility: CLI flags, manifest keys, registry API paths, file
layouts, and language semantics can all change. Pin your toolchain
(`rnx --version`, lockfiles, pinned container images) and read release
notes before upgrading. Do not build production systems on 0.x without
accepting that upgrades may require code and workflow changes. The
public registry is a development environment: its stored packages may
be wiped and republished from scratch at any time with no migration.

## What is Rasmalai?

Rasmalai is a high-performance compiled programming language. It pairs
TypeScript-like developer ergonomics with native C-ABI interop, a
multi-pass LIR optimizer, a Cranelift JIT for fast dev cycles, and an
LLVM AOT backend for release builds. One toolchain covers scripting,
native binaries, and a browser playground.

## Why does it exist?

Mainstream systems languages force a choice: ergonomics or control.
Rasmalai refuses it — memory safety through ARC with explicit cyclic
edges instead of a garbage collector, Python-grade readability with
native execution speed, and a standard library that treats files,
sockets, threads, and SIMD as first-class citizens rather than FFI
afterthoughts.

## Install

Linux and macOS (installs into `~/.local/bin` by default, honors
`$XDG_BIN_HOME` when set):

```bash
curl -fsSL https://rasmalai.rovelstars.com/install.sh | sh
```

Windows (PowerShell, installs into `%LOCALAPPDATA%\rnx` by default):

```powershell
irm https://rasmalai.rovelstars.com/install.ps1 | iex
```

Pin a version, or install elsewhere:

```bash
curl -fsSL https://rasmalai.rovelstars.com/install.sh | sh -s -- v0.1.0
curl -fsSL https://rasmalai.rovelstars.com/install.sh | sh -s -- --prefix /usr/local
```

```powershell
& ([scriptblock]::Create((iwr https://rasmalai.rovelstars.com/install.ps1).Content)) -Version v0.1.0
```

Build from source (needs Rust stable and LLVM 22):

```bash
cargo build --release
```

The binary lands at `./target/release/rnx`.

The installer also seeds the standard library into the global cache
(`~/.cache/rnx`, or `$RNX_CACHE_HOME` when set) by running
`rnx download` for you: the `rnx` binary carries no library sources, so
every `@std/*` import resolves from that cache. An offline install warns
instead of failing — run `rnx download` once online, or
`rnx doctor --repair-std` to check and repair the cache at any time.

Upgrade by re-running the same command you installed with — running it
again with no version argument always moves you to the latest release:

```bash
curl -fsSL https://rasmalai.rovelstars.com/install.sh | sh
```

```powershell
irm https://rasmalai.rovelstars.com/install.ps1 | iex
```

## Platform support

The compiler, CLI, and language runtime install on Linux, macOS, and Windows.
Native UI apps are Linux/Wayland only. `@std/app`, the window host, the
accessibility bridge (AccessKit), audio, video, camera, and the file picker are
implemented for Linux. Other platforms keep the language, compiler, and
headless image/scene paths, but there is no window host or accessibility
bridge yet. `rnx build` links native Linux executables; non-Linux link targets
return `UnsupportedTarget`.

The accessibility packet is platform neutral. Windows UIA and macOS
NSAccessibility bridges wait on native window hosts; the packet already carries
the roles, names, values, bounds, actions, state, relations, and announcements
they need. The approved order is: complete the sysroot implementation, then the
in-house linker for the other operating systems and CPU architectures, then the
Windows/macOS UI hosts and their accessibility and media backends. The full
matrix and porting contract are in `requirements_UI/72-platform-support.md`.

## Use

Run a script (JIT, dev mode):

```bash
rnx run path/to/main.rnx
```

Compile a native binary ahead of time (LLVM, release mode):

```bash
rnx build path/to/main.rnx
```

Program arguments go after `--`:

```bash
rnx run path/to/main.rnx -- arg1 arg2
```

Run the test suite with the pinned runner:

```bash
cargo nextest run --release --profile ci --workspace
```

Nextest runs every test binary in parallel, so this finishes in seconds instead
of waiting for one suite at a time. Release is the supported test profile:
debug builds are much slower, and a few runtime paths only show their real
behavior when optimized. `rnx test` uses `target/release/rnx` when it exists,
and `compiler/stdlib/src/app/tools/ci-wrappers.sh` selects the release binary
automatically.

Try it without installing anything: the
[playground](https://rasmalai.rovelstars.com/playground) runs the
language in your browser.

## Roadmap

- **Accounts and package management server integration**, so the
  registry can serve publish, yank, ownership, and access-control
  requests instead of org-token-only seeding.
- **Native cross-platform UI**, so Rasmalai programs can ship graphical
  interfaces on every supported OS from one codebase.

## How to contribute

Issues and pull requests are welcome at
[rovelstars/rasmalai](https://github.com/rovelstars/rasmalai/issues).
Open an issue describing the problem or proposal first for anything
beyond a trivial fix, then send a PR against `main`. Every compiler
change needs tests; every user-facing change needs docs.

## Layout

- `compiler/` — the language itself: lexer, parser, semantic analysis,
  LIR lowering and optimization, interpreter, Cranelift JIT, LLVM AOT,
  native C-ABI runtime, and the `rnx` CLI.
- `tools/rnx-bindgen/` — C-header binding generator written in pure
  `.rnx`. Turns system headers (zlib, sqlite3) into Rasmalai modules.
- `benches/` — performance and scaling benchmarks.
- `editors/` — Helix, Neovim, VSCode, and Zed extensions.
- `website/` — SvelteKit documentation and interactive playground.

## License

Dual-licensed, at your option: MIT OR Apache-2.0. See `LICENSE`.
