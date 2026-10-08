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

## Native linker (`src/native.rs`, `RNX_NATIVE_LINK=1`)

System files (`crt1.o`, `crti.o`, `crtbeginT.o`, `crtend.o`, `crtn.o`,
`libc.a`, `libm.a`, `libgcc.a`, `libc.so.6`, `libm.so.6`) are found by
searching `$RNX_LIB_DIRS` (colon-separated) first, then
`/usr/lib/gcc/<triplet>/<ver>/`, `/usr/lib/x86_64-linux-gnu`,
`/usr/lib64`, `/usr/lib`, `/lib64`, `/lib`. No compiler driver is run.
Static links always pull the `c` and `m` archives even when the program
has no explicit foreign libraries.

Known gap: native output is `ET_EXEC` (fixed base `0x400000`) in both
static and dynamic modes, while the `cc` reference emits PIE. Release
binaries therefore miss ASLR (RELRO and NX are present). Static PIE
needs `rcrt1.o`/`Scrt1.o` + `crtbeginS.o`/`crtendS.o` selection,
`ET_DYN` output at base 0, `R_X86_64_RELATIVE` entries for every baked
absolute address with a `PT_DYNAMIC` for `_dl_relocate_static_pie`,
working `IRELATIVE` handling in static links, and a 32-bit absolute
relocation audit, plus differential tests against `cc -static-pie`.
