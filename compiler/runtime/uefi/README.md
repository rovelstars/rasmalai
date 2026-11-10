# UEFI runtime archive

The target runtime for `x86_64-unknown-uefi`. It is a `no_std` staticlib
that the compiler links into a PE32+ firmware application, so the UEFI
target needs no host C toolchain and no libc.

## Build

```sh
cargo build \
  --manifest-path compiler/runtime/uefi/Cargo.toml \
  --target x86_64-unknown-uefi \
  --release
```

Output: `compiler/runtime/uefi/target/x86_64-unknown-uefi/release/libruntime_native.a`.

The crate declares its own `[workspace]`, so the root workspace ignores it.
`rustup target add x86_64-unknown-uefi` installs the target. There are no
external crates: `core` and `compiler-builtins` are the whole dependency
tree, and `panic = "abort"` keeps unwinding tables out of the image.

## What the archive defines

`efi_main` and the helpers the compiler emits:

- allocation: `rnx_alloc`, `rnx_free`, `rnx_retain`, `rnx_release_str`
- arrays: `rnx_array_new`, `rnx_array_len`, `rnx_array_push`, `rnx_array_get`,
  `rnx_release_array`
- `Any`: `rnx_any_box`, `rnx_any_unbox`, `rnx_any_unbox_heap`,
  `rnx_any_retain`, `rnx_any_release`, `rnx_any_release_box`
- heap bookkeeping: `rnx_heap_track`, `rnx_note_array_kind`
- strings: `rnx_string_concat`
- entry glue: `rnx_init`, `rnx_set_assert_strict`, `rnx_assert`,
  `rnx_panic_str`, `rnx_fatal_span`, `rnx_error_take`, `rnx_report_fatal`,
  `rnx_report_uncaught`, `rnx_genref_invalidate`
- console: `rnx_uefi_println`, `rnx_uefi_write`
- memory primitives this target's libc would have provided: `memcpy`,
  `memmove`, `memset`, `memcmp`, `bcmp`

`efi_main` stores the system table, calls `Main`, and returns
`EFI_SUCCESS` for zero or `EFI_ABORTED` for any other exit code.

## Using it

Stage the archive into a target pack with the dev helper, which copies the
archive, writes a version 2 manifest, seals `integrity.digest` with
`sysroot::pack::digest`, and re-validates the result:

```sh
cargo run -p sysroot --example uefi_pack -- \
  --runtime compiler/runtime/uefi/target/x86_64-unknown-uefi/release/libruntime_native.a \
  --out /tmp/uefi-pack
rnx sysroot add --local /tmp/uefi-pack
```

The pack contains `sysroot.json` and
`lib/rustlib/x86_64-unknown-uefi/lib/libruntime_native.a`. Its link map has
an empty `crt`, `crt_end`, `libs`, and `builtins`, and `runtime` names
`libruntime_native.a`.

```sh
rnx build src/main.rnx --target x86_64-unknown-uefi --release
qemu-system-x86_64 -machine q35 -m 256 -bios /usr/share/edk2/x64/OVMF.4m.fd \
  -drive format=raw,file=fat:rw:esp -display none -serial stdio -no-reboot
```

Place the PE at `EFI/BOOT/BOOTX64.EFI` inside the `esp` directory.

## Limits

- Maps, sets, JSON, files, sockets, threads, timers, and the event loop are
  not implemented. A program that calls one fails at link with the missing
  symbol rather than running with wrong behavior.
- `println!` renders `Int`, `Bool`, `Float`, `String`, and `null`. Arrays,
  objects, and enums render as `<array>`, `<object>`, and `<value>`.
- The heap registry and the `Any` box set are fixed-capacity tables of 4096
  entries. Overflow degrades rendering, never memory safety.
- `Main` is the entry name. `efi_main` calls `Main`, so `fn main()` does not
  link.
