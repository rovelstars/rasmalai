---
title: "Vectors"
description: "Build a Vec2, read and change its fields, and measure it with dot and length."
section: Vectors
order: 40
---

# Vectors

`Vec2` holds an x and a y component as `Float` values. The fields are public and mutable, so the type works like a plain record. It is meant for positions, offsets, and directions: nothing is normalized, and no component is range-checked.

## Build and read

```rnx
import { Vec2 } from "@std/math";

let v = new Vec2(3.0, 4.0);
println!(v.x);
println!(v.y);
println!(v.lengthSq());
println!(v.length());
```

The fields print `3` and `4`. `lengthSq()` is `x*x + y*y`, so it prints `25`. `length()` is `Math.sqrt(lengthSq())`, so it prints `5`.

`length()` calls the square root every time. When you only need to compare two vectors, use `lengthSq()` and skip the root.

## Dot products

`dot(other)` returns `this.x * other.x + this.y * other.y`. It is zero for two non-zero perpendicular vectors, and positive when they point in a similar direction.

```rnx
let v = new Vec2(3.0, 4.0);
println!(v.dot(new Vec2(1.0, 0.0)));
println!(v.dot(new Vec2(0.0, 1.0)));
println!(v.dot(new Vec2(0.0 - 4.0, 3.0)));
```

The lines print `3`, `4`, and `0`. The last pair is perpendicular, so the dot product is zero.

## Change a component

The fields are not read-only:

```rnx
let v = new Vec2(3.0, 4.0);
v.x = 6.0;
println!(v.lengthSq());
```

The program prints `52`, because the squared length is now `6*6 + 4*4`. Nothing recomputes in the background; `length()` and `lengthSq()` read the current fields each time.

## A real case

```rnx
fn nearer(a: Vec2, b: Vec2, point: Vec2): Vec2 {
    let da = new Vec2(point.x - a.x, point.y - a.y);
    let db = new Vec2(point.x - b.x, point.y - b.y);
    if da.lengthSq() < db.lengthSq() {
        return a;
    }
    return b;
}

let origin = new Vec2(0.0, 0.0);
let far = new Vec2(10.0, 0.0);
let pick = nearer(origin, far, new Vec2(2.0, 0.0));
println!(pick.x);
```

The program prints `0`, because the origin is nearer to `(2.0, 0.0)` than the far point. Comparing squared lengths avoids three square roots in the decision, and the vector subtraction produces plain offsets with no hidden state.
