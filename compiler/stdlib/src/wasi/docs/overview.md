---
title: "@std/wasi"
description: Prelude and console output for wasm32-unknown-wasi command modules.
section: Overview
order: 0
---

# @std/wasi

`@std/wasi` is the prelude and console surface for `wasm32-unknown-wasi`.
A WASI command module starts with no host process to inherit. It reaches the
outside world through `wasi_snapshot_preview1` imports, and the in-house
linker supplies those imports when it links the module.

```rnx
fn Main(): Int {
    println!("hello wasi");
    return 0;
}
```

`@std/wasi` is selected automatically as the prelude for the target, so
`println!`, `eprintln!`, and the foundation names are in scope with no import.
The package re-exports the foundation catalog (`Int`, `String`, `Array`,
`Map`, and the rest) and `Date` from `@std/time`.

## What is inside

The console surface is three call macros and four plain functions:

- `println!` writes one line to standard output.
- `wasi_println!` is the same macro under a distinct name.
- `eprintln!` writes one line to standard error.
- `printLine(text)` writes a `String` and a newline to standard output.
- `printText(text)` writes a `String` with no newline to standard output.
- `printErrorLine(text)` writes a `String` and a newline to standard error.
- `printErrorText(text)` writes a `String` with no newline to standard error.

Macro arguments are joined with a single space and the line ends with `\n`.
The bytes leave the module through `fd_write` on descriptor 1 for standard
output and descriptor 2 for standard error.

## Target details

`@std/wasi` declares the `alloc`, `console`, and `clock` capabilities, which
the WASI descriptor provides. The module exports `_start` and `memory` and
imports only `wasi_snapshot_preview1` `fd_write` and `proc_exit`. A `Main`
return value becomes the `proc_exit` status.

The target is Tier 3 and experimental. `@std/io`, `@std/log`, `@std/fs`,
`@std/process`, `@std/net`, and `@std/sync` are not part of this prelude.
`@std/process`, `@std/net`, and `@std/sync` also need capabilities the WASI
descriptor does not declare, so importing them fails the capability check
before linking. The runtime does not carry the wider I/O surface yet.

## Guides

- [Console streams](guides/streams) covers standard output, standard error,
  formatting, and the functions for strings you already hold.
