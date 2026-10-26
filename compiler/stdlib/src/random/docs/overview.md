# @std/random

Deterministic seeded random streams and OS entropy.

```rnx
import { Rng } from "@std/random";

let r = Rng.seeded(7);
print(r.next());
```

## What is inside

- `Rng`
- `CryptoRng`

## When to use

Reach for @std/random in two shapes: Rng.seeded(n) for deterministic streams that replay bit-identically (simulations, tests), CryptoRng for OS entropy when unpredictability matters.

It covers: Seeded Rng streams; OS-backed CryptoRng; Ints, bools, ranges.
