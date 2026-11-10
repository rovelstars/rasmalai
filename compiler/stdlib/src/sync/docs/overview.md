---
title: Sync
description: Share atomics, channels, locks, and threads through process-wide registry ids.
section: Overview
order: 0
---

# Sync

@std/sync gives threads a way to share state. Every shared primitive is a
handle around a registry id, and `Thread` wraps a native thread slot. Two
handles built with the same id point at the same atomic cell, queue, or
lock, even when the threads that built them never met.

The registry is the sharing mechanism. A thread only needs the number to
reach the object, so you can pass an `Int` between threads and open a handle
with `AtomicInt.byId(id)`. Registries are separate per type: `AtomicInt.byId(5)`
and `Mutex.byId(5)` are unrelated objects that happen to use the same
number. Pick ids that no other part of the program uses.

Values start in their zero state the first time an id is touched, and the
entry stays registered for the rest of the run. There is no unregister step.
Reusing an id means reopening the same object.

The package covers seven types:

- `AtomicInt`: a shared 64-bit integer with atomic read, write, add, and
  compare-and-swap.
- `Channel`: an unbounded first-in-first-out queue.
- `Mutex`: one holder at a time.
- `RwLock`: many readers or one writer.
- `Condvar`: park and wake threads that wait on a mutex.
- `Barrier`: release a group once every party arrives.
- `Thread`: the handle returned by `Thread.spawn`.

A small program that uses two of them:

```rnx
import { AtomicInt, Channel } from "@std/sync";

let hits = AtomicInt.byId(1);
hits.set(0);
hits.fetchAdd(5);

let queue = Channel.byId(2);
queue.send("job");

println!(hits.get());
println!(queue.len(), queue.recv());
```

This prints `5` and then `1 job`. The atomic cell and the queue keep their
state for the rest of the process.

## Blocking and polling

Blocking calls park the calling thread until the condition is met.
`Channel.recv`, `Mutex.lock`, `RwLock.readLock`, `RwLock.writeLock`,
`Condvar.wait`, `Barrier.wait`, and `Thread.join` all block.

Most blocking calls have a non-blocking partner: `Channel.tryRecv`,
`Mutex.tryLock`, `RwLock.tryReadLock`, and `RwLock.tryWriteLock`. They
report absence in their return value instead of parking, so polling loops
stay writable. `Condvar.waitTimeout` is the bounded partner: it parks until
a notify arrives or the timeout elapses.

Locks are not reentrant and do not record an owner. Locking a mutex twice
from one thread parks forever. Unlocking a mutex nobody holds aborts the
program. Give every `lock` exactly one matching `unlock` on every path out
of the critical section.

## Guides

- [Handles and registry ids](guides/handles) explains sharing, lifetimes,
  and id choice.
- [Atomic counters and CAS](guides/atomics) covers `AtomicInt`.
- [Channels](guides/channels) covers queued values and producer threads.
- [Locks and condition variables](guides/locks) covers `Mutex`, `RwLock`,
  and `Condvar`.
- [Threads and barriers](guides/threads) covers `Thread.spawn`, `join`, and
  `Barrier`.

Start with [Getting started](getting-started) if you want a program to run
first.
