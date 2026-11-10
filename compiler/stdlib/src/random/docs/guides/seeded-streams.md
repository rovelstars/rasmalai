---
title: Seeded streams
description: Replay a sequence in tests, draw an inclusive range, and choose OS entropy when predictability matters.
section: Streams
order: 20
---

# Seeded streams

A randomized test is only useful if a failure can be replayed. A seeded stream turns randomness into a sequence of values determined by one integer. Store the seed next to the failure, run it again, and the same calls take the same path.

```rnx
import { Rng } from "@std/random";

let dice = Rng.seeded(42);
let rolls = [dice.int(1, 6), dice.int(1, 6), dice.int(1, 6)];
println!(rolls);
```

That prints three values in `1..6` and repeats them on every run. `Rng.seeded(42)` and `new Rng(42)` build the same stream; the factory is the shorter form.

## Ranges and raw words

`next()` is the raw draw: one signed 64-bit word, and it advances the stream. `int(min, max)` takes the raw draw, removes the sign, and maps it with modulo into the inclusive range. `bool()` returns `true` when the draw is even and `false` when it is odd.

```rnx
import { Rng } from "@std/random";

let r = Rng.seeded(3);
println!(r.next());
println!(r.int(-5, 5));
println!(r.bool());
```

The range is inclusive at both ends, so `int(1, 6)` can return `1` and can return `6`. Pass `min <= max`; there is no guard for a reversed range. Because the mapping uses modulo rather than rejection sampling, a range that does not divide the draw space evenly is very slightly biased. That is fine for games, simulations, and tests; do not use it for cryptographic selection.

## OS entropy

`CryptoRng.int()` reads one word from the operating system. It has no seed and no state to replay. Use it for keys, session tokens, salts, and anything else an attacker must not predict.

```rnx
import { CryptoRng } from "@std/random";

let key = [CryptoRng.int(), CryptoRng.int()];
println!(key.length());
```

That prints `2`. A real key needs more bits than two 64-bit words, so draw as many words as the key size requires. Do not replace `CryptoRng` with `Rng.seeded(clockValue)`; the seed is visible and the sequence is reproducible by design.
