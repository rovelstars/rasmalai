---
title: "Results"
description: "Carry a success payload or an error value with Result and its helpers."
section: Results
order: 50
---

# Results

`Result<T, E>` answers one question: did the operation produce a payload, or did it fail with an error value? `Ok` carries the payload. `Err` carries the error. The prelude merges the type into every program as `std.prelude.Result`, so ordinary code needs no import.

Use a result when a failure is part of the contract and the caller should decide what to do. `throw` is for failures the caller cannot handle.

## Build and inspect

```rnx
fn parse(text: String): Result<Int, String> {
    if text == "7" {
        return Result.Ok<Int, String>(7);
    }
    return Result.Err<Int, String>("not a number: " + text);
}

println!(parse("7").isOk());
println!(parse("x").isErr());
println!(parse("x").unwrapOr(0));
```

The lines print `true`, `true`, and `0`. `isOk` and `isErr` inspect the variant. `unwrapOr(fallback)` returns the payload for an `Ok` and the fallback for an `Err`.

`unwrap()` returns the payload too, but an `Err` makes it throw `unwrap of Err: <reason>`. Reach for `unwrapOr` when a default is reasonable and `unwrap` only when a failure means the program cannot continue.

## Transform the payload

```rnx
fn parse(text: String): Result<Int, String> {
    if text == "7" {
        return Result.Ok<Int, String>(7);
    }
    return Result.Err<Int, String>("not a number: " + text);
}

println!(parse("7").map((v: Int): Int => v * 2).unwrapOr(0));
println!(parse("x").map((v: Int): Int => v * 2).unwrapOr(0));
```

The first line prints `14`. The second prints `0`, because `map` runs the closure only on the `Ok` payload and passes an `Err` through untouched.

## A real case

```rnx
fn findEven(values: Array<Int>): Result<Int, String> {
    let found = values.find((x: Int): Bool => x % 2 == 0);
    if found == null {
        return Result.Err<Int, String>("no even value");
    }
    return Result.Ok<Int, String>(found as Int);
}

println!(findEven([1, 3, 4]).unwrapOr(0));
println!(findEven([1, 3, 5]).unwrapOr(0));
println!(findEven([1, 3, 5]).isErr());
```

The calls print `4`, `0`, and `true`. `find` returns `Int?`, so the `null` check narrows it before the cast to `Int`, and the empty answer becomes an `Err` with a reason the caller can read.
