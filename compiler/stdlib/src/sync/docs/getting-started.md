---
title: Getting started with Sync
description: Run a shared atomic counter and see registry state survive the whole process.
section: Getting started
order: 10
---

# Getting started with Sync

The module ships inside the compiler, so there is no install step. Import
the types you need:

```rnx
import { AtomicInt } from "@std/sync";
```

## Your first counter

Put this in a file named `counter.rnx`:

```rnx
import { AtomicInt } from "@std/sync";

let counter = AtomicInt.byId(7);
counter.set(40);
counter.fetchAdd(2);
println!(counter.get());
```

Run it:

```sh
rnx run counter.rnx
```

The output is `42`.

Several things happen in those four lines. `AtomicInt.byId(7)` opens the
cell registered under id 7, creating it at 0 because this is the first use
in the process. `set(40)` stores 40. `fetchAdd(2)` adds 2 and returns the
old value, 40, which the program ignores. `get()` then reads 42.

The cell lives until the process exits. Another part of the same program
that calls `AtomicInt.byId(7)` reaches the same value without any
registration step, including from another thread.

## Run it twice

Registry state is per process, not per file. Running `rnx run counter.rnx`
again starts a new process, so the id starts at 0 and the program prints 42
again. There is no cache on disk and no cleanup step.
