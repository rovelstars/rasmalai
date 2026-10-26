# @std/simd

128-bit SIMD vector value types with no heap allocation.

Two types: `Vec4f` holds four 32-bit floats, `Vec4i` holds four signed
32-bit ints. Both are 128 bits wide and stay in vector registers, so
building one, combining two, or reading a lane allocates nothing and emits
no ARC retain or release.

```rnx
import { Vec4f } from "@std/simd";

let v = Vec4f.splat(2.0);
print(v.dot(v));
```

## What is inside

- `Vec4f`
- `Vec4i`

## When to use

Reach for @std/simd when four floats (or ints) move together: graphics, physics, signal processing. Vec4f lives inline with plain operators — no heap, no ARC traffic, no intrinsics headers.

It covers: Vec4f/Vec4i lanes; Reductions (dot/min/max); Zero-allocation ops.
