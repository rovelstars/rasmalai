---
title: "Keys and ordering"
description: "Which types can be keys, how identity works, and how insertion order and threading behave."
section: Keys
order: 40
---

# Keys and ordering

A map or set is only as predictable as its key rules. This page covers the types that can be keys, how two keys are compared, how insertion order survives updates, and what happens when threads share a table.

## Hashable keys

Keys must be `Int`, `Float`, `Bool`, `String`, or a heap object compared by identity. Arrays and `null` are not hashable. Passing one aborts the interpreter with `unhashable key type`, so keep keys to the supported types.

`Int` and `Float` never mix. A map that accepts both has two separate entries, not one:

```rnx
import { Map } from "@std/collections";

let mixed = new Map<Any, Any>();
mixed.set(1, "int");
mixed.set(1.0, "float");
println!(mixed.len());
```

The program prints `2`. The key `1` and the key `1.0` are different keys.

Float keys compare by bit pattern. That makes `0.0` and `-0.0` different keys, while every `NaN` counts as one key.

## Objects compare by identity

Two class instances with equal fields are still different keys. Store an object and look it up with the same reference.

```rnx
class Tag {
    let name: String;

    init(name: String) {
        this.name = name;
    }
}

let first = new Tag("ore");
let second = new Tag("ore");
let tags = new Map<Tag, Int>();
tags.set(first, 1);
println!(tags.has(first), tags.has(second));
```

The program prints `true false`. `first` and `second` hold the same text, but they are distinct objects, so only the stored reference finds the entry.

## Insertion order

An entry keeps the position its key was first given. Setting an existing key leaves that position alone. Deleting a key and setting it again moves it to the end. `keys()` and `values()` line up index by index, and each call returns a fresh array, so writes to the map never change an array you already took.

A `Set` follows the same rules. Adding a member that is already present keeps one copy and leaves its position alone.

## Ownership

The map owns a reference to every key and value it holds. The count goes up on `set` and down when an entry is overwritten, deleted, cleared, or when the map itself is deinitialized. Keys and values need no cleanup of their own. A set owns its members the same way.

## Threads

One lock guards the table, so a single map or set can be shared between threads. A read-modify-write pair such as `get` then `set` is still two separate operations. Hold a `Mutex` from `@std/sync` around the pair when threads race.

## Printing

`println!` renders a map as `{k: v, ...}` in insertion order through the `@std/io` pretty renderer. Read the contents with `keys()` or `values()` when you need the pieces as values.
