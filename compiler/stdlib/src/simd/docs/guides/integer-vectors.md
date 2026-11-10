---
title: "Integer vectors"
description: "Build a Vec4i, read its lanes, and work with wrapping 32-bit arithmetic."
section: Integer vectors
order: 30
---

# Integer vectors

`Vec4i` packs four signed 32-bit integers into one 128-bit register value. Use it for counts, indices, flags, and small integer samples where four values move together. The constructor takes exactly four `Int` lanes in x/y/z/w order. Anything else is `E108`.

## Build and read

```rnx
import { Vec4i } from "@std/simd";

let a = new Vec4i(1, 2, 3, 4);
let b = Vec4i.splat(10);
let sum = a + b;

println!(sum.x() == 11, sum.w() == 14);
println!(a.z() == 3);
```

The first line is `true true`. The second is `true`. `splat` broadcasts one value to all four lanes.

Lane reads sign-extend from 32 bits, so a lane of `-1` reads back as `-1` on every backend.

## Pick a lane at run time

```rnx
let v = new Vec4i(1, 2, 3, 4);
let sum = 0;
let i = 0;
while i < 4 {
    sum = sum + v.get(i);
    i = i + 1;
}
println!(sum == 10);
```

The program prints `true`. `get(lane)` takes an index from 0 to 3. An index outside that range traps everywhere with `vector lane N out of range`.

## Arithmetic wraps

Integer lanes are signed 32-bit. A wider value keeps only its low 32 bits, and `+`, `-`, `*`, and `/` wrap inside the signed 32-bit range instead of growing. Division truncates toward zero.

A zero lane in the divisor is not portable. The interpreter stops with `division by zero`, Cranelift traps, and LLVM returns an unspecified value. Test divisors before dividing.

```rnx
let a = new Vec4i(8, 9, 10, 11);
let b = new Vec4i(2, 3, 3, 4);
let q = a / b;
println!(q.x() == 4, q.y() == 3, q.z() == 3, q.w() == 2);
```

The program prints `true true true true`. The division truncates each lane independently.

## Integer lanes have no float helpers

`min`, `max`, `dot`, and `sqrt` belong to `Vec4f`. Calling one on a `Vec4i` is `E108`. Extract lanes with `x()`, `y()`, `z()`, `w()`, or `get()` and compare those when you need an integer lane-wise choice.
