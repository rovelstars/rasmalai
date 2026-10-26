# Getting started with @std/simd

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Vec4f } from "@std/simd";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { Vec4f } from "@std/simd";

let v = Vec4f.splat(2.0);
print(v.x() == 2.0, v.w() == 2.0);
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
