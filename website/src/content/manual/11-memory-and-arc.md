---
title: "Memory and ARC"
description: "Deterministic ARC: scopes, stack vs heap, retains, releases, and write-dominance."
---

# Memory and ARC

The whole language rests on one invariant: **every value is owned by exactly one scope — the block that created it — and when execution leaves that scope, the value is released.** No collector, no annotations, no manual pairing of allocation with release.

## Scopes own values

If you can see the closing brace, you can see the free:

```rnx
let total = 0;
let i = 0;
while i < 3 {
    let bonus = i * 10;
    total = total + bonus;
    i = i + 1;
}
print(total);
```

`bonus` dies at the end of each iteration; `total` dies when `main` returns. There is no background thread tracing garbage, so there are no pauses.

## Stack first, heap when needed

Locals live on the **stack**: returning from a function frees all of its locals in one step, at zero per-value cost. Larger or longer-lived values live on the **heap** under reference counting.

The working rule: small fixed-size values (`Int`, `Float`, `Bool`, `Vec4f`) live inline with no heap involved, while classes, strings, arrays, and enums go on the heap. Every heap object carries a 16-byte header (reference count, type id, generation); strings extend it with byte length and a cached character count. Retains and releases are atomic across threads, so sharing an object with another thread is always safe.

## Sharing with ARC

`let b = a` shares the object, bumping its count. The compiler inserts every retain and release — field stores retain the new value and release the old one, copies move the single share, field loads retain:

```rnx
let arr = [1, 2, 3];
let same = arr;
print(same.length());
```

`same` and `arr` point at one array. When the last owner goes away, the array drops immediately — deterministically, on the thread that released it, with no pause and no finalizer queue.

Passing arguments usually costs nothing extra: non-escaping arguments pass as **borrows**, so most calls emit no retain/release traffic at all.

## Release requires write-dominance

A release of a local's old value is emitted only when the compiler can prove the local was definitely written on every path reaching the release point (**write-dominance**). A slot that might still hold its initial state is never released — this is what makes loop-carried values and multi-branch assignment sound:

```rnx
let acc = "";
let i = 0;
while i < 3 {
    acc = i % 2 == 0 ? acc + "e" : acc + "o";
    i = i + 1;
}
print(acc);
```

Each iteration's copy of `acc` releases exactly the value the previous iteration (or the initializer) provably wrote. Moved-from slots are never re-read for release; suppression of a release only delays a free, it never introduces one.

User code never manages any of this — the invariant exists so you can trust that aliasing never produces use-after-free: every handle keeps its object alive exactly as long as the handle itself lives.

## Heap layouts

- **Strings**: 32-byte header (16-byte object header plus byte length and a cached character count), then UTF-8 bytes and a NUL. Literals are immortal and never freed; concatenated strings drop like objects.
- **Arrays**: 40-byte header (object header plus length, capacity, data pointer). Elements drop recursively through per-type destructors.
- **Enums**: 16-byte header plus tag and payload slots sized to the largest variant. Owned payloads drop through a synthesized per-enum destructor.

## Nullable values

`String?`, class, and array slots hold a raw pointer (`0x0` for `null`) at zero extra cost. Nullable scalars (`Int?`, `Bool?`, `Float?`) are stored boxed: a non-null value lives in a small heap box so the `0` word unambiguously means `null`. Boxing is inserted at typed boundaries (`let` annotations, parameters, returns, fields, call results); each box is released when its slot dies, so nullable scalar traffic shows no live-count growth.

Limits: erased `Any` slots do not track nullability. A raw `0` smuggled through `Any` (for example a native call returning a bare word, or an `Any` holding integer `0`) reads as `null` in `== null` / `??` checks. Keep scalars typed through nullable flows instead of round-tripping them through `Any`. Arrays of nullable scalars erase element nullability the same way; `null` elements round-trip only through `Any`-element arrays or explicit sentinel values.

## Summary

- One owner per scope; scope end is the free point.
- Stack for locals; 16-byte-header heap objects shared by atomic ARC.
- Borrows make most calls free; releases require proven writes (write-dominance).
- Cycles are the one shape scopes cannot express — see [Cycles and Handles](/manual/12-cycles-and-handles).
