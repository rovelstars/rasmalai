# Getting started with @std/time

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Clock } from "@std/time";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { Clock } from "@std/time";

let c = Clock.virtual(0);
c.tick(2500000);
print(c.now().toMillis());
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
