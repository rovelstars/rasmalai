---
title: Clocks and durations
description: Monotonic and virtual time sources, with nanosecond durations and unit conversions.
section: Overview
order: 0
---

# Clocks and durations

`@std/time` answers two questions: how long did this work take, and what time does a test think it is. It does not report calendar dates. A `Duration` counts nanoseconds from whatever epoch its source uses, so it measures elapsed time and nothing else.

There are two clocks. `Clock.mono()` reads the hardware monotonic clock. It moves forward, ignores system clock changes, and returns nanoseconds since the first `Clock.mono()` call in the process. Use it for timeouts, retries, and profiling. `Clock.virtual(startNanos)` builds a `VirtualClock` that never reads the OS. You advance it with `tick`, overwrite it with `reset`, and read it with `now`. The same calls produce the same timestamps on every run.

`Duration` wraps one signed `Int` of nanoseconds. `toMillis()` truncates toward zero, and `toSeconds()` returns a `Float` with the fraction kept. `VirtualClock` exposes its current count as `currentNanos`. `tick` adds a delta and `reset` replaces the count, so a negative delta or a backwards reset can move a virtual clock back.

```rnx
import { Clock, Duration } from "@std/time";

let start = Clock.mono();
let step = new Duration(2_500_000);
println!(step.toMillis(), step.toSeconds());
let virtual = Clock.virtual(start.nanos);
virtual.tick(60_000_000_000);
println!(virtual.now().nanos - start.nanos);
```

That prints `2 0.0025` and `60000000000`.

Calendar time is not part of the package. The exported `Date` name is a placeholder with no fields or methods.

## Guides

[Getting started](getting-started) has a first program with a virtual clock. [Durations](guides/durations) covers unit conversion and exact arithmetic. [Clocks](guides/clocks) explains when to use the monotonic clock and when to drive a virtual one.
