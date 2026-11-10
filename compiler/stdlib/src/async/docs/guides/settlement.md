---
title: "Settlement"
description: "Resolve, reject, attach handlers, and wait for a promise to settle."
section: Settlement
order: 20
---

# Settlement

A promise has three states: pending, fulfilled, and rejected. It moves out of pending once. The first settlement wins, and later calls are ignored. Most code gets a promise from an `async fn`, but sometimes the value arrives from a worker thread, an OS callback, or an I/O handler. That is when the manual handles matter.

## Already settled promises

`Promise.resolve(value)` builds a fulfilled promise. `Promise.reject(reason)` builds a rejected one. Both are useful in tests and as fallbacks.

```rnx
let ok: Result<Int, String> = Promise.resolve(41).wait();
println!(ok.unwrap());

let bad: Result<Int, String> = Promise.reject<Int>("boom").wait();
println!(bad.isErr());
```

The first line prints `41`. The second prints `true`. `wait()` returns `Ok(value)` for a fulfillment and `Err(reason)` for a rejection.

## Settle a pending promise by hand

`Promise.withResolvers<T>()` returns a pending promise plus the calls that settle it. Keep the handles with the code that produces the value.

```rnx
let wr = Promise.withResolvers<Int>();
wr.resolve(42);
println!(wr.promise.wait().unwrap());
```

The program prints `42`. `wr.promise` is the pending `Promise<Int>`, `wr.resolve(value)` fulfills it, and `wr.reject(reason)` rejects it. Calling either one after the promise has settled changes nothing.

## Attach both continuations

`attach(onFulfilled, onRejected)` registers one handler for each outcome. A pending promise queues the pair. A promise that has already settled runs the matching handler at once on the calling thread.

```rnx
let wr = Promise.withResolvers<Int>();
wr.promise.attach(
    (v: Int): Void => println!("got", v),
    (e: String): Void => println!("bad", e)
);
wr.resolve(7);
```

The program prints `got 7`. `resolve` settles the promise and then runs the queued fulfillment handler. Only one of the two handlers runs for a given promise.

`fulfill(value)` and `rejectWith(reason)` are the instance methods behind the handles. They run queued handlers after releasing the guard, never while holding it.

## Block with wait

`wait()` parks the calling OS thread until the promise settles. It sleeps on a condition variable in one-second slices and gives up after five seconds. A timeout comes back as `Err` with a message that names the wait.

```rnx
let wr = Promise.withResolvers<Int>();
let r: Result<Int, String> = wr.promise.wait();
println!(r.isErr());
```

This program waits five seconds, then prints `true` because nothing settled the promise.

`wait()` is fine in a plain function or a `Thread.spawn` body. Inside an `async fn` it is a compile error, because parking an event-loop task would deadlock.
