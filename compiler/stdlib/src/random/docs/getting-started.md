# Getting started with @std/random

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Rng } from "@std/random";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { Rng } from "@std/random";

let a = Rng.seeded(7);
let b = Rng.seeded(7);
print(a.next() == b.next());
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
