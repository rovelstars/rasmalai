---
title: "Ambient names"
description: "What the prelude puts in scope, what is a compiler builtin, and how shadowing works."
section: Names
order: 20
---

# Ambient names

Every file starts with a set of names already resolved. The compiler needs those names to type common code without imports, and a target needs to be able to replace the set when the platform has no host console or file system. This page covers what is ambient and what happens when you redeclare one of the names.

## The catalog

The prelude re-exports these foundation names:

`Int`, `Float`, `FastFloat`, `Bool`, `String`, `Char`, `Array`, `Map`, `Set`, `Error`, `Any`, `Void`, `GenRef`, `Result`, `Iterator`, `Iterable`, `ArrayIter`.

It adds `Date`, `Promise`, `PromiseResult`, `PromiseWithResolvers`, and `RnxHost`. It also exports the output macros from `@std/io` and the log macros from `@std/log`. `IO`, `Thread`, and `ThreadPool` are catalog stubs declared in the prelude, so the compiler can lower operations such as thread creation without a library import.

`Pointer` and `Address` stay compiler builtins. `Null` is a language builtin with no declaration, so a prelude does not need to declare it. `assert` and `typeOf` are compiler intrinsics rather than prelude exports, so they also stay available without an import and are not part of the catalog a replacement must declare.

## Shadowing

A declaration in your file wins over the prelude name. So does an import. This is normal and useful when a package provides a richer version of a catalog name:

```rnx
import { Map } from "@std/collections";

let ore = new Map<String, Int>();
ore.set("iron", 12);
println!(ore.get("iron") ?? 0);
```

The ambient `Map` is only a catalog stub. Importing `Map` from `@std/collections` brings the table implementation with `set`, `get`, `has`, and the rest.

You can shadow a type name with your own declaration too:

```rnx
class Date {
    let label: String;

    init(label: String) {
        this.label = label;
    }
}

println!(new Date("today").label);
```

The local `Date` wins, and the program prints `today`. The re-exported `Date` from `@std/time` is no longer visible under that name in this file.

## What to reach for

Use the ambient names for ordinary code. Import `Map` and `Set` from `@std/collections` when you need their methods. Import a type explicitly when your own declaration or another package has shadowed it, or when you want the reader to see where a name comes from.
