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

```rnx
class Ticker {
    let cb: fn(): Int;
    let n: Int;
    init() {
        this.n = 0;
        this.cb = fn decay(this) { return 1; }
    }
}

let t = new Ticker();
assert(t.cb() == 1, "decayed call");
```

## GenRef reference

| Item | Signature | Meaning |
|---|---|---|
| `GenRef<T>` | 16-byte generational handle | the back-edge type; own down with strong references, point back up with this |
| `GenRef(obj)` | wraps a live object | copying the handle never touches atomics |
| `.get()` | `.get(): T?` | the object, or `null` after the owner is freed — dangling handles read as absence |
| mutual strong pair | — | warns as `W108`, suggesting one side become a `GenRef` |

```rnx
class Node {
    let value: Int;
    init(v: Int) { this.value = v; }
}

fn probe(): Int {
    let weakRef: GenRef<Node>? = null;
    do {
        let temp = new Node(99);
        weakRef = GenRef(temp);
        guard let active = weakRef.get() else { return 0; }
    } while false
    guard let stale = weakRef.get() else { return 42; }
    return stale.value;
}

assert(probe() == 42, "expired reads null");
```

On native backends, handles resolve through a process-wide slot table, and dropping an object invalidates its slots via the synthesized destructor.

## Intentional cycle reference

| Shape | Marker | Clearing rule | Warning when broken |
|---|---|---|---|
| back-edge (usual case) | `GenRef<T>` field | none needed; expiry reads as `null` | `W108` on mutual strong pairs |
| self-capturing closure | `fn decay(this)` | none needed; dead owner returns early / yields `null` | `W104` without `decay` |
| deliberate cycle (doubly-linked list, parent pointer) | `#[Allow(CyclicReference)]` on the field | clear to `null` in `deinit` or a `clear*`/`close*`/`reset*` method | `W109` when never cleared |

```rnx
class Link {
    let next: Link?;
    #[Allow(CyclicReference)]
    let back: Link?;
    init() {
        this.next = null;
        this.back = null;
    }
    fn clearLinks() { this.back = null; }
    deinit { this.clearLinks(); }
}

let a = new Link();
assert(a.back == null, "starts clear");
```

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
