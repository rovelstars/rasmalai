# Getting started with @std/sync

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { AtomicInt } from "@std/sync";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { AtomicInt } from "@std/sync";

let a = AtomicInt.byId(101);
print(a.get());
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
