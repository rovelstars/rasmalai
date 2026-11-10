---
title: Clocks
description: Measure real elapsed time with Clock.mono and drive deterministic time with Clock.virtual.
section: Clocks
order: 30
---

# Clocks

A clock answers "what time is it" for a program. The answer has to be cheap, and in a test it also has to be repeatable. Those goals conflict, so the package provides two clocks and no default.

## Monotonic time

`Clock.mono()` reads the hardware monotonic clock. The first call in a process returns a small value near zero, and later calls return nanoseconds since that first call. System date changes, NTP steps, and daylight-saving changes do not move it.

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

That prints `true 499500`, and `elapsed` is the nanoseconds spent in the loop. Use differences, never the absolute reading. Each call costs one clock read, so sample around a region of work instead of inside a loop.

The count is signed 64-bit nanoseconds, which covers about 292 years of process uptime without wrapping.

## Virtual time

`Clock.virtual(startNanos)` returns a `VirtualClock`. It stores one `Int` and moves only when you call `tick` or `reset`. Nothing blocks and nothing reads the OS, so a test can reach a 30-second deadline instantly.

```rnx
import { Clock } from "@std/time";

let clock = Clock.virtual(0);
let deadline = 30_000_000_000;
while clock.now().nanos < deadline {
    clock.tick(1_000_000_000);
}
println!(clock.now().toSeconds());
```

That prints `30.0`. The loop records 30 seconds of virtual time without spending them.

`tick` adds a signed delta, so a negative value moves the clock backward. `reset` replaces the current count with any value. A virtual clock is monotonic only if you pass non-negative deltas and never reset backwards; the type does not enforce that.

## Calendar time

The package does not report dates or wall-clock times. The exported `Date` name is a placeholder with no fields or methods. If a program needs a calendar date, it has to get one from another source.
