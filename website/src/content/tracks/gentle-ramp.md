---
title: "Zero to Systems"
description: "Machine concepts and first programs for developers new to low-level code."
track: "gentle-ramp"
---

# Zero to Systems

You can write application code and want to understand what happens closer to
the metal. This track teaches the machine model first, then the language.

## Stack vs heap

Every value lives somewhere. Locals live on the **stack**: a fixed-size region
that grows and shrinks as functions enter and exit. Stack allocation costs
nothing at runtime because the space is reclaimed the moment the scope ends.

```rnx
let score = 0;
score = score + 10;
print(score);
```

Larger or longer-lived values live on the **heap**: memory requested at
runtime and released later. Heap objects in Rasmalai carry a small header with
a reference count, and the compiler inserts the retain/release calls for you.

## Fixed-width integers

`Int` is always 64 bits. There is no platform-dependent `int`, no implicit
widening surprises. `Float` is always an IEEE-754 double:

```rnx
let count: Int = 41;
let ratio: Float = 3.0;
print(count + 1, Int(ratio));
```

## Why no garbage collector

A collector pauses your program to find dead objects. Rasmalai instead ties
each value to a **lexical scope**: the block of code that owns it. When the
block ends, its locals drop in reverse order. The owner is visible in the
source, so there is nothing to trace at runtime:

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

`bonus` dies at the end of each loop iteration. `total` dies when `main`
returns. No background thread, no pause.

## Cleanup with defer

Resources that are not memory (files, locks, timers) release with `defer`.
Deferred blocks run when the scope exits, last-in first-out:

```rnx
defer { print("closed"); }
print("working");
```

## Grouping data

A `struct` bundles fields with a free memberwise initializer:

```rnx
struct Meter {
    let name: String;
    let reading: Int;
}

let m = Meter("heat", 21);
print(m.name, m.reading);
```

Next: run these samples in the [playground](/playground), then read the
[Variables guide](/manual/03-bindings-and-scope).
