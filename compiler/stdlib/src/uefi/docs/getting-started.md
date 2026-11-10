# Getting started with @std/uefi

The module ships inside the compiler. Import what you need:

```rnx
import { println } from "@std/uefi";
```

## Copy this

A UEFI application returns an exit code from `Main`. The firmware returns
to the boot manager when `Main` returns:

```rnx
import { println } from "@std/uefi";

fn Main(): Int {
    println!("hello from uefi");
    return 0;
}
```

Build it for the UEFI target:

```sh
rnx build src/main.rnx --target x86_64-unknown-uefi --release
```

The output is a PE32+ image with subsystem 10. Copy it to
`EFI/BOOT/BOOTX64.EFI` on a FAT image and boot the image under qemu with
OVMF:

```sh
qemu-system-x86_64 -machine q35 -m 256 -bios /usr/share/edk2/x64/OVMF.4m.fd \
  -drive format=raw,file=fat:rw:esp -display none -serial stdio -no-reboot
```

## Keep going

Read the [overview](overview) for the capability story, then see the
project and toolchain manual page for target packs and the boot loop.
