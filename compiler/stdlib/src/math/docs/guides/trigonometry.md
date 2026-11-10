---
title: "Trigonometry"
description: "Work in radians with PI, TAU, the sine and cosine family, and atan2."
section: Trigonometry
order: 30
---

# Trigonometry

Every angle in `@std/math` is in radians. If your input is in degrees, convert it once at the edge:

```rnx
import { Math } from "@std/math";

let degrees = 90.0;
let radians = degrees * Math.PI() / 180.0;
println!(radians);
```

The same formula works in reverse: `radians * 180.0 / Math.PI()` gives degrees.

## Constants

`Math.PI()` is the ratio of a circle's circumference to its diameter. `Math.TAU()` is a full turn, or `2 * PI()`. `Math.E()` is the base of the natural logarithm, and `Math.log(Math.E())` is 1.

```rnx
println!(Math.PI());
println!(Math.TAU());
println!(Math.E());
```

## Sine, cosine, and tangent

```rnx
let deg = 90.0;
println!(Math.sin(deg * Math.PI() / 180.0));
println!(Math.cos(Math.PI()));
println!(Math.round(Math.tan(Math.PI() / 4.0) * 1000.0) / 1000.0);
```

The first line is the sine of 90 degrees, which is 1. The second is the cosine of half a turn, which is -1. The third rounds the tangent of 45 degrees to three decimals, which is 1.

`sin` and `cos` stay inside -1 and 1. `tan` is unbounded: it grows without limit as the angle approaches `Math.PI() / 2.0` from either side.

## atan2 takes y first

`Math.atan2(y, x)` returns the angle of the vector `(x, y)` in radians, in the range from `-PI` to `PI`, measured counterclockwise from the positive x axis. The argument order is vertical first, horizontal second.

```rnx
println!(Math.atan2(1.0, 1.0) * 180.0 / Math.PI());
println!(Math.atan2(0.0, -1.0) * 180.0 / Math.PI());
```

The first line is the angle of the vector `(1, 1)` in degrees, about 45. The second is the angle of `(-1, 0)`, which is 180. `atan2(0.0, 0.0)` is 0 by definition.

## Keep a phase in one turn

`TAU()` is the natural upper bound for a phase. A value in `[0.0, TAU())` needs no normalization before it reaches `sin` or `cos`.

```rnx
let phase = Math.TAU() / 4.0;
println!(Math.round(Math.sin(phase) * 1000.0) / 1000.0);
```

The phase is a quarter turn, so the sine is 1. The program multiplies by 1000, rounds, and divides back to trim the noise from the floating-point result.
