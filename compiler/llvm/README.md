# LLVM

AOT backend for `rnx build`. Lowers verified LIR to native executables
through LLVM 22 (`inkwell` 0.10), reusing the same `rnx_*` runtime symbols
as the interpreter and JIT.

## Files

- `codegen.rs` — instruction selection and object emission.
- `lib.rs` — public entry points used by `cli`.

`tests/jit.rs` exercises codegen against CLI fixtures. Backend-agnostic
behavior (operators, closures, ABI layout) is pinned by the `cli`
integration suites across all three backends instead of here.
