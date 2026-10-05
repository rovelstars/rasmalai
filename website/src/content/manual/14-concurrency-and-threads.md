---
title: "Concurrency and Threads"
description: "Threadpool architecture, barriers, synchronization, and lock-free Atomics."
section: "Memory, Systems, and Concurrency"
icon: "Activity"
---

# Concurrency and Threads

One thread is easy to reason about and slow at scale. Rasmalai threads share memory under reference counting, coordinate through a few small primitives, and offer cooperative tasks for the waits in between. None of it needs `unsafe`.

## Spawning threads

`Thread.spawn` takes a zero-arg non-throwing function or closure and returns a handle; `handle.join()` blocks and returns a `Result` — `Ok` with the worker value, `Err` with a message when the worker fails. Await each handle once; worker results are `Int`/`Bool`/`Float`/`String`, and anything else is an `E108` at spawn:

```rnx
let factor = 2;
let handle = Thread.spawn(() => 21 * factor);
let res = handle.join();
print(res.unwrap());
```

Handles have a nameable type: `import { Thread } from "@std/sync"` and store them in an `Array<Thread>`, then join each element. Spawning inside the loop and joining immediately would serialize the workers, so spawn every handle first and join afterwards:

```rnx
import { Thread } from "@std/sync";

fn worker(): Int {
    return 1;
}

let workers: Array<Thread> = [];
let i = 0;
while i < 16 {
    workers.push(Thread.spawn(worker));
    i = i + 1;
}
for t in workers {
    t.join();
}
```

A barrier only trips when `count` parties are alive and waiting at the same time. Spawning one worker and joining it in the same loop iteration deadlocks a multi-party barrier: each iteration blocks in `join()` with a single worker alive, so the gate never fills and the program hangs with no diagnostic. Spawn every handle first, join afterwards — and size `count` to the number of concurrently running parties, never the number of loop iterations.

The ownership rule is load-bearing, not stylistic: the parent's objects can drop while the worker runs, so anything the worker needs must be computed from values it owns. A spawned closure captures its environment by reference, and the compiler does not stop a shared object from crossing threads — concurrent access to a shared object is the programmer's responsibility. Values that are safe to share are integers, `@std/sync` primitives built `byId`, and the internally locked `Map`/`Set`; a plain class with mutable fields is not: two threads bumping one counter lose updates to a read-modify-write race, with a final count that varies run to run and falls short of the sum.

## Sharing through byId primitives

When threads must share, they share through the `@std/sync` primitives, all constructed `byId` so separate threads open the same cell. An `AtomicInt` counts without locks; a `Channel` moves values between threads (`send` accepts `Int`, `Float`, `String`, `Array`, or any object, and `recv` returns `Any`, so the receiver casts before use); `Mutex` (`lock`/`unlock`/`tryLock`), `RwLock` (shared read or exclusive write locks), `Condvar` (`wait` with its mutex held, `notifyOne`/`notifyAll`), and `Barrier` (`byId(id, count)` with `count` greater than zero — zero or negative counts fail immediately — plus `wait`, which parks every party until the last one arrives, returns true for exactly one leader, and resets for the next round) cover the rest:

```rnx
import { AtomicInt } from "@std/sync";

let c = AtomicInt.byId(1);
c.set(40);
print(c.fetchAdd(2), c.get());
```

> [!WARNING]
> A spawned closure captures its environment by reference: nothing stops a plain object from reaching two threads, and concurrent mutation of a shared object is a data race the compiler will not catch. Share integers, `@std/sync` primitives built `byId`, or the internally locked `Map`/`Set` — never a plain class with mutable fields.

A `Map` filled concurrently from two threads, collected with `join`:

```rnx
import { Map } from "@std/collections";

fn fill(totals: Map, base: Int): Int {
    for j in 0 .. 1000 {
        totals.set(base + j, j);
    }
    return 1;
}

fn Main(): Int {
    let totals = new Map<Int, Int>();
    let a = Thread.spawn((): Int => fill(totals, 0));
    let b = Thread.spawn((): Int => fill(totals, 1000));
    print("a=${a.join().unwrap()} b=${b.join().unwrap()}");
    print("len=${totals.len()} (expected 2000)");
    return 0;
}
```

`Channel.close` releases anything still queued, and the channel retains sent heap values, transferring the reference to the receiver. Every primitive has a generated reference page under [its module docs](/docs/@std/sync/overview).

## Sharing primitive reference

All sharing primitives construct `byId` so separate threads open the same cell. The id is the rendezvous, the registry owns the cell, and dropping your handle never drops the shared state out from under another holder.

| Primitive | Construction | Key calls |
|---|---|---|
| `AtomicInt` | `AtomicInt.byId(id)` | `set(v)`, `get()`, `fetchAdd(n)` |
| `Channel` | `Channel.byId(id)` | `send(v)` (`Int`, `Float`, `String`, `Array`, or any object), `recv(): Any` (cast before use), `close()` |
| `Mutex` | `Mutex.byId(id)` | `lock()`, `unlock()`, `tryLock()` |
| `RwLock` | `RwLock.byId(id)` | shared read or exclusive write locks |
| `Condvar` | `Condvar.byId(id)` | `wait` (with its mutex held), `notifyOne()`, `notifyAll()` |
| `Barrier` | `Barrier.byId(id, count)` (`count` above zero; zero or negative fails immediately) | `wait()` parks every party until the last arrives, returns true for exactly one leader, resets for the next round |

