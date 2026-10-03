---
title: "Cycles and Handles"
description: "GenRef weak handles, the byId registry pattern, decay, and intentional cycles."
section: "Memory, Systems, and Concurrency"
icon: "Recycle"
---

# Cycles and Handles

Scopes handle almost everything, but sooner or later two objects need to point at each other. When A holds B and B holds A, neither count ever reaches zero and neither object dies. This chapter gives you the three tools for exactly that shape, plus the destruction rules underneath.

## Back-edges use GenRef

The rule is directional: own **down** the tree with strong references, point **back up** with `GenRef<T>`, a 16-byte generational handle:

- Copying a `GenRef` never touches atomics.
- `.get()` returns the object — or `null` after the owner is freed, so dangling handles read as absence instead of corruption.
- On native backends, handles resolve through a process-wide slot table, and dropping an object invalidates its slots via the synthesized destructor.

The compiler watches the directions: a mutual strong pair warns as `W108` and suggests turning one side into a `GenRef`.

## The byId registry pattern

Cross-thread and cross-scope sharing uses the same idea through the standard library: primitives constructed `byId` open a shared cell that separate threads — or separate phases of one program — resolve independently. The handle, not the object, crosses the boundary:

```rnx
import { AtomicInt } from "@std/sync";

AtomicInt.byId(1).set(40);
print(AtomicInt.byId(1).fetchAdd(2));
print(AtomicInt.byId(1).get());
```

`Mutex`, `Channel`, `Barrier`, and `ThreadPool.byId` follow the same pattern: the id is the rendezvous, the registry owns the cell, and dropping your handle never drops the shared state out from under another holder. Full rules are in [Concurrency and Threads](/manual/14-concurrency-and-threads).

## Closures that capture this

A closure stored on its own object would cycle the same way: the object owns the closure, the closure captures the object. The fix is `fn decay(this)`, which captures `GenRef(this)` instead and checks it once on entry — void bodies return early, value bodies produce `null`. Forgetting `decay` on a self-capturing closure warns as `W104`; rewrite the closure with `fn decay(this)` to fix it.

## Intentional cycles opt out per field

Sometimes a cycle is the design — a doubly-linked list, a parent pointer you promise to clear. Opt out per field with `#[Allow(CyclicReference)]` and clear it manually in `deinit` or a `clear*`/`close*`/`reset*` method. A marked field that is never cleared warns as `W109`.

Note what cycles are *not*: `unsafe`. At worst an unbroken cycle wastes memory; it can never corrupt it, so cycle management stays in safe code. `unsafe {}` remains reserved for raw `Pointer`/`Address` work, C-ABI calls, and data races.

## What runs at destruction

Every class gets a synthesized destructor that releases its object fields recursively and expires weak handles. `init(params)` runs at construction for validation, derived fields, and resource acquisition; `deinit` runs at destruction for manual teardown:

```rnx
class Meter {
    let reading: Int;

    init(reading: Int) {
        this.reading = reading;
    }

    deinit {
        print("meter dropped");
    }

    fn bump(amount: Int): Int {
        this.reading = this.reading + amount;
        return this.reading;
    }
}

let m = new Meter(20);
print(m.bump(22));
```

`new Meter(20)` calls `init` — you never write `.init()`. When `m` goes out of scope, `deinit` runs, then fields release recursively.

## Summary

- Own down, `GenRef` up; `W108` flags mutual strong pairs.
- Shared cells rendezvous `byId`; handles cross boundaries, objects do not.
- Self-capturing closures use `fn decay(this)` (`W104` otherwise).
- Intentional cycles opt out per field with manual clearing (`W109` otherwise) — and cycles are never `unsafe`.
