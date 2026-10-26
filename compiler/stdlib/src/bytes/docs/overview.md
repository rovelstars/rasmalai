# @std/bytes

Raw byte buffers with numeric lanes in either byte order.

`ByteBuffer` is a fixed-size block of bytes with an address, a length,
and endian-aware accessors for the usual integer and float widths. Use
it for file contents, network frames, C ABI calls, and hand-written
wire formats, where the byte order and the lane width both matter.

```rnx
import { ByteBuffer } from "@std/bytes";

let buf = ByteBuffer.allocate(8);
buf.writeInt32BE(0, 0x01020304);
print(buf.readInt32BE(0));
```

## What is inside

- `ByteBuffer`

## When to use

Reach for @std/bytes when a program talks to the outside world in binary: wire protocols, file headers, or any format where byte order matters. Text stays in String; everything else goes through a ByteBuffer.

It covers: Fixed-size raw buffers; Explicit-endian int/float lanes; Zero-copy slicing.
