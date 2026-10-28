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

## Portability

Every `Vec4f`/`Vec4i` operation is a single 128-bit instruction on the
portable baseline (SSE2 on x86-64, NEON on aarch64), so release artifacts
run on any CPU of the target architecture. Functions that contain loops
additionally ship tiered versions (baseline plus x86-64 micro-architecture
levels v2/v3/v4 on x86-64) with a startup-resolved dispatch stub; every tier
computes identical values. `min`/`max` return the other lane when exactly
one side is NaN, on all backends; lane reads sign-extend; an out-of-range
lane index traps everywhere.

