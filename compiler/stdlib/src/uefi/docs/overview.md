---
title: "@std/uefi"
description: Prelude and console output for x86_64-unknown-uefi applications.
section: Overview
order: 0
---

# @std/uefi

`@std/uefi` is the prelude and console surface for `x86_64-unknown-uefi`.
A firmware application starts with no operating system underneath it. There
is no stdout file descriptor, no `@std/io`, and no `@std/fs`. The firmware
hands the image a system table, and text reaches the user through that
table's console protocol.

```rnx
fn Main(): Int {
    println!("hello from uefi");
    return 0;
}
```

`@std/uefi` is selected automatically as the prelude for the target, so
`println!` is already in scope. The package also re-exports the foundation
catalog (`Int`, `String`, `Array`, `Map`, and the rest) and `Date` from
`@std/time`.

## What is inside

The console surface is two call macros and two plain functions:

- `println!` renders its arguments as one line on ConOut and on COM1.
- `uefi_println!` is the same macro under a name that cannot be confused with
  a host console macro.
- `printLine(text)` prints a `String` followed by a newline.
- `printText(text)` prints a `String` with no trailing newline.

Each write goes twice: once through ConOut, which the firmware draws, and
once to the serial port when the platform exposes one. A qemu run with
`-serial stdio` therefore captures the same text the screen shows.

## Target details

The package declares the `console`, `clock`, and `firmware` capabilities. The
UEFI target provides all three. Building the same import for a host target
fails the capability check with `E108`, because host descriptors do not
provide `firmware`.

The target is Tier 3 and experimental. A build produces a PE32+ image with
subsystem 10, the EFI application subsystem, and entry `efi_main`. The image
imports nothing and needs no C runtime. A target pack supplies the UEFI
runtime archive that implements the console and allocator.

Maps, sets, JSON, files, sockets, threads, and the event loop are not
implemented by the UEFI runtime. A program that uses them fails at link with
the missing symbol. `println!` renders `Int`, `Bool`, `Float`, `String`, and
`null`; arrays and objects render as placeholders.

## Guides

- [Console output](guides/console) covers formatting, the print functions,
  and what reaches the serial port.
