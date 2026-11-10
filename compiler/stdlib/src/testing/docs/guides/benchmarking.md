---
title: Benchmarking
description: Time work with rnx bench and keep the optimizer from deleting the measured code.
section: Benchmarking
order: 20
---

# Benchmarking

A benchmark measures work. If the backend can prove that nobody reads the
result, it may delete the loop, and the benchmark then measures an empty
block. `blackBox` prevents that by passing the value through an opaque
boundary that the optimizer cannot see through.

## Time a block

`bench` blocks are timed by `rnx bench`. They live in the project source, and
a bench block needs no `Main` and no registration:

```rnx
import { blackBox } from "@std/testing";

bench "sum" {
    let total = 0;
    let i = 0;
    while i < 1000 {
        total = total + i;
        i = i + 1;
    }
    blackBox(total);
}
```

Put the block in `src/main.rnx` of a project with a `Project.config`, then
run:

```sh
rnx bench
```

The runner reports one line per block with a per-iteration time in
nanoseconds and an iteration count. The numbers differ by machine and by
build. The `blackBox(total)` call is what keeps the summation in the
measurement.

`rnx bench` builds with LLVM release by default, accepts a filter, and can
select a backend. Use `--filter` to run only blocks whose names match a
substring.

## The return type

`blackBox` takes exactly one argument. Two or more arguments are a compile
error reported as `std.testing.blackBox takes 1 args`, and a tuple argument
is refused as `tuple value cannot be used in blackBox`.

The result is typed `Any`, but the runtime type survives. `typeOf` still
reports `Int`, `Float`, `String`, and the rest, and arithmetic on the result
works:

```rnx
import { blackBox } from "@std/testing";

let total = blackBox(40) + blackBox(2);
println!(total);
let value: Any = blackBox("hello");
println!(typeOf(value));
```

This prints `42` and `String`. Assigning the result to a narrower binding is
a type error: `let n: Int = blackBox(7)` reports `cannot assign value of type
Any to variable n of type Int`. Pass the result straight on, or declare the
binding as `Any`.

## What blackBox does not do

It does not print or change the value. It only makes the value opaque to
the optimizer for the trip through the call. Wrapping a value that is not
used by the measured work does not make that work measurable; put the call
on the result you actually want counted.

## Related API

`blackBox(val: Any): Any` from @std/testing. The `bench` block and
`rnx bench` are toolchain features. [Leak checks](leak-checks) covers the
other function in this package.
