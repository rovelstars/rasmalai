# @std/testing

Keep benchmark work from being optimized away and assert allocation balance in tests.

```rnx
import { blackBox } from "@std/testing";

let total = blackBox(40) + blackBox(2);
println!(total);
```

Read the [overview](/docs/@std/testing/overview) or open the [API reference](/docs/@std/testing/api).
