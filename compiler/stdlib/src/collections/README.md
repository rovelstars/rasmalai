# @std/collections

Insertion-ordered Map and Set with hashable keys and stable iteration.

```rnx
import { Map, Set } from "@std/collections";

let m = new Map<String, Int>();
m.set("a", 1);
m.set("b", 2);
println!(m.keys().join(","));
println!(m.get("a") ?? -1);

let s = new Set<String>();
s.add("x");
println!(s.has("x"));
```

Read the [overview](/docs/@std/collections/overview) or open the [API reference](/docs/@std/collections/api).
