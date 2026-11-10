---
title: Locks and condition variables
description: Guard shared state with Mutex and RwLock, and park threads with Condvar.
section: Locks
order: 50
---

# Locks and condition variables

An atomic cell protects one integer. Shared structures such as arrays, maps,
or several fields that must change together need a lock. A lock makes a
group of statements run as one critical section.

## Mutual exclusion

One thread holds a `Mutex` at a time. Others park in `lock` until it is
released.

```rnx
import { Mutex } from "@std/sync";

let m = Mutex.byId(31);
m.lock();
m.unlock();
println!(m.tryLock());
m.unlock();
```

This prints `true`. `tryLock` takes the lock only when it is free and
reports the outcome instead of waiting. Release it with `unlock` only when
the call returned true.

The lock is not reentrant and carries no owner. A thread that already holds
it and calls `lock` again parks forever. `unlock` on a lock nobody holds
aborts the program. Every `lock` needs exactly one matching `unlock` on
each path out of the critical section, including error paths.

Keep critical sections short. The lock is a coordination point, so holding
it while doing slow I/O serializes every other thread that needs it.

## Readers and writers

A `RwLock` lets many readers hold the lock at once, or one writer hold it
exclusively, never both. It fits a structure that is read far more often
than it changes.

```rnx
import { RwLock } from "@std/sync";

let r = RwLock.byId(41);
r.writeLock();
println!("read while writer:", r.tryReadLock());
println!("second writer:", r.tryWriteLock());
r.writeUnlock();
println!("read when free:", r.tryReadLock());
r.readUnlock();
```

This prints `read while writer: false`, `second writer: false`, and
`read when free: true`. Readers never wait for each other. A writer waits
for the last reader as well as for any other writer, so keep read sections
short when writers need to make progress.

There is no way to upgrade a held read lock to a write lock. The thread
would wait for its own read access to end. Release the read lock first.

The lock carries no owner and is not reentrant. `readUnlock` with no reader
and `writeUnlock` with no writer both abort the program.

## Wait for a condition

A `Condvar` carries no state. It only parks and wakes threads, so the thing
being waited on lives somewhere else, normally behind the mutex handed to
`wait`. `wait` and `waitTimeout` must be called with that mutex held. They
release the mutex while the thread sleeps and take it again before
returning, so no other thread can slip in between the test and the sleep.

```rnx
import { Condvar, Mutex } from "@std/sync";

let m = Mutex.byId(51);
let c = Condvar.byId(51);
m.lock();
println!("woken:", c.waitTimeout(m, 30));
m.unlock();
c.notifyAll();
println!("notify with no waiters is a no-op");
```

This prints `woken: false`, because no thread notified the condvar within
30 milliseconds. `notifyAll` then does nothing because there are no waiters.

The usual shape is a loop: take the mutex, test the condition, and wait
while the test still says not ready. A notify that arrives before the sleep
is missed, so re-test after every wake.

`waitTimeout` is the bounded version. It returns true when a notify woke
the thread and false when the timeout elapsed. A negative timeout is treated
as 0. Use it when a poll loop must not block forever.

`notifyOne` wakes one waiter and `notifyAll` wakes every waiter. Both are
no-ops when nobody is waiting. Calling `wait` or `waitTimeout` without
holding the mutex aborts the program.

## Related API

`Mutex.byId`, `lock`, `unlock`, and `tryLock`; `RwLock.byId`, `readLock`,
`readUnlock`, `writeLock`, `writeUnlock`, `tryReadLock`, and `tryWriteLock`;
`Condvar.byId`, `wait`, `waitTimeout`, `notifyOne`, and `notifyAll`.
