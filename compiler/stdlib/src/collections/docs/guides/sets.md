---
title: "Sets"
description: "Keep distinct members, test membership, and list a Set in insertion order."
section: Sets
order: 30
---

# Sets

A `Set<T>` keeps one copy of each member. It is backed by a `Map<T, Bool>`, so members follow the same rules as map keys and come back in insertion order. Adding a member that is already there keeps one copy and leaves its position alone.

## Add and test

```rnx
import { Set } from "@std/collections";

let seen = new Set<String>();
seen.add("iron");
seen.add("coal");
seen.add("iron");
println!(seen.len(), seen.has("coal"));
println!(seen.values().join(", "));
```

The first line prints `2 true`. The second prints `iron, coal`. The duplicate `iron` does not increase the count or move the member.

`values()` is the member list, because the member is the key. It returns a fresh array, so writing to it does not change the set.

## Delete and clear

```rnx
let s = new Set<String>();
s.add("ore");
s.add("coal");
println!(s.delete("ore"), s.len());
println!(s.values().join(", "));
```

The first line prints `true 1`. The second prints `coal`. `delete` returns whether a member was removed. A member added again after deletion lands at the end.

`clear()` drops every member at once and leaves the set usable.

## Iterate

```rnx
let s = new Set<String>();
s.add("iron");
s.add("coal");
for member in s {
    println!(member);
}
```

The loop prints `iron` and then `coal`. `iterator()` starts a fresh pass each time, so two loops over the same set both see every member.

## A real case

```rnx
fn uniqueTags(tags: Array<String>): Array<String> {
    let seen = new Set<String>();
    let out: Array<String> = [];
    let i = 0;
    while i < tags.length() {
        let tag = tags[i];
        if seen.has(tag) == false {
            seen.add(tag);
            out.push(tag);
        }
        i = i + 1;
    }
    return out;
}

println!(uniqueTags(["red", "blue", "red", "green"]).join(", "));
```

The program prints `red, blue, green`. The set answers membership without a second scan, and the output array keeps the order the tags first appeared in.
