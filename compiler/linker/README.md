# Linker

Turns compiled output into host programs.

## API (`src/lib.rs`)

- `host_triple()` — the triple binaries are built for.
- `link_executable()` — statically link an object with the runtime archive
  into a standalone executable.
- `link_executable_dynamic()` — link against an on-disk
  `libruntime_native.so` for fast dev-mode builds.
- `find_shared_runtime_dir()` — locates the shared runtime
  (`$RNX_RUNTIME_DIR`, beside the `rnx` binary, then fallbacks).
- `bundle_static_library()` — produce a static library instead of an
  executable (`rnx build --static`).

The linker shells out to the system toolchain; it contains no codegen of
its own.
