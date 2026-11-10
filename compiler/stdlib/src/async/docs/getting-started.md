# Getting started with @std/async

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Promise } from "@std/async";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
async fn answer(): Promise<Int> {
    return 42;
}

fn Main(): Int {
    println!(answer().wait().unwrapOr(0));
    return 0;
}
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
