# @std/time

Monotonic and virtual time sources, with nanosecond durations and unit conversions.

```rnx
import { Clock } from "@std/time";

let clock = Clock.virtual(0);
clock.tick(2500000);
println!(clock.now().toMillis());
println!(clock.now().toSeconds());
```

Read the [overview](/docs/@std/time/overview) or open the [API reference](/docs/@std/time/api).
