---
title: Threads and barriers
description: Start native threads with Thread.spawn, collect results with join, and release a group with Barrier.
section: Threads
order: 60
---

# Threads and barriers

A program that does two independent jobs can run them at the same time.
`Thread.spawn` starts a native thread, and `join` waits for it and returns
the value the thread produced. `Barrier` handles the other direction: several
threads reach a point and wait until all of them have arrived.

## Start a thread

`spawn` takes a function with no parameters. The function returns an `Int`,
`Bool`, `Float`, `String`, or null. The checker rejects any other return
type at the call site.

```rnx
import { Thread } from "@std/sync";

fn worker(): Int {
    return 40 + 2;
}

let workers: Array<Thread> = [];
workers.push(Thread.spawn(worker));
let result = workers[0].join();
println!(result.unwrapOr(0));
```

This prints `42`. `join` waits up to 30 seconds, merges the thread's output
into the joiner's output, and consumes the handle. Joining the same handle a
second time reports a dead thread instead of returning the value again.

The result is a `Result`. `unwrapOr(0)` gives the value on success and the
fallback on failure. Keep the handle if you need the result later; a bare
`Thread.spawn(worker)` expression discards it and leaves the thread
unjoined.

## Share data through ids

A spawned body runs on its own native thread. It does not see local
variables from the parent, and ordinary objects are not shared. Pass only a
registry id or another value that is safe to copy, then open a handle on the
other side.

```rnx
import { AtomicInt, Thread } from "@std/sync";

fn bump(): Int {
    let counter = AtomicInt.byId(21);
    counter.fetchAdd(1);
    return 0;
}

let counter = AtomicInt.byId(21);
counter.set(0);
let worker = Thread.spawn(bump);
worker.join();
println!(counter.get());
```

This prints `1`. The worker builds its own handle for id 21 and updates the
same cell the parent reads.

## Failure and timeouts

A spawned function that throws is not a build error. The throw is captured
and surfaces as an `Err` from `join`. `join` also reports a panic, a deadline
of 30 seconds, a reused dead handle, and a thread that never started. The
message names the cause.

A thread that has not finished after 30 seconds yields
`Err(thread N join timed out after 30s)`. Since `join` consumes the handle,
there is nothing left to wait on after a timeout.

## Meet at a barrier

A `Barrier` blocks every party in `wait` until the count-th party arrives.
It then releases the whole generation, resets itself, and returns true for
exactly one leader. Use the leader flag to elect one thread for per
generation work such as merging results.

```rnx
import { Barrier } from "@std/sync";

let solo = Barrier.byId(61, 1);
println!("first generation:", solo.wait());
println!("gate resets:", solo.wait());
```

This prints `true` and `true`. A barrier of one never blocks and reports the
leader on every call, which is a convenient way to exercise the gate.

For a real gate, every thread must build its handle with the same count. The
first party to arrive sets the threshold. A thread that exits early leaves
the others parked forever, and one thread calling `wait` on a barrier of two
or more never returns.

`Barrier.byId` throws when the count is 0 or negative. That throw runs in a
function that is not declared `throws`, so an enclosing `try` does not
intercept it: the program stops with `Uncaught exception` and exit status 1.
Validate a count that comes from outside the program before you build the
barrier.

## Related API

`Thread.spawn` starts a thread and `Thread.join` waits for it. `Barrier.byId`
takes an id and a party count, `wait` blocks until the generation trips, and
`count` reports the threshold. `Thread.byId` is not callable from Rasmalai
source because the compiler claims every `Thread.<name>` call; hold the
handle that `spawn` returns.
