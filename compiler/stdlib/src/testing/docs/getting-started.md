---
title: Getting started with Testing
description: Run blackBox in a small program and see the value come back unchanged.
section: Getting started
order: 10
---

# Getting started with Testing

The module ships inside the compiler:

```rnx
import { blackBox } from "@std/testing";
```

## Copy this

Save this as `box.rnx`:

```rnx
import { blackBox } from "@std/testing";

let total = blackBox(40) + blackBox(2);
println!(total);
```

Run it:

```sh
rnx run box.rnx
```

The output is `42`. Each call returns the value it was given, so the sum is
the same as `40 + 2`. The point is not the number; it is that the optimizer
cannot fold the calls away. In a benchmark, the work that produced the value
stays in the measured block.

## What to expect

`blackBox` is an ordinary call at run time. It takes one value, returns it,
and does not print or change the value. The result is typed `Any`, so
arithmetic works but assigning it to a narrower binding does not.
[Benchmarking](guides/benchmarking) explains that rule and shows a bench
block.
