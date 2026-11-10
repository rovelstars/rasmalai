---
title: "Getting started with @std/collections"
description: "Import Map and Set and run a first table program."
section: Getting started
order: 10
---

# Getting started with @std/collections

There is no install step. The package ships inside the compiler. Import the table types you need:

```rnx
import { Map, Set } from "@std/collections";
```

## Copy this

Save it as the entry file of a project and run `rnx run`, or start with `rnx init` and replace `src/main.rnx`:

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

## What to expect

The program prints `a,b`, then `1`, then `true`. `keys()` returns the keys in insertion order. A missing key reads as `null`, so `?? -1` supplies a default. `has` answers whether a member is present.
