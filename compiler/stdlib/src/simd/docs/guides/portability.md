---
title: "Portability"
description: "How vector operations map to CPU instructions and which lane rules hold on every backend."
section: Portability
order: 40
---

# Portability

Vector code should produce the same values on every machine that runs the same target architecture. This page covers the instruction baseline, the release tiers, and the lane rules that hold across the interpreter, Cranelift, and LLVM.

## Instruction baseline

Every `Vec4f` and `Vec4i` operation is a single 128-bit instruction on the portable baseline: SSE2 on x86-64 and NEON on aarch64. A release artifact built for a target architecture runs on any CPU of that architecture.

Functions that contain loops additionally ship tiered versions. The baseline is joined by x86-64 micro-architecture levels v2, v3, and v4 on x86-64, with a startup-resolved dispatch stub. Every tier computes identical values. The tier only changes which instructions carry the operation.

## No heap traffic

Building a vector, combining two vectors, and reading a lane allocate nothing and emit no ARC retain or release. Vectors pass to and from functions by value, like `Int` or `Float`.

## Lane rules

Lane order is always x, y, z, w. `get(lane)` takes 0 through 3, and an index outside that range traps on every backend with `vector lane N out of range`.

Lane reads widen to the host scalar type. Integer lanes sign-extend from 32 bits, so `-1` stays `-1`. Float lanes widen from 32-bit float to `Float` and keep about seven decimal digits.

`min` and `max` return the other lane when exactly one side is NaN. When both sides are NaN, the result is NaN, and the payload bits are backend-defined.

Comparison operators are not part of the vector surface. `==`, `<`, and the rest are rejected with `E108`. Extract lanes and compare those:

```rnx
import { Vec4f } from "@std/simd";

let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = new Vec4f(1.0, 2.0, 0.0, 4.0);
println!(a.z() == b.z(), a.w() == b.w());
```

The program prints `false true`.

Integer division by a zero lane is the one operation with different behavior per backend: the interpreter stops with `division by zero`, Cranelift traps, and LLVM returns an unspecified value. Check the divisor lanes before dividing.
