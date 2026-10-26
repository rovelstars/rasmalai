# Getting started with @std/math

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Math, Vec2 } from "@std/math";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { Math, Vec2 } from "@std/math";

let v = new Vec2(3.0, 4.0);
print(v.length());
print(Math.sin(Math.PI() / 2.0));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
