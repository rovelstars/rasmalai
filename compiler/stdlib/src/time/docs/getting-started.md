---
title: Getting started with @std/time
description: Import Clock, advance a virtual clock, and read a duration in milliseconds and seconds.
section: Getting started
order: 10
---

# Getting started with @std/time

The module ships inside the compiler. Import the names you need:

```rnx
import { Clock } from "@std/time";
```

## Copy this

```rnx
import { Clock } from "@std/time";

let clock = Clock.virtual(0);
clock.tick(2500000);
println!(clock.now().toMillis());
println!(clock.now().toSeconds());
```

Run it with `rnx run`. The output is:

```text
2
0.0025
```

`Clock.virtual(0)` starts at zero nanoseconds. `tick(2500000)` adds 2.5 milliseconds without waiting, and `now()` returns a `Duration`. `toMillis()` truncates to whole milliseconds, while `toSeconds()` keeps the fraction.

For real elapsed time, sample `Clock.mono()` before and after the work:

```rnx
import { Clock } from "@std/time";

let start = Clock.mono();
let total = 0;
let i = 0;
while i < 1000 {
    total = total + i;
    i = i + 1;
}
let elapsed = Clock.mono().nanos - start.nanos;
println!(elapsed >= 0, total);
```

The difference is nanoseconds. Only differences between two readings are meaningful; the absolute value is not a wall-clock time.
