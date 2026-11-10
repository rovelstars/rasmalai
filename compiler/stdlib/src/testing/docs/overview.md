---
title: Testing
description: Keep benchmark work from being optimized away and assert allocation balance in tests.
section: Overview
order: 0
---

# Testing

@std/testing supports two jobs that happen outside normal program runs. The
first is benchmarking: an optimizing backend is free to delete a loop whose
result nobody reads, which turns a benchmark into a measurement of nothing.
`blackBox` hands a value to the runtime as an opaque value and hands it
straight back, so the work stays in the timing.

The second is leak checks. `liveCount` reads the runtime's live
object-plus-array count, so a test can snapshot it before and after the code
under test and assert that allocations balance.

The package has two free functions:

- `blackBox(val: Any): Any` returns the value unchanged through the
  backend's black-box barrier.
- `liveCount(): Int` returns the live allocation count at the call instant.

A first call:

```rnx
import { blackBox } from "@std/testing";

println!(blackBox(40 + 2));
```

This prints `42`.

## Tests are not in this module

`test fn` blocks, the `assert(cond, message)` builtin, and the `rnx test`
runner are part of the language and toolchain. You do not import @std/testing
to write a test. Import it when a test needs an allocation snapshot, or when
a benchmark needs a black-box fence.

`bench` blocks are timed by `rnx bench`. A bench block needs no `Main` and no
test registration. Put it in the project source and run the benchmark
command.

## Guides

- [Benchmarking](guides/benchmarking) covers `blackBox`, `bench` blocks,
  and the limits of the return type.
- [Leak checks](guides/leak-checks) covers `liveCount` and the shape of an
  allocation-balance test.

Start with [Getting started](getting-started) for a runnable program.
