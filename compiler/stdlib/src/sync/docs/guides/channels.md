---
title: Channels
description: Pass values between threads with an unbounded FIFO queue.
section: Channels
order: 40
---

# Channels

A shared counter is enough for a number. When one thread produces work and
another consumes it, you need an order and a place to wait. `Channel` is an
unbounded first-in-first-out queue shared through a registry id.

## Send and receive

```rnx
import { Channel } from "@std/sync";

let jobs = Channel.byId(9);
jobs.send(7);
jobs.send(8);
println!("queued:", jobs.len());
println!("took:", jobs.tryRecv());
println!("next:", jobs.recv());
println!("empty:", jobs.tryRecv());
```

This prints `queued: 2`, `took: 7`, `next: 8`, and `empty: -1`. Values come
out in the order they went in.

`send` never blocks, whatever the queue length. `recv` parks the calling
thread while the queue is empty and returns the next value. `tryRecv` is the
polling version: it returns `-1` when nothing is queued. Because `tryRecv`
returns an `Int`, use it for integer messages and `recv` for strings,
objects, and arrays.

## Hand a value to a worker

`send` takes an `Any`, and `recv` hands the value back as an `Any`. Narrow
the result before you use it:

```rnx
import { Channel, Thread } from "@std/sync";

fn produce(): Int {
    let outbox = Channel.byId(31);
    outbox.send("payload");
    return 1;
}

let inbox = Channel.byId(31);
let worker = Thread.spawn(produce);
let message = inbox.recv();
if message is String {
    println!("message:", message);
}
let result = worker.join();
println!("worker:", result.unwrapOr(0));
```

This prints `message: payload` and `worker: 1`. Heap values such as strings
are retained for the receiver and released when the receiver drops them, so
a value sent across threads stays valid.

Start the worker before the receive. If the parent calls `recv` first and no
value is queued, it parks until the worker sends one. That is usually what
you want, but a worker that never sends leaves the parent blocked.

## Growth and close

The queue grows to fit whatever is written to it. A fast producer can
outrun a slow consumer and use memory until the queue is drained. Size the
work or add backpressure when the producer can run far ahead.

`close` releases whatever is queued and leaves the id registered. A later
`send` on the same handle works again. It does not wake a thread parked in
`recv`, so treat it as a way to free buffered values, not as a shutdown
signal. A blocked consumer needs a `send` from somewhere else, or a
`Condvar` with a timeout from [Locks and condition variables](locks).

## Related API

`Channel.byId` opens the queue, `send` enqueues, `recv` blocks for the next
value, `tryRecv` polls, `len` reports the queued count, and `close` releases
the buffer. The count is advisory: another thread may enqueue or dequeue
right after the read.
