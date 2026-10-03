---
title: "Extensions and Operators"
description: "Extension blocks, static receiver desugaring, Iterable, and operator hooks."
section: "Type System and Object Model"
icon: "Zap"
---

# Extensions and Operators

Rasmalai prefers static desugaring over dynamic dispatch. Protocols (`Iterable`/`Iterator`), operator overloads (`op_*`), and extensions all compile to direct calls with no runtime lookup. This chapter states each desugaring exactly.

## Iterable and Iterator

`Iterable<T>` requires one method: `iterator(): Iterator<T>`. `Iterator<T>` requires `next(): T?`, yielding the value per element and `null` at exhaustion. `Array`, `Map` (keys), and `Set` (members) conform; your own types conform the same way:

```rnx
class Counter with Iterable<Int> {
    let limit: Int;
    init(limit: Int) { this.limit = limit; }

    fn iterator(): Iterator<Int> {
        return new CounterIter(0, this.limit);
    }
}

class CounterIter with Iterator<Int> {
    let at: Int;
    let limit: Int;
    init(at: Int, limit: Int) { this.at = at; this.limit = limit; }

    fn next(): Int? {
        if this.at >= this.limit { return null; }
        this.at = this.at + 1;
        return this.at - 1;
    }
}

let total = 0;
for x in new Counter(3) {
    total = total + x;
}
print(total);
```

`for..in` over anything with `.iterator()` desugars to a `while` loop over `next()` with identical `break`/`continue` semantics. Arrays and ranges keep their zero-allocation fast paths.

## Operators desugar to methods

Non-primitive types overload operators by defining hook methods — directly on the type or in an `extension` block. Primitives keep their hardware instructions; anything else desugars statically to a direct call with no dynamic dispatch. A missing hook is `E108` naming the operator and the type:

| Operator | Hook | Signature shape |
|---|---|---|
| `a + b` | `op_add` | `(other: Self): Self` |
| `a - b` | `op_sub` | `(other: Self): Self` |
| `a * b` | `op_mul` | `(other: Self): Self` |
| `a / b` | `op_div` | `(other: Self): Self` |
| `-a` | `op_neg` | `(): Self` |
| `a[i]` | `op_index` | `(index: Int): T` |
| `a[i] = v` | `op_index_set` | `(index: Int, value: T)` |

```rnx
struct Vec2 {
    let x: Float;
    let y: Float;
}

extension Vec2 {
    fn op_add(other: Vec2): Vec2 {
        return Vec2(this.x + other.x, this.y + other.y);
    }
}

let v = Vec2(1.0, 2.0) + Vec2(3.0, 4.0);
print(v.x, v.y);
```

## Extensions add methods from outside

An `extension` block attaches methods to an existing type — a primitive, a collection, or your own class — without touching its declaration. Calls desugar to plain static functions taking the receiver first, so there is no runtime cost, and intrinsics keep priority on conflicts.

This is also how the standard library grows foundation types: `Array`'s `map`/`filter`/`find`/`reduce`/`join` and `String`'s `split`/`replace`/`contains` are all extension methods in `@std/prelude`.

## Traits describe capabilities

A `trait` declares a method set that classes adopt with `with`, letting generic code depend on behavior rather than concrete types. Small interfaces, adopted explicitly, composed freely.

## Interfaces dispatch without vtables

An `interface` declares method signatures only. A class adopts one or more with `:` after its name (or `with`, as `ArrayIter<T> with Iterator<T>` shows), and the compiler checks every method is implemented with a matching signature. Values can then be typed by the interface, and calls dispatch to the runtime implementation:

```rnx
interface Summarizable {
    fn summary(): String;
}

class User : Summarizable {
    let name: String;
    init(name: String) { this.name = name; }
    fn summary(): String { return this.name; }
}

let u = new User("Al");
let s: Summarizable = u;
print(s.summary());
print(s is Summarizable);
```

There are no fat pointers or vtables in the ABI — the compiler resolves the runtime class id to a direct static call per implementation. `is` tests interface membership, and `typeOf` on an interface-typed value reports the concrete class. Calls through an interface work identically on every backend.

## Summary

- `Iterable<T>` + `Iterator<T>` drive `for..in`; `next()` yields `T?` with `null` at exhaustion.
- Operators on non-primitives desugar to `op_*` hooks statically; missing hooks are `E108`.
- Extensions compile to static functions; intrinsics win conflicts.
- Traits compose behavior; interfaces dispatch by class id with no vtables.
