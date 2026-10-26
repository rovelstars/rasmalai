# @std/sync

Atomics, channels, locks, and barriers for threads.

Every type here is a thin handle over a number. A handle carries nothing
but a registry id, so passing a handle between threads shares the same
underlying cell, queue, or lock, and two handles built from the same id
always name the same object. The registries are separate per type: an
`AtomicInt` and a `Mutex` that both use id 5 are unrelated objects.

```rnx
import { AtomicInt } from "@std/sync";

let a = AtomicInt.byId(101);
print(a.get());
```

## What is inside

- `AtomicInt`
- `Channel`
- `Mutex`
- `RwLock`
- `Condvar`
- `Barrier`
- `Thread`

## When to use

Reach for @std/sync whenever threads share anything. Every primitive is constructed byId so separate threads open the same cell — the only legal cross-thread sharing path in the language.

It covers: AtomicInt counters; Mutex/RwLock guards; Channel/Barrier coordination.
