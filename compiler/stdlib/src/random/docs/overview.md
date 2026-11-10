---
title: Random streams
description: Reproducible seeded streams for tests and OS entropy for security-sensitive values.
section: Overview
order: 0
---

# Random streams

`@std/random` separates two jobs that are often mixed by mistake. A seeded `Rng` gives a repeatable sequence: the same seed and the same calls produce the same draws, which makes a failing test reproducible. `CryptoRng` draws from the operating system: there is no seed to replay, and the values are meant for keys, tokens, and salts.

`Rng` is a Xoshiro256++ stream. `next()` returns one raw signed 64-bit word and advances the state. `int(min, max)` returns an integer in the inclusive range, and `bool()` returns a coin flip. `CryptoRng.int()` is a static call that returns one OS-provided word.

```rnx
import { Rng, CryptoRng } from "@std/random";

let r = Rng.seeded(7);
println!(r.int(1, 6));
println!(r.bool());
println!(CryptoRng.int());
```

The first two lines repeat on every run for seed `7`. The third does not.

Use `Rng.seeded` in tests, simulations, and procedural generation where you need to replay a sequence. Use `CryptoRng` whenever an attacker must not predict the value. A seeded stream is not a substitute for OS entropy.

## Guide

[Seeded streams](guides/seeded-streams) covers reproducibility, ranges, and the difference between the two generators.
