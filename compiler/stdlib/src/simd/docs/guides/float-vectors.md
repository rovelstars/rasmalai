---
title: "Float vectors"
description: "Build a Vec4f, read its lanes, and use dot, min, max, and sqrt."
section: Float vectors
order: 20
---

# Float vectors

`Vec4f` packs four 32-bit floats into one 128-bit register value. Use it when four related floats move together: a position, a color, a set of weights, or a row of samples. The constructor takes exactly four `Float` lanes in x/y/z/w order. Anything else, including a mix of `Float` and `Int`, is `E108`.

## Build and read

```rnx
import { Vec4f } from "@std/simd";

let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = Vec4f.splat(2.0);

println!(a.x() == 1.0, a.y() == 2.0, a.z() == 3.0, a.w() == 4.0);
println!(a.dot(b) == 20.0);
```

The first line is `true true true true`. The second is `true`, because `1*2 + 2*2 + 3*2 + 4*2` is 20. `splat` broadcasts one value to all four lanes.

Lane reads widen from 32-bit float to `Float`. A value outside the `f32` range becomes infinity on the way in.

## Pick a lane at run time

`get(lane)` takes a lane index from 0 to 3. Use it when the index comes from a loop or an array.

```rnx
let v = new Vec4f(1.0, 2.0, 3.0, 4.0);
let sum = 0.0;
let i = 0;
while i < 4 {
    sum = sum + v.get(i);
    i = i + 1;
}
println!(sum == 10.0);
```

The program prints `true`. An index outside 0 to 3 traps on every backend with `vector lane N out of range`, so keep the loop bounds in range.

## Lane-wise math

The four operators work lane by lane:

```rnx
let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = Vec4f.splat(2.0);
let sum = a + b;
println!(sum.x() == 3.0, sum.w() == 6.0);
```

That prints `true true`. `min`, `max`, and `sqrt` add the rest of the float surface:

```rnx
let low = new Vec4f(1.0, 5.0, 3.0, 4.0).min(new Vec4f(4.0, 2.0, 6.0, 0.0));
println!(low.x() == 1.0, low.y() == 2.0, low.z() == 3.0, low.w() == 0.0);

let roots = new Vec4f(4.0, 9.0, 16.0, 25.0).sqrt();
println!(roots.x() == 2.0, roots.z() == 4.0);
```

The first line prints four `true` values, and the second prints two. `min` and `max` keep equal lanes and return the other lane when exactly one side is NaN. `sqrt` gives NaN for a negative lane, following IEEE-754.

## Compare lanes, not vectors

`==` and `<` do not accept vectors. Extract the lanes you care about and compare those:

```rnx
let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = new Vec4f(1.0, 2.0, 0.0, 4.0);
println!(a.z() == b.z(), a.w() == b.w());
```

The program prints `false true`.
