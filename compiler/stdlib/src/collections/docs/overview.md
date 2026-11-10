---
title: "@std/collections"
description: "Insertion-ordered Map and Set with hashable keys and stable iteration."
section: Overview
order: 0
---

# @std/collections

`@std/collections` provides two tables. `Map<K, V>` maps keys to values. `Set<T>` keeps distinct members. A set is a `Map<T, Bool>` whose values are always `true`, so the two types follow the same key rules and the same ordering rules.

Both iterate with `for..in`, and both hand their items back in insertion order. An entry keeps the position its key was first given. Setting a key that is already there leaves that position alone. Deleting a key and setting it again moves it to the end.

```rnx
import { Map, Set } from "@std/collections";

let ore = new Map<String, Int>();
ore.set("iron", 12);
ore.set("coal", 7);
println!(ore.get("iron") ?? 0);

let seen = new Set<String>();
seen.add("iron");
seen.add("iron");
println!(seen.len(), seen.values().join(", "));
```

The program prints `12`, then `1` and `iron`. Adding `iron` twice leaves one member.

`Map` and `Set` also appear in the prelude as catalog stubs. Import them from this package to get the table methods.

## Guides

[Maps](guides/maps) covers `set`, `get`, `has`, `delete`, `keys`, `values`, and iteration. [Sets](guides/sets) covers `add`, membership, deletion, and the member list. [Keys and ordering](guides/keys-and-order) covers which types can be keys, identity comparison, insertion order, ownership, and thread safety.
