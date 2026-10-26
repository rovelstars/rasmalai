# @std/time

Time sources for measuring elapsed work and for tests that need a clock
they control.

The module has two kinds of clock, and they answer different questions.
`Clock.mono()` is monotonic hardware time: it always moves forward, and it
is unaffected by system clock adjustments, so it is what you want for
timeouts, deadlines, and profiling. `Clock.virtual()` is a plain `Int` you
advance yourself: it never reads the OS clock, so the same sequence of
ticks produces the same timestamps on every run.

```rnx
import { Clock } from "@std/time";

let c = Clock.virtual(0);
c.tick(1000);
print(c.now().toMillis());
```

## What is inside

- `Duration`
- `VirtualClock`
- `Clock`

## When to use

Reach for @std/time in two shapes: Clock.mono for real monotonic durations, and Clock.virtual plus tick() for deterministic virtual time that tests can drive step by step.

It covers: Monotonic clock; VirtualClock control; Duration math.
