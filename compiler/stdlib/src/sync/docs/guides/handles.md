---
title: Handles and registry ids
description: How byId shares one cell, queue, or lock between threads, and how to choose ids safely.
section: Sharing
order: 20
---

# Handles and registry ids

The problem this page solves is sharing. Threads do not share ordinary
objects, so a counter local to one thread cannot be read by another. @std/sync
puts the shared object in a process-wide registry and hands each thread a
small handle that names it.

## One id, one object

A handle carries nothing except a number. Build two handles from the same id
and they reach the same object:

```rnx
import { AtomicInt } from "@std/sync";

let first = AtomicInt.byId(12);
let second = AtomicInt.byId(12);
first.set(41);
second.fetchAdd(1);
println!(first.get());
```

The program prints `42`. There is no copy of the cell in `second` and no
registration call between the two handles.

This is also how a thread receives shared state. Pass the id, then open a
handle on the other side:

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

The spawned function builds its own handle for id 21 and adds to the same
cell the parent reads.

## Registries are separate per type

An id identifies an object only inside one registry. The same number in two
registries names two unrelated objects:

```rnx
import { AtomicInt, Mutex } from "@std/sync";

let cell = AtomicInt.byId(5);
cell.set(9);

let lock = Mutex.byId(5);
println!(cell.get(), lock.tryLock());
lock.unlock();
```

This prints `9 true`. The mutex at id 5 is not holding the atomic cell at id
5. It is simply a different registry entry.

## First use creates the zero state

The first handle for an id creates the object in its zero state. An atomic
cell starts at 0, a channel starts empty, a mutex starts unlocked, an rwlock
starts released, and a condvar starts with no waiters. A barrier takes its
party count from the `byId` call.

The entry then stays registered until the process exits. There is no
`remove` or `close` that unregisters an id. `Channel.close` releases queued
values but leaves the id registered, so a later `send` on the same handle
works again.

Reuse an id only when you mean to reopen the same object. Reusing id 5 for
two unrelated counters gives them one cell and one shared value.

## Choosing ids

Pick numbers that no other code in the program uses. The registries are
shared with every dependency in the process, so a fixed small id can collide
with a library that picked the same number.

A common pattern is to keep the ids for one subsystem together and allocate
from a counter at startup. The registry does not provide allocation or name
lookup; the numbers are yours to manage.

## Blocking calls

Sharing a lock is only useful if waiting works. `Mutex.lock`,
`RwLock.readLock`, `RwLock.writeLock`, `Channel.recv`, `Condvar.wait`,
`Barrier.wait`, and `Thread.join` park the calling thread until the
condition is met.

The polling partners are `Mutex.tryLock`, `RwLock.tryReadLock`,
`RwLock.tryWriteLock`, and `Channel.tryRecv`. They return a value that
reports absence instead of waiting, so a loop can keep doing other work.
`Condvar.waitTimeout` is the bounded form: it waits up to a millisecond
count, then returns.
