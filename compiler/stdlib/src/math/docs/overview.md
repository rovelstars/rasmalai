---
title: "@std/math"
description: "Scalar math helpers on Math and the Vec2 value type for 2D geometry."
section: Overview
order: 0
---

# @std/math

`@std/math` has two parts. `Math` is a class of static helpers for constants, comparisons, rounding, powers, roots, logarithms, and trigonometry. `Vec2` is a small two-component vector type for positions, offsets, and directions.

Angles are radians. Turn degrees into radians with `value * Math.PI() / 180.0`.

```rnx
import { Math, Vec2 } from "@std/math";

let v = new Vec2(3.0, 4.0);
println!(v.length());
println!(Math.sin(Math.PI() / 2.0));
```

The first line prints `5`, the length of the vector. The second prints the sine of a right angle, which is 1.

`Math` declares no `init`, so an instance holds nothing and reaches no method. Call every member as `Math.name(...)`. The runtime-backed functions are `sqrt`, `sin`, `cos`, `tan`, `atan2`, `pow`, `floor`, `ceil`, `round`, and `log`. They follow IEEE-754: a negative argument to `sqrt` or `log` gives NaN, `log(0.0)` gives negative infinity, and `0.0` raised to a negative power gives infinity. Nothing in the class throws.

`Vec2` is a plain record. Its `x` and `y` fields are public and mutable. Nothing is normalized, no component is range-checked, and the type is meant for geometry rather than unit-vector invariants.

## Guides

[Scalar math](guides/scalar-math) covers comparisons, clamping, rounding, powers, roots, logarithms, and NaN behavior. [Trigonometry](guides/trigonometry) covers radians, the sine and cosine family, and `atan2`. [Vectors](guides/vectors) covers `Vec2`, dot products, and lengths.
