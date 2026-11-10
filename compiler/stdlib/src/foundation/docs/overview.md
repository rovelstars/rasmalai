---
title: "@std/foundation"
description: "The core value types, results, iteration, and array and string helpers shared by every Rasmalai target."
section: Overview
order: 0
---

# @std/foundation

`@std/foundation` declares the types that the rest of the standard library and the language rely on: `Int`, `Float`, `FastFloat`, `Bool`, `String`, `Char`, `Array`, `Map`, `Set`, `Error`, `Any`, `Void`, `GenRef`, `Result`, and the `Iterator` and `Iterable` interfaces with `ArrayIter`. It also holds the extension methods that give arrays and strings their search and transform helpers.

Most programs see these names through `@std/prelude` and never import this package. Import foundation directly when you write a target prelude or a library that must not depend on the host console and logging surface. The package imports only the native `runtime "bytes"` and `runtime "core"` shims, so it brings in no file, console, or logging code.

The declarations fall into three groups. Numeric and text values cover `Int`, `Float`, `FastFloat`, `Bool`, `String`, and `Char`. Containers cover `Array`, `Map`, and `Set`, with `Map` and `Set` kept as catalog stubs whose full bodies live in `@std/collections`. Outcomes and iteration cover `Result`, `Error`, `Any`, `Void`, `GenRef`, `Iterator`, `Iterable`, and `ArrayIter`.

`GenRef<T>` is the type for a back edge in a graph. `GenRef.of(x)` takes a reference without raising the owner's retain count, and `.get()` gives `null` once the owner is gone, so a back edge never keeps a graph alive. ARC owns, `GenRef` points back. Use `#[Allow(CyclicReference)]` on a field when you mean to keep a strong cycle and break it yourself. `Any` is the untyped escape hatch: nothing on an `Any` is resolved statically, so cast before use with `v as Int` or `v as String`. `Void` marks a function that only has effects, and `null` is the absent value itself.

```rnx
import { Result } from "@std/foundation";

fn parseSeven(text: String): Result<Int, String> {
    if text == "7" {
        return Result.Ok<Int, String>(7);
    }
    return Result.Err<Int, String>("not a number: " + text);
}

let xs = [4, 9, 16];
println!(parseSeven("7").unwrapOr(0));
println!(parseSeven("x").unwrapOr(0));
println!(xs.find((x: Int): Bool => x > 5) ?? -1);
println!(xs.join(","));
```

The first two lines print `7` and `0`. The third finds the first value above 5, so it prints `9`. The last joins the array in order and prints `4,9,16`.

## Guides

[Numbers and booleans](guides/numbers) covers `Float`, `FastFloat`, and `Bool`, including the conversion between the two float types. [Arrays](guides/arrays) covers the built-in sequence operations and the search, fold, and join helpers. [Strings](guides/strings) covers text measurement, slicing, and transforms. [Results](guides/results) explains `Result` and how to carry a failure value without throwing. [Iteration](guides/iteration) covers `Iterator`, `Iterable`, `ArrayIter`, and `for..in`.
