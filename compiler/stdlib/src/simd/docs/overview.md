---
title: "@std/simd"
description: "128-bit float and integer vectors that stay in registers and allocate nothing."
section: Overview
order: 0
---

# @std/simd

`@std/simd` provides two 128-bit value types. `Vec4f` holds four 32-bit floats. `Vec4i` holds four signed 32-bit integers. Both stay in vector registers, so building one, combining two, or reading a lane allocates nothing and emits no ARC retain or release.

Lane order is fixed: lane 0 is `x`, lane 1 is `y`, lane 2 is `z`, and lane 3 is `w`. `x()`, `y()`, `z()`, and `w()` read a fixed lane. `get(lane)` reads a lane chosen at run time, which is what a loop or an array index needs.

```rnx
import { Vec4f } from "@std/simd";

let v = Vec4f.splat(2.0);
println!(v.dot(v));
```

The program prints `16`. `splat` fills all four lanes with `2.0`, and `dot` sums the four lane products.

The operators `+`, `-`, `*`, and `/` apply one operation to all four lanes and return a new vector. Both operands must be the same vector type: `Vec4f` with `Vec4f`, or `Vec4i` with `Vec4i`. Those four operators are the whole operator set. `==`, `<`, and the rest are rejected with `E108`, so extract lanes and compare those.

The two types store lanes differently. Float lanes are 32-bit, so a `Float` is narrowed on the way into a `Vec4f` and widened on the way out, keeping about seven decimal digits. Integer lanes are signed 32-bit, so a lane keeps only its low 32 bits and arithmetic wraps inside the signed 32-bit range.

## Guides

[Float vectors](guides/float-vectors) covers `Vec4f`, lane reads, dot products, and lane-wise math. [Integer vectors](guides/integer-vectors) covers `Vec4i`, wrapping arithmetic, and division. [Portability](guides/portability) covers the instruction baseline, release tiers, lane traps, and NaN rules.
