---
title: Byte order
description: How the LE and BE suffixes decide which byte comes first, and how to keep a reader matched to its writer.
section: Byte order
order: 20
---

# Byte order

A 16-bit value needs two bytes, and the order of those bytes is not universal. The same pair can mean `0x1234` or `0x3412`. Every multi-byte accessor in `ByteBuffer` carries a `LE` or `BE` suffix so the choice is written at the call site instead of depending on the machine.

`BE` means big-endian: the most significant byte goes at the lowest offset. This is network byte order, and it is common in file headers.

```rnx
import { ByteBuffer } from "@std/bytes";

let be = ByteBuffer.allocate(2);
be.writeUInt16BE(0, 0x1234);
println!(be.readUInt8(0), be.readUInt8(1));
println!(be.readUInt16BE(0));
```

That prints `18 52` and `4660`. `18` is `0x12` and `52` is `0x34`, so the high byte came first. Reading the pair with `readUInt16BE` puts it back together.

`LE` means little-endian: the least significant byte goes first. x86 and ARM processors store integers this way in memory, and many desktop file formats use it.

```rnx
import { ByteBuffer } from "@std/bytes";

let le = ByteBuffer.allocate(2);
le.writeUInt16LE(0, 0x1234);
println!(le.readUInt8(0), le.readUInt8(1));
println!(le.readUInt16LE(0));
```

That prints `52 18` and `4660`. The bytes are reversed, and the matching reader still returns the original value.

## The failure case

A reader with the wrong suffix does not fail. It returns a different number, and that is the part that hides in production.

```rnx
import { ByteBuffer } from "@std/bytes";

let mixed = ByteBuffer.allocate(4);
mixed.writeUInt16LE(0, 0x1234);
mixed.writeUInt16LE(2, 0xabcd);
println!(mixed.readUInt16BE(0));
println!(mixed.readUInt16BE(2));
```

That prints `13330` and `52651`, which are `0x3412` and `0xcdab`. The data was written little-endian and read big-endian, so each field came back byte-swapped.

Pick one order per field and use it for both the write and the read. For a wire protocol, read the specification. For a file format, read its header description. Do not guess from the host: `ByteBuffer` never consults the host byte order, which is what makes the same code portable.

The same rule applies to 32 and 64-bit integers and to `readFloat32LE`, `readFloat32BE`, `readFloat64LE`, and `readFloat64BE`.
