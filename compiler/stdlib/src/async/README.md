# @std/async

One-shot promises, settlement handles, chaining helpers, and combinators.

```rnx
import { Promise } from "@std/async";

async fn answer(): Promise<Int> {
    return 42;
}

fn Main(): Int {
    let value: Int = answer().wait().unwrapOr(0);
    println!(value);
    return 0;
}
```

Read the [overview](/docs/@std/async/overview) or open the [API reference](/docs/@std/async/api).
