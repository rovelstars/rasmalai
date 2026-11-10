---
title: Leak checks
description: Snapshot live allocations with liveCount and assert balance around the code under test.
section: Leak checks
order: 30
---

# Leak checks

A test can prove that a path returns a value and still miss a leak. If the
code allocates ten objects and releases nine, the result looks correct while
the process keeps growing. `liveCount` reports the runtime's live
object-plus-array count, so a test can compare the count before and after
the code under test.

## Read the count

`liveCount()` returns the live allocation count at the call instant. It is
the same number the toolchain uses to balance allocations in its own test
suite.

```rnx
import { liveCount } from "@std/testing";

let before = liveCount();
let values = [1, 2, 3];
println!(values.length());
println!("live objects:", liveCount() - before);
```

The first line prints `3`. The second prints the difference the array added
at that moment. The number depends on the surrounding code and the backend,
so use it for balance checks, not as a fixed constant.

## Assert balance in a test

Snapshot before the work, run the work, then compare. Keep the work in a
function so its locals leave scope before the second snapshot:

```rnx
import { liveCount, blackBox } from "@std/testing";

fn useArray() {
    let values = [1, 2, 3];
    blackBox(values);
}

test fn allocationBalance() {
    let before = liveCount();
    useArray();
    assert(liveCount() <= before, "temporary array released");
}
```

Run it with:

```sh
rnx test
```

The runner prints one line for the test and a summary. `assert` takes a
`Bool` condition and a `String` message. On false it prints the message and
marks that test failed without aborting the rest of the run.

`blackBox` keeps the optimizer from deleting the array before the snapshot.
It is not required for every leak check, but it prevents the compiler from
removing work whose only purpose is to allocate.

## Where to use it

Use `liveCount` in tests, not in program logic. User code must never branch
on the allocation count for control flow. A count that changes with the
backend or the host would make the program behave differently for reasons
that have nothing to do with the data.

Take the snapshot at the narrowest point that covers the work. A count taken
at startup includes unrelated allocations and makes the comparison noisy.

## Related API

`liveCount(): Int` from @std/testing, plus `test fn` blocks and the
`assert(cond, message)` builtin from the language. [Benchmarking](benchmarking)
covers `blackBox`.
