# Getting started with @std/env

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Env } from "@std/env";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { Env } from "@std/env";

Env.set("GREETING", "hi");
print(Env.get("GREETING"));
print(Env.args().len() > 0);
print(Env.cwd().len() > 0);
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
