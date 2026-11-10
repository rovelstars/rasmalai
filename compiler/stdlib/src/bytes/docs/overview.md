---
title: Byte buffers
description: Fixed-size byte buffers with explicit byte order for integers, floats, and text.
section: Overview
order: 0
---

# Byte buffers

`@std/bytes` gives you `ByteBuffer`, a fixed-size block of bytes with a length, an address, and typed reads and writes. Use it when a file, a network frame, or a C call carries binary data and both the byte order and the lane width matter. Text stays in `String`; everything else can go through a buffer.

A buffer is created once with `ByteBuffer.allocate`, `ByteBuffer.fromString`, or `ByteBuffer.fromArray`. It never grows or shrinks, so `length()` and `capacity()` always report the same number and the address from `address()` stays put. Reads and writes are bounds checked: an offset outside the buffer stops the program with `byte buffer out of bounds` instead of returning an error or clamping.

The accessors cover 8, 16, 32, and 64-bit integers and 32 and 64-bit floats, each with an `LE` or `BE` suffix where the width has more than one byte. Signed readers sign-extend and unsigned readers zero-extend. Writes keep the low bits of the value, so `writeInt8` and `writeUInt8` store the same byte. Choose the reader that matches how the field should be interpreted.

```rnx
import { ByteBuffer } from "@std/bytes";

let frame = ByteBuffer.allocate(8);
frame.writeUInt16BE(0, 0x1234);
frame.writeInt32LE(2, -2);
println!(frame.readUInt16BE(0), frame.readInt32LE(2));
```

That prints `4660 -2`.

## Guides

[Getting started](getting-started) shows the import and a first program. [Byte order](guides/byte-order) explains when to pick `LE` or `BE` and what a mismatched reader does to a field. [Reading and writing lanes](guides/reading-and-writing) covers lane widths, text, slicing, and copying ranges.
