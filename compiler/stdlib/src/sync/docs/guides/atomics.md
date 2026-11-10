---
title: Atomic counters and CAS
description: Use AtomicInt for race-free counters and compare-and-swap updates.
section: Atomics
order: 30
---

# Atomic counters and CAS

A shared value updated in separate read and write steps is not safe from two
threads. Both threads can read the old value, add one, and write back, so
one increment disappears. `AtomicInt` keeps the read and the write together,
in one indivisible step.

## Count without losing updates

`fetchAdd` returns the value that was there before the add. The cell is
updated before the call returns, so other threads see the new value.

```rnx
import { AtomicInt, Thread } from "@std/sync";

fn bump(): Int {
    let counter = AtomicInt.byId(21);
    let i = 0;
    while i < 100 {
        counter.fetchAdd(1);
        i = i + 1;
    }
    return 0;
}

let counter = AtomicInt.byId(21);
counter.set(0);

let first = Thread.spawn(bump);
let second = Thread.spawn(bump);
first.join();
second.join();
println!(counter.get());
```

The two threads add 100 each, and the program prints `200`. If each increment
were a separate read and write, the two sequences could interleave and lose
one of the updates.

`fetchAdd` accepts a negative delta to subtract. The return value is still
the value before the operation, which makes it useful for handing out
unique numbers from a counter.

## Swap only on a match

`cas(expected, newVal)` compares the cell with `expected` and stores
`newVal` only when they match. It returns `true` when the swap happened.

```rnx
import { AtomicInt } from "@std/sync";

let slot = AtomicInt.byId(8);
slot.set(1);
println!(slot.cas(1, 2));
println!(slot.cas(1, 3));
println!(slot.get());
```

This prints `true`, `false`, and `2`. The first call finds 1 and stores 2.
The second still expects 1, but the cell now holds 2, so it leaves the cell
alone and reports false.

Use `cas` when a thread must only write if nobody else moved the value
first. When it returns false, read the cell again and decide what to do with
the current value.

## What is atomic

Only the cell is atomic. An ordinary field of an object is not covered by
`AtomicInt`, even when a thread holds a handle for the object. If two pieces
of data must change together, put them behind a `Mutex` from
[Locks and condition variables](locks).

Every operation is sequentially consistent. A value stored with `set` is
visible to the next `get` from any thread, and `fetchAdd` and `cas` do the
read and the write without a lock in between.

## Related API

`AtomicInt.byId` opens the cell, `get` and `set` read and replace the value,
`fetchAdd` adds, and `cas` swaps on a match. The [overview](../overview)
lists the other shared types.
