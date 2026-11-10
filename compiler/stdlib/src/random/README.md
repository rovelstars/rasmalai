# @std/random

Reproducible seeded streams for tests and OS entropy for security-sensitive values.

```rnx
import { Rng } from "@std/random";

let a = Rng.seeded(7);
let b = Rng.seeded(7);
let x = a.next();
let y = b.next();
println!(x == y);
println!(a.int(1, 10), b.int(1, 10));
```

Read the [overview](/docs/@std/random/overview) or open the [API reference](/docs/@std/random/api).
