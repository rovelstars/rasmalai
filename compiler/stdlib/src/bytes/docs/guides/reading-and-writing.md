---
title: Reading and writing lanes
description: Size a buffer, pack fields at exact offsets, and copy ranges without sharing memory.
section: Buffers
order: 30
---

# Reading and writing lanes

A binary record is a sequence of fixed-width fields at fixed offsets. You allocate enough bytes, write each field at its offset, and read it back with the same width. `ByteBuffer` does not track a cursor, so the offset is always explicit.

```rnx
import { ByteBuffer } from "@std/bytes";

let b = ByteBuffer.allocate(13);
b.writeUInt8(0, 7);
b.writeInt16LE(1, -2);
b.writeFloat32LE(3, 1.5);
let n = b.writeString(7, "ok");
println!(b.length(), b.capacity(), n);
println!(b.readUInt8(0), b.readInt16LE(1), b.readFloat32LE(3));
println!(b.readString(7, n));
```

That prints `13 13 2`, then `7 -2 1.5`, then `ok`. The fields occupy offset 0, offsets 1 through 2, offsets 3 through 6, and offsets 7 through 8. `writeString` returns the byte count, which is what `readString` takes.

The lane has to fit. A 4-byte read at offset 11 in this buffer stops the program with `byte buffer out of bounds`, because the last valid start is 9. The same message covers reads and writes, and a negative capacity stops allocation with `negative byte buffer capacity`. There is no error value to check.

## Signed, unsigned, and widths

`readInt8` sign-extends, so byte `0xFF` comes back as `-1`. `readUInt8` zero-extends and gives `255`. The same split exists at 16 and 32 bits, where `readInt16LE` and `readUInt16LE` read the same two bytes with different results.

Writes do not split. `writeInt8` and `writeUInt8` both store the low 8 bits, and `writeInt16LE` and `writeUInt16LE` both store the low 16 bits. A later read decides whether the byte should be treated as signed. There is no unsigned 64-bit reader because `Int` is already 64 bits and holds the same bit pattern either way.

Float lanes have their own widths. A 32-bit float keeps about 7 decimal digits, so `0.1` reads back as `0.10000000149011612`. The 64-bit accessors keep the full value.

## Copying ranges

`slice(start, end)` copies bytes into a new buffer. `copyWithin(target, start, end)` moves a range inside the buffer that already exists and handles overlap the way `TypedArray.set` does.

```rnx
import { ByteBuffer } from "@std/bytes";

let src = ByteBuffer.fromArray([0, 1, 2, 3, 4, 5]);
let mid = src.slice(1, 4);
src.copyWithin(2, 0, 3);
println!(mid.length(), mid.readUInt8(0), mid.readUInt8(2));
println!(src.readUInt8(2), src.readUInt8(3));
```

That prints `3 1 3` and `0 1`. `mid` holds its own copy, so the later `copyWithin` does not change it. Both ranges are checked before anything is written, so a bad call stops the program without a partial copy.

## Text

`fromString` sizes the buffer to the UTF-8 byte count, not the character count. A string containing `é` has one character and two bytes. `writeString` writes the encoding and returns the byte count; it appends no NUL terminator, so a C function that expects one needs a zero byte stored after the text. `readString` decodes invalid bytes as U+FFFD instead of failing.

## Raw addresses

`address()` returns the first byte for `Pointer.fromAddress` inside `unsafe`. Size the buffer for the element type: a pointer to a 64-bit integer over a 2-byte buffer reads 8 bytes and walks off the end. Keep the buffer alive for as long as the pointer is used, and stay within `capacity()`.
