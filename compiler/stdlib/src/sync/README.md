# @std/sync

Share atomics, channels, locks, and threads through process-wide registry ids.

```rnx
import { AtomicInt } from "@std/sync";

let counter = AtomicInt.byId(7);
counter.set(40);
counter.fetchAdd(2);
println!(counter.get());
```

Read the [overview](/docs/@std/sync/overview) or open the [API reference](/docs/@std/sync/api).
