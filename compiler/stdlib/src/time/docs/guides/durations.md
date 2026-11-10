---
title: Durations
description: Convert nanosecond counts to milliseconds and seconds without losing track of precision.
section: Durations
order: 20
---

# Durations

`Duration` stores one signed 64-bit nanosecond count in `nanos`. There is no validation and no hidden unit change, so you always know what the number means. Convert only when you print, compare with a wall-clock unit, or pass a value to another API.

```rnx
import { Duration } from "@std/time";

let d = new Duration(2_500_000);
println!(d.nanos, d.toMillis(), d.toSeconds());
```

That prints `2500000 2 0.0025`. Integer division in `toMillis()` truncates toward zero, so the same call on `-1_500_000` returns `-1`, not `-2`.

## Working with a budget

Keep arithmetic in nanoseconds and convert at the end. This avoids rounding each step.

```rnx
import { Duration } from "@std/time";

fn remaining(budget: Duration, used: Duration): Duration {
    return new Duration(budget.nanos - used.nanos);
}

let budget = new Duration(2_000_000_000);
let used = new Duration(750_000_000);
println!(remaining(budget, used).toMillis());
```

That prints `1250`.

`toSeconds()` converts through `Float`, so counts past 2^53 nanoseconds (about 104 days) lose the last digits. Use `nanos` for exact comparisons and arithmetic. Negative durations are valid; nothing in this type prevents them. A `Duration` also does not know which clock produced it, so compare durations from the same source.
