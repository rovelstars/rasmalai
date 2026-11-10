---
title: "@std/prelude"
description: "The names every Rasmalai compilation unit sees without an import, and how a target replaces them."
section: Overview
order: 0
---

# @std/prelude

The prelude is the set of names that every compilation unit sees without an import. It contains the foundation types, the date and promise types, the output and log macros, and a few catalog stubs that the compiler lowers to runtime calls.

The foundation names come from `@std/foundation`: `Int`, `Float`, `FastFloat`, `Bool`, `String`, `Char`, `Array`, `Map`, `Set`, `Error`, `Any`, `Void`, `GenRef`, `Result`, `Iterator`, `Iterable`, and `ArrayIter`, with the array and string extension methods. The prelude re-exports them and adds `Date` from `@std/time`, `Promise`, `PromiseResult`, and `PromiseWithResolvers` from `@std/async`, `RnxHost` from `@std/env`, the output macros from `@std/io`, and the log macros from `@std/log`. `IO`, `Thread`, and `ThreadPool` are catalog stubs declared here, so the compiler can lower their operations without a separate import.

An explicit declaration or an import always shadows a prelude name. That is how a program can use `Map` from `@std/collections` instead of the ambient catalog stub.

```rnx
println!("no import needed");
let found = [1, 2, 3].find((n: Int): Bool => n > 1);
println!(found ?? -1);
```

The program prints `no import needed` and then `2`. `println!`, `Array`, and the `find` extension method all come from the prelude.

`Map` and `Set` are catalog stubs in this package. Their methods live in `@std/collections`. Import those two names from that package when you need the real tables:

```rnx
import { Map } from "@std/collections";

let ore = new Map<String, Int>();
ore.set("iron", 12);
println!(ore.get("iron") ?? 0);
```

## Guides

[Ambient names](guides/ambient-names) explains what is in scope, how shadowing works, and which names are compiler builtins rather than prelude declarations. [Custom preludes](guides/custom-prelude) covers selecting a different prelude for a target and the names a replacement must provide.
