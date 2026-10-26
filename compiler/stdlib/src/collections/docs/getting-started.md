# Getting started with @std/collections

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Map, Set } from "@std/collections";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { Map, Set } from "@std/collections";

let m = new Map<String, Int>();
m.set("a", 1);
m.set("b", 2);
print(m.keys().join(","));
print(m.get("a") ?? -1);
let s = new Set<String>();
s.add("x");
print(s.has("x"));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
