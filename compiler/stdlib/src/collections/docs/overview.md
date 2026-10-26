# @std/collections

Insertion-ordered `Map` and `Set`.

`Map<K, V>` maps keys to values and `Set<T>` keeps distinct members;
`Set` is a `Map<T, Bool>` whose values are always `true`. Both iterate
with `for..in`, and both hand their items back in insertion order.

```rnx
import { Map } from "@std/collections";

let m = new Map<String, Int>();
m.set("a", 1);
print(m.get("a") ?? -1);
```

## What is inside

- `Map`
- `Set`

## When to use

Reach for @std/collections when keys need lookup or members need deduplication. Map preserves insertion order for keys() and values(); Set is built on top of it with the same guarantee.

It covers: Insertion-ordered Map; Distinct-member Set; for..in iteration.
