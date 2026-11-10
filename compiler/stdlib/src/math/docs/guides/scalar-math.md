---
title: "Scalar math"
description: "Compare, clamp, round, and raise Float values with Math."
section: Scalar math
order: 20
---

# Scalar math

The scalar helpers answer the everyday questions: which value is smaller, how do I keep a number inside a range, which integer is nearest, and what is this power or logarithm? All of them take and return `Float`, and none of them throw.

## Compare and clamp

```rnx
import { Math } from "@std/math";

println!(Math.abs(0.0 - 3.5));
println!(Math.min(3.0, 7.0));
println!(Math.max(3.0, 7.0));
println!(Math.clamp(9.0, 0.0, 1.0));
```

The lines print the absolute value 3.5, the smaller value 3, the larger value 7, and the clamped value 1. `abs` returns the value unchanged when it is not less than zero, so `-0.0` and NaN pass through. `min` returns the second argument when the two are equal. `clamp(v, minVal, maxVal)` does not check the bounds against each other.

## Round

```rnx
println!(Math.floor(0.0 - 2.5));
println!(Math.ceil(2.1));
println!(Math.round(2.5));
println!(Math.round(0.0 - 2.5));
```

`floor` gives the largest integer not greater than the value, so this prints `-3`, `3`, `3`, and `-3`. `round` sends ties away from zero rather than to the nearest even number, which is why `2.5` becomes 3 and `-2.5` becomes -3. All three return a `Float` even when the result is a whole number.

## Powers, roots, and logarithms

```rnx
println!(Math.pow(2.0, 10.0));
println!(Math.sqrt(9.0));
println!(Math.log(Math.E()));
```

The lines print `1024`, `3`, and `1`. `pow(0.0, 0.0)` is 1. A negative base with a fractional exponent gives NaN, and `0.0` raised to a negative power gives infinity. `log` is the natural logarithm: `log(1.0)` is 0, `log(0.0)` is negative infinity, and a negative argument gives NaN.

## NaN does not compare

`min`, `max`, `abs`, and `clamp` are written with `<` and `>` comparisons. A NaN loses every such comparison, so it is dropped or passed through depending on the call.

```rnx
let bad = Math.sqrt(0.0 - 1.0);
println!(Math.min(bad, 5.0));
println!(Math.max(bad, 5.0));
println!(Math.clamp(bad, 0.0, 1.0));
```

The first two lines print `5`, because the NaN loses the comparison and the other value comes back. The third prints `NaN`, because both clamp comparisons fail and the input is returned unchanged. `println!` renders the special values as `NaN`, `inf`, and `-inf`.
