# @std/uefi

Prelude and console output for `x86_64-unknown-uefi` applications.

A UEFI application has no operating system underneath it. There is no
stdout file descriptor, no `@std/io`, and no `@std/fs`. The firmware hands
the image a system table, and everything the application shows a user goes
through that table's console protocol. `@std/uefi` is the small surface for
that: one call macro and two plain functions.

```rnx
import { println } from "@std/uefi";

fn Main(): Int {
    println!("hello from uefi");
    return 0;
}
```

## What is inside

- the foundation catalog and `Date` re-exports, so a UEFI program needs no
  stock prelude import
- `println!`, the call macro that renders its arguments as one line
- `uefi_println!`, the same macro under a distinct name
- `printLine(text)` and `printText(text)` for a `String` you already hold

## When to use

`@std/uefi` is the prelude for `x86_64-unknown-uefi`. The package declares
the `console`, `clock`, and `firmware` capabilities, which the UEFI
descriptor provides. Importing it on a host target passes the capability
gate but fails to link, because only the UEFI runtime archive defines
`__rnx_uefi_println`.

Every line is written twice: once through ConOut, which the firmware draws,
and once to the serial port when the platform exposes one. qemu with
`-serial stdio` therefore captures the same text the screen shows.
