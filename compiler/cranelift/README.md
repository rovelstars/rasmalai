# Cranelift

JIT backend for `rnx run` and `rnx dev`. Compiles verified LIR to machine
code in memory (Cranelift 0.135) and resolves `rnx_*` runtime symbols by
name at run time, so generated code calls straight into the native runtime
with no stub layer.

## Files

- `jit.rs` — code generation and in-memory execution.
- `lib.rs` — public entry points used by `cli` and the dev watcher.

`tests/jit.rs` pins JIT behavior against CLI fixtures; JIT-specific
semantics (dynamic symbol resolution, in-memory calling convention) live
here, shared lowering behavior is tested in `lir` and `cli`.
