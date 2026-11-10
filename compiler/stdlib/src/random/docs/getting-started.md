---
title: Getting started with @std/random
description: Import Rng, seed two streams, and confirm they produce the same values.
section: Getting started
order: 10
---

# Getting started with @std/random

The module ships inside the compiler. Import what you need:

```rnx
import { Rng } from "@std/random";
```

## Copy this

```rnx
import { Rng } from "@std/random";

let a = Rng.seeded(7);
let b = Rng.seeded(7);
let x = a.next();
let y = b.next();
println!(x == y);
println!(a.int(1, 10), b.int(1, 10));
```

Run it with `rnx run`. The first line is `true`, and the last line prints two equal numbers. Both streams started at seed `7`, and each received the same calls in the same order.

## Where the seeds go

Use a fixed seed when you want the same run again, and a value from `CryptoRng.int()` when you want a different sequence each time. The seed is an `Int`, so any integer works.
