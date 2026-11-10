---
title: "Maps"
description: "Store, read, overwrite, delete, and iterate Map entries."
section: Maps
order: 20
---

# Maps

A `Map<K, V>` answers one question quickly: what value belongs to this key? Build one with `new Map<K, V>()`, fill it with `set`, and read it back with `get` or `has`. Entries keep insertion order, so the same map can also drive a stable list of keys or values.

## Store and read

```rnx
import { Map } from "@std/collections";

let ore = new Map<String, Int>();
ore.set("iron", 12);
ore.set("coal", 7);
println!(ore.get("iron") ?? 0);
println!(ore.get("gold") ?? 0);
println!(ore.len());
```

The lines print `12`, `0`, and `2`. A missing key reads as `null` rather than aborting, so `??` and `?.` supply defaults. `len()` counts the live entries.

`has(key)` tells you whether a key is present. It is the only way to tell an absent key from a key whose stored value is `null`.

## Overwrite and delete

```rnx
let m = new Map<String, Int>();
m.set("a", 1);
m.set("b", 2);
m.set("a", 10);
println!(m.keys().join(","));
println!(m.values().join(","));
println!(m.len());
```

The lines print `a,b`, `10,2`, and `2`. Overwriting `a` keeps its first position and replaces the value. The count does not change.

```rnx
let m = new Map<String, Int>();
m.set("a", 1);
m.set("b", 2);
println!(m.delete("a"), m.len());
println!(m.delete("a"));
println!(m.keys().join(","));
```

That prints `true 1`, then `false`, then `b`. `delete` returns whether an entry was removed. Deleting and setting the same key again appends it at the end.

`clear()` drops every entry at once and leaves the map usable.

## Read the pieces

`keys()` and `values()` return fresh arrays in insertion order, aligned index by index. Writing to a returned array does not change the map. `for..in` walks the keys:

```rnx
let m = new Map<String, Int>();
m.set("iron", 12);
m.set("coal", 7);
for name in m {
    println!(name, m.get(name) ?? 0);
}
```

The loop prints `iron 12` and then `coal 7`. Each loop calls `iterator()` for a fresh pass, so two loops do not share a cursor.

## A real case

```rnx
let counts = new Map<String, Int>();
let words = ["iron", "coal", "iron"];

for word in words {
    let current = counts.get(word) ?? 0;
    counts.set(word, current + 1);
}

for word in counts {
    println!(word, counts.get(word) ?? 0);
}
```

The loop prints `iron 2` and `coal 1`. Reading a missing key as `0` keeps the counting code short, and the map preserves the order in which words first appeared.
