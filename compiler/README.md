# Compiler

The Rasmalai compiler toolchain. Source of truth is this directory: the
repository root `Cargo.toml` defines the workspace, and every member lives
directly under `compiler/`. There is exactly one `target/` directory, at the
repository root.

## Pipeline

```
frontend → lir → { runtime (interpreter), cranelift (JIT), llvm (AOT) }
                     ↕ native C-ABI runtime          ↕ linker (host binaries)
                                   cli (the `rnx` binary)
```

`diagnostics` and `stdlib` are shared leaves: every stage reports through
the same error codes, and every program links the same standard library.

## Crates

- `frontend/` — lexer, parser, semantic analysis, desugaring, module graph,
  project manifests and dependency lockfiles, capabilities and sandboxing,
  LSP helpers. Turns `.rnx` source into a checked AST.
- `lir/` — lowers the AST to LIR (a typed intermediate representation),
  runs the optimizer passes, and verifies the result. The contract all
  three backends compile against.
- `runtime/` — tree-walking interpreter plus the native C-ABI runtime: the
  `rnx_*` functions for strings, collections, threads, files, sockets, and
  math, with async I/O on `mio` and TLS on `rustls`.
- `cranelift/` — JIT backend for `rnx run` and `rnx dev`. Compiles LIR to
  machine code in memory and resolves `rnx_*` symbols dynamically.
- `llvm/` — AOT backend for `rnx build`. Emits native binaries through
  LLVM.
- `linker/` — assembles host executables: static and dynamic linking
  against `libruntime_native`, host triple detection.
- `cli/` — the `rnx` binary: two dozen subcommands (`run`, `build`,
  `check`, `test`, `fmt`, `doc`, `pack`, …) and 117 integration test
  suites that pin end-to-end behavior.
- `diagnostics/` — the error-code registry (`Code`), spans, rendered
  diagnostics, terminal themes. Every user-facing error flows through here.
- `stdlib/` — the standard library, 16 `.rnx` modules embedded into the
  compiler with `include_str!`, shipped with every program.
- `wasm_playground/` — `cdylib` build of the frontend and interpreter for
  the browser playground: tokenize, check, and run behind three functions.

Each crate has its own README with details. Run the suite from the
repository root with `cargo test --workspace`.
