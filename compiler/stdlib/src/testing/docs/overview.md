# @std/testing

Benchmark helper that keeps measured work from being optimized away.

`bench` blocks are timed by `rnx bench`. An optimizing backend is free to
delete a loop whose result nobody reads, which turns a benchmark into a
measurement of nothing. `blackBox` hands the value to the runtime as an
opaque value and hands it straight back, so the work stays in the timing.

```rnx
import { blackBox } from "@std/testing";

print(blackBox(40 + 2));
```

## What is inside

- free functions: `blackBox()`

## When to use

Reach for @std/testing inside benchmarks: blackBox keeps the optimizer from deleting the code being measured. Tests themselves live in test fn blocks and run under rnx test.

It covers: blackBox fences; Benchmark support.
