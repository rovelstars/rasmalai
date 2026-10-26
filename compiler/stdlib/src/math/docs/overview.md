# @std/math

Scalar math helpers and the Vec2 value type. Angles are radians, so turn
degrees into radians with value * Math.PI() / 180.0.

```rnx
import { Math, Vec2 } from "@std/math";

print(new Vec2(3.0, 4.0).length());
print(Math.PI());
```

## What is inside

- `Math`
- `Vec2`

## When to use

Reach for @std/math for scalar numerics (sqrt, trig, floor/ceil) and the Vec2 value type for 2D geometry. For 4-lane vector throughput, step up to @std/simd.

It covers: IEEE-754 intrinsics; Vec2 geometry; Float helpers.
