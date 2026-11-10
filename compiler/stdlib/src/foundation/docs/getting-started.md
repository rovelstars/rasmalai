---
title: "Getting started with @std/foundation"
description: "Import the foundation catalog and run a first program that uses Result, Array, and ArrayIter."
section: Getting started
order: 10
---

# Getting started with @std/foundation

There is no install step. The package ships inside the compiler, and the stock prelude re-exports its names, so `Result`, `Array`, `String`, and the rest are already in scope in ordinary programs.

Import the package explicitly when you write a prelude or a library that must work without the stock prelude's console and logging surface:

```rnx
import { Result, ArrayIter } from "@std/foundation";
```

## Copy this

Save it as the entry file of a project and run `rnx run`, or start a project with `rnx init` and replace `src/main.rnx`:

```rnx
import { Result, ArrayIter } from "@std/foundation";

fn firstEven(values: Array<Int>): Result<Int, String> {
    let found = values.find((n: Int): Bool => n % 2 == 0);
    if found == null {
        return Result.Err<Int, String>("no even value");
    }
    return Result.Ok<Int, String>(found as Int);
}

fn Main(): Int {
    let it = new ArrayIter<Int>([2, 4]);
    println!(it.next() ?? 0);
    println!(firstEven([1, 2, 3]).unwrapOr(0));
    return 0;
}
```

The program prints `2` twice and exits with status 0. `ArrayIter.next()` returns `2`, and `firstEven` finds `2` in the array. The `?? 0` on the iterator handles the `null` that `next()` returns when the sequence runs out.

## Use it in a prelude

A custom prelude re-exports the foundation names and adds only the types its target provides. Here is the foundation portion:

```rnx
export {
    Int, Float, FastFloat, Bool, String, Char,
    Array, Map, Set, Error, Any, Void, GenRef,
    Result, Iterator, Iterable, ArrayIter
} from "@std/foundation";
```

See [Custom preludes](/docs/@std/prelude/guides/custom-prelude) for the full catalog requirement. In practice, a target starts from the stock prelude's export list and removes the packages it cannot build.

## What to run

```sh
rnx run
```

`rnx check` also accepts the file and reports type errors without running it.
