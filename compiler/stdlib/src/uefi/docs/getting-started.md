---
title: "Getting started with @std/uefi"
description: Build a first UEFI application, boot it under qemu with OVMF, and read its output.
section: Getting started
order: 10
---

# Getting started with @std/uefi

`@std/uefi` is the prelude for `x86_64-unknown-uefi`, so there is no import
line to write. The build loads the package as the target prelude, and
`println!` is already in scope. An explicit
`import { println } from "@std/uefi";` is not accepted, because the package is
not an ordinary library import on this target.

## Copy this

A UEFI application returns an integer from `Main`. The firmware returns to
the boot manager when `Main` returns:

```rnx
fn Main(): Int {
    println!("hello from uefi");
    return 0;
}
```

Save it as `src/main.rnx`, or use whichever file the project's `entries.main`
names. Install the target pack if `rnx target list` reports it missing, then
build:

```sh
rnx sysroot add x86_64-unknown-uefi
rnx build --target x86_64-unknown-uefi --release
```

The artifact is a PE32+ image with subsystem 10, the EFI application
subsystem. Place it at `EFI/BOOT/BOOTX64.EFI` inside a FAT image directory
named `esp`, then boot that directory under qemu with OVMF:

```sh
qemu-system-x86_64 -machine q35 -m 256 -bios /usr/share/edk2/x64/OVMF.4m.fd \
  -drive format=raw,file=fat:rw:esp -display none -serial stdio -no-reboot
```

The serial capture prints:

```text
hello from uefi
```

The same line appears on the firmware console. The runtime maps a zero return
to `EFI_SUCCESS` and any non-zero return to `EFI_ABORTED`, then returns to
the boot manager.

## Keep going

Read [Console output](guides/console) for formatting, `printLine`,
`printText`, and the `uefi_println!` alias. The project and toolchain manual
covers target packs and the boot loop in more detail.
