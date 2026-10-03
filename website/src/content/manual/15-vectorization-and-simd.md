---
title: "Vectorization and SIMD"
description: "Vec4f hardware lane mapping, lane arithmetic, and horizontal reductions."
section: "Memory, Systems, and Concurrency"
icon: "Timer"
---

# Vectorization and SIMD

Scalar code touches one number at a time; hot loops touch four. `Vec4f` and `Vec4i` are 128-bit SIMD value types: four lanes in one register, no heap, no ARC traffic, plain operators.

## Constructing vectors

Four lanes from four values, or one value broadcast to all four with
`splat`. Vectors are values: they pass as function arguments and return
from functions on every backend:

```rnx
import { Vec4f } from "@std/simd";

fn dbl(v: Vec4f): Vec4f {
    return v * Vec4f.splat(2.0);
}

print(dbl(new Vec4f(1.0, 2.0, 3.0, 4.0)).get(2));
```

`Vec4i` mirrors the shape for 32-bit integer lanes (`new Vec4i(1, 2, 3, 4)`,
`Vec4i.splat(3)`), with wrapping integer arithmetic.

> [!NOTE]
> Everything in this chapter behaves identically on the Cranelift (dev)
> and LLVM (release) backends. If a result ever differs between the two,
> that is a compiler bug: please report it.

## Lane arithmetic

`+ - * /` combine lane-wise, and `/` on integer lanes traps on zero
divisors like scalar division. Read lanes back with `x()`/`y()`/`z()`/`w()`
or the indexed `get(i)` (out-of-range lanes trap):

```rnx
import { Vec4f } from "@std/simd";

let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = Vec4f.splat(10.0);
let c = a + b;
print(c.get(0), c.get(3));
```

Reductions collapse lanes to scalars: `dot` sums pairwise products,
`min`/`max` take per-lane extrema, and `sqrt` roots each lane (negative
lanes yield NaN per IEEE-754):

```rnx
import { Vec4f } from "@std/simd";

let v = new Vec4f(4.0, 9.0, 16.0, 25.0);
print(v.sqrt().get(3), v.dot(Vec4f.splat(1.0)));
```

## Float discipline still applies

Lane values are strict `Float`s. Mixing a `FastFloat` lane source without
`.asFast()` is the same `E305` error as scalar code, and `Int` lane
readouts convert with `Int(...)`. The process entry point still returns
`Int` — vectors travel through helpers, and lanes cross the final
boundary.

## Summary

- `Vec4f`/`Vec4i`: construct, `splat`, lane operators, `x/y/z/w/get`.
- Reductions: `dot`, `min`, `max`, `sqrt` with IEEE-754 edge semantics.
- First-class function values on all backends; `Int` at the entry point.