```rnx
import { AtomicInt } from "@std/sync";

let c = AtomicInt.byId(7);
c.set(0);
assert(c.fetchAdd(5) == 0, "previous value");
assert(c.get() == 5, "updated");
```

## Thread and pool reference

| Call | Signature | Returns |
|---|---|---|
| `Thread.spawn(task)` | zero-arg non-throwing function or closure | handle |
| `handle.join()` | blocks until the worker finishes | `Result`: `Ok` with the worker value, `Err` with a message on failure |
| `ThreadPool.new(workers)` | pool size | pool (no import needed; compiler-recognized) |
| `ThreadPool.byId(id, workers)` | named pool | shared pool |
| `pool.submit(task)` | zero-arg function or closure | task handle |
| `pool.submitArg(task, arg)` | one-`Int`-arg function or closure plus its argument | task handle |
| `pool.parallelFor(start, end, chunk, worker)` | chunked index range plus worker | blocks until done |
| `task.join()` | blocks exactly once per handle | that task's `Result` |
| `pool.shutdown()` | lifecycle end | — |

Worker results are `Int`/`Bool`/`Float`/`String`; anything else is an `E108` at spawn. Await each handle once. Spawn every handle first and join afterwards — spawning inside the loop and joining immediately serializes the workers, and a multi-party barrier with one live worker per iteration deadlocks with no diagnostic.

```rnx
import { Thread } from "@std/sync";

let handles: Array<Thread> = [];
let i = 0;
while i < 4 {
    handles.push(Thread.spawn((): Int => 1));
    i = i + 1;
}
let sum = 0;
for t in handles {
    sum = sum + t.join().unwrap();
}
assert(sum == 4, "all workers");
```

`task.join()` is a method on pool and thread handles that blocks for that handle's `Result`. It is unrelated to the `await` keyword, which suspends an `async fn` until a `Promise<T>` settles; calling `await` on a handle is an `E108` error. Cooperative `async`/`await` over `Promise<T>` is specified in [Functions and Closures](/manual/05-functions-and-closures).

## Pools for data parallelism

`ThreadPool` is compiler-recognized like `Thread` — no import needed. `new(workers)` creates a pool (or `byId(id, workers)` for a named one), `submit(task)` queues a zero-arg function or closure and returns a task handle, `submitArg(task, arg)` queues a one-`Int`-arg function or closure, and `parallelFor(start, end, chunk, worker)` runs a worker over every index in chunks and blocks until done, with `join` and `shutdown` for lifecycle control. `task.join()` blocks exactly once per handle and returns that task's `Result`:

```rnx
let pool = ThreadPool.new(4);
let task = pool.submit(() => "done".concat("!"));
let out = task.join();
print(out.unwrap());
pool.shutdown();
```

Retains and releases are atomic across threads, so an object shared through a pool drops deterministically on whichever thread releases it last — with no pause and no finalizer queue.

> [!NOTE]
> `task.join()` here is a *method* on pool and thread handles that blocks for that handle's `Result`. It is unrelated to the `await` *keyword*, which suspends an `async fn` until a `Promise<T>` settles (see below); calling `await` on a handle is an `E108` error.

The filesystem pool follows the same shape with no per-call setup: one process-wide pool serves every `@std/fs` `*Async` call, sized from the CPU count and clamped into 1..8. `fs.setWorkers(n)` resizes it (clamped the same way) but only while no async operation is in flight — resizing stops the old pool, so anything still queued never runs. Calls queue work instead of spawning a thread each, and a nested `*Async` issued from inside a pool worker runs inline instead of queuing, so a full pool can never deadlock waiting on itself. Plain sync calls never touch the pool; they block the caller directly.

## Cooperative tasks with async

`async fn foo(): T` returns a `Promise<T>`; `await child()` parks the worker thread at 0% CPU until the child settles, then resumes with the fulfilled value (rejection propagates and rejects the awaiting function in turn). No hidden executor, no polling: direct `.poll()` calls are an `E111` error and the `Poll` enum is retired. `pass` is the explicit no-op for empty cases.

```rnx
async fn compute(): Int {
    return 41 + 1;
}

let result = await compute();
return result;
```

`Main` itself may be `async`; the compiler runs it and uses the resolved value as the exit code. Or skip the wrapper: the entry file runs top-level statements directly, so top-level `await` just works. From synchronous code, bridge with a blocking wait instead: `compute().wait().unwrap()`. `await` outside an `async fn` is an `E109` error, except at the top level of the entry file.

## Summary

- `Thread.spawn` + `join()` returning `Result`; await each handle once.
- `@std/sync` primitives (`AtomicInt`, `Channel`, `Mutex`, `RwLock`, `Condvar`, `Barrier`) constructed `byId` for cross-thread sharing — the only legal sharing path.
- `ThreadPool.new`/`byId` with `submit` + `join()` and chunked `parallelFor`.
- One process-wide fs pool backs every `@std/fs` `*Async` call (CPU count clamped to 1..8, `setWorkers` to resize, nested calls run inline).
- `async`/`await` over `Promise<T>` with `.wait()` bridging; `E109` guards misplaced awaits, `E111` bans direct polling.
