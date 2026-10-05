# rnx-bindgen (Phase 2)

Pure-Rasmalai C header binding generator. Ingests `clang -Xclang -ast-dump=json`,
extracts whitelisted `FunctionDecl` nodes, transparent `RecordDecl` structs,
and integer `#define` constants, maps C types to C-ABI primitives, and emits
a Rasmalai package (`src/lib.rnx` + `src/types.rnx` + `Project.config`).

## Layout

```
tools/rnx-bindgen/
  Project.config   # export default manifest, entries.main src/main.rnx
  src/main.rnx     # CLI arg parsing, orchestration
  src/clang.rnx    # clang subprocess via Process.run
  src/extract.rnx  # AST filter, struct/define extraction, C-to-RNX type map
  src/emit.rnx     # insertion sort, codegen, file emission
```

## Usage

```sh
rnx run --backend cranelift tools/rnx-bindgen/src/main.rnx -- \
  --header /usr/include/sqlite3.h --native-lib sqlite3 --pkg-name sqlite3 \
  --pkg-version 3.53.4 --prefix sqlite3_ --out packages/sqlite3 \
  --skip sqlite3_carray_bind --skip sqlite3_win32_set_directory \
  [--include <dir>]... [--clang-flags "<flags>"]
```

Repeatable filters: `--prefix <pfx>` (keep; empty means the legacy zlib
allowlist for functions and everything for structs/defines), `--skip <name>`
(exact function name to drop). Skip list exists because headers declare
functions the shipped `.so` does not export (extension-gated APIs such as
`sqlite3_carray_bind`, `sqlite3_snapshot_*`, win32-only shims); native
backends resolve foreign symbols eagerly, so one missing symbol fails the
whole package. Enumerate the gap with
`comm -23 <(generated fns) <(nm -D --defined-only libfoo.so | awk '{print $3}')`.

The tool writes `<out>/src/lib.rnx` (and `src/types.rnx` when structs or
constants exist) directly and prints `Project.config`
between `---BEGIN/END Project.config---` markers. Extract it with:

```sh
sed -n '/---BEGIN Project.config---/,/---END Project.config---/p' run.log \
  | sed '1d;$d' > packages/sqlite3/Project.config
```

## Extraction rules

* Functions: main-file `FunctionDecl` whose name passes the prefix filter
  and does not start with `_`. Variadic (`...`) functions are skipped: the
  foreign ABI cannot express varargs, and emitting a fixed-arity signature
  would be unsound.
* Structs: `RecordDecl` with `tagUsed == "struct"` and
  `completeDefinition == true`. Transparent only when every child is a
  `FieldDecl` with a scalar/pointer type (arrays, nested complete records,
  and unknown named types downgrade the struct to opaque
  `Pointer<Byte>`). Function-pointer fields map to `Pointer<Byte>`.
  Forward declarations never emit.
* Constants: `#define NAME <decimal-int>` lines scanned from the header
  text (clang AST has no macro nodes, so `SQLITE_OK` never appears as an
  `EnumDecl`). Hex, string, expression, and >18-digit values are skipped.
  Define matching is case-insensitive on the prefix with trailing
  digits/underscores trimmed, so `--prefix sqlite3_` keeps `SQLITE_*`.

## Ground-truth deviations from the original spec

Recorded here so the next phase does not re-litigate them:

1. File I/O is `@std/fs` (`fs` one-shots, `File` with `OpenMode` /
   `WriteMode`, `Path`); processes are `@std/process` (`Process.run` /
   `Process.spawn`, no free `spawn`, output is `ByteBuffer`).
2. `Project.config` is an `export default {...}` module, not a raw object
   literal.
3. Runtime file writes to any `Project.config` path abort with `S301`
   (protected path). Hence the stdout-marker split above; the config is
   written by the host shell, never by the tool.
4. `panic(...)` is not a language builtin. Failures print to stdout and
   return nonzero from `Main`.
5. `Array.sort` with a comparator does not exist. Ordering uses the local
   `asciiLess` (charCodeAt loop): the `<` operator on `String` misorders
   strings built from JSON values (`"" + any`), verified live. Do not
   switch the sort back to `<` without re-running
   `cli/tests/bindgen_tool.rs`.
6. `val`, `type`, `mut` are not keywords (`frontend/src/token.rs`). The
   sanitizer covers the real keyword set; extra prefixes are harmless.
7. Permissions take capability strings (`unsafe:ffi` for `from native`
   imports). `native:<lib>` is a deplock source tag, not a permission.
8. No SHA-256 API exists in stdlib, so no header digest is emitted.
9. Phase 1 coerces every indirection level to flat `Pointer<Byte>`
   (matches the 1-byte `Pointer<Byte>` lane). Struct returns and
   double-pointer out-params are out of scope until Phase 2.
10. String escapes: `{` must be written `\{` inside string literals;
    `}` needs no escape. `"` escapes as `\"`.

## Running it safely

`rnx run` on multi-megabyte clang JSON plus the Rust test suite can
saturate the host (parallel LLVM builds, expected-failure aborts that
each trigger systemd-coredump). Contain heavy runs:

```sh
ulimit -c 0   # expected-failure SIGABRTs must not file coredumps
systemd-run --user --scope -p MemoryMax=12G -p CPUQuota=300% -- \
  bash -c 'ulimit -c 0; cargo test -p cli -- --test-threads=2'
```

Prefer `--test-threads=2`, `nice -n 10`, and single-package test targets.
Full-workspace runs belong in CI or a memory-capped container, not on
the interactive host.
