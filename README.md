# Rasmalai (`rnx`)

Rasmalai is a high-performance compiled programming language. It pairs
TypeScript-like developer ergonomics with native C-ABI interop, a
multi-pass LIR optimizer, a Cranelift JIT for fast dev cycles, and an
LLVM AOT backend for release builds. One toolchain covers scripting,
native binaries, and a browser playground.

## Install

From a GitHub release (Linux and macOS):

```bash
sh install.sh
```

This installs `rnx` into `~/.local/bin`. Pin a version with
`sh install.sh v0.1.0`, or pick another prefix with
`sh install.sh --prefix /usr/local`.

## Quickstart

Build from source:

```bash
cargo build --release
```

Run a script (JIT, dev mode):

```bash
./target/release/rnx run path/to/main.rnx
```

Compile a native binary ahead of time (LLVM, release mode):

```bash
./target/release/rnx build path/to/main.rnx -o ./myapp
```

Program arguments go after `--`:

```bash
./target/release/rnx run path/to/main.rnx -- arg1 arg2
```

Run the test suite:

```bash
cargo test --workspace
```

## Layout

- `compiler/` — the language itself: lexer, parser, semantic analysis,
  LIR lowering and optimization, interpreter, Cranelift JIT, LLVM AOT,
  native C-ABI runtime, and the `rnx` CLI.
- `examples/` — reserved for language samples (currently empty).
- `tools/rnx-bindgen/` — C-header binding generator written in pure
  `.rnx`. Turns system headers (zlib, sqlite3) into Rasmalai modules.
- `benches/` — performance and scaling benchmarks.
- `editors/` — Helix, Neovim, VSCode, and Zed extensions.
- `website/` — SvelteKit documentation and interactive playground.

## License

MIT OR Apache-2.0. See `LICENSE`.
