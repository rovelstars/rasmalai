---
title: "Classes and Objects"
description: "Heap reference types under ARC: construction, inheritance, methods, binding."
section: "Type System and Object Model"
icon: "Shapes"
---

# Classes and Objects

## Reference semantics

A `class` is the reference type: instances live on the heap under ARC, and assignment shares identity (two handles, one object) rather than copying. The rule for choosing between the two data types: `class` for shared identity or polymorphism, `struct` for plain data. Memory behavior is specified in [Memory and ARC](/manual/11-memory-and-arc).

```rnx
class Meter {
    let reading: Int;

    init(reading: Int) {
        this.reading = reading;
    }

    fn bump(amount: Int): Int {
        this.reading = this.reading + amount;
        return this.reading;
    }
}

let m = new Meter(20);
print(m.bump(22));
```

Construction uses `new`: `new Meter(20)` allocates the object and runs `init`. Calling the class directly (`Meter(20)`) is an `E204` error.

## Single inheritance

`extends` names one parent class. The child inherits fields and methods: parent fields prefix the child's layout, so parent methods run unchanged on child instances, and `is` checks match ancestors. Every class gets a synthesized destructor releasing object fields recursively and expiring weak handles; `deinit` adds manual teardown.

```rnx
class Base {
    let x: Int;
    init(x: Int) { this.x = x; }
    fn get(): Int { return this.x; }
}

class Child extends Base {
    let y: Int;
    init(x: Int, y: Int) { super(x); this.y = y; }
}

let c = new Child(20, 22);
print(c.get());
print(c.y);
```

Method definitions in the child replace the parent's when signatures match; a mismatched signature is an `E108` error. An inherited method's `this.` self-call resolves to the child's override, so template-method shapes behave the same through `extends` as through `with` — and `super.method()` is the escape hatch that reaches the parent implementation. `private` members stay invisible to the child (`E203` outside the defining class), and redefining an inherited field name is an `E108` error. The base must be a class (not a struct, enum, or interface) and cycles are rejected. A child without its own `init` constructs positionally over all fields, inherited first, then runs the parent `init` when it takes no parameters.

A child `init` runs the parent `init` first. Call `super(...)` with the base arguments explicitly; when the base `init` takes no parameters the call is inserted implicitly. Omitting `super(...)` while the base `init` takes parameters is an `E108` error, as is `super` outside a method or `init` body, or in a class without a parent. A throwing base `init` propagates through `new` to the caller's `try`/`catch` like any throwing call.

## Methods and binding

Methods defined in the class body take an implicit receiver and call with dot syntax; `this` refers to the receiver inside the body. `static` methods belong to the type rather than any instance and call with no receiver (`Clock.virtual(0)`, `Vec4f.splat(0.0)`, `AtomicInt.byId(1)`). `private` methods accessed outside their scope are `E203` errors.

Declaring one puts `static` ahead of `fn` inside the `class` or `struct` body, and every call lands on the type, with no instance in it:

```rnx
class Clock {
    let seconds: Int;
    init(seconds: Int) { this.seconds = seconds; }

    static fn base(): Int { return 60; }

    static fn virtual(seconds: Int): Clock {
        return new Clock(seconds + Clock.base());
    }
}

let c = Clock.virtual(0);
print(c.seconds);
```

That prints `60`. A static calling a sibling static qualifies the name the same way (`Clock.base()`, not the bare `base()`, which is an `E303` error), and a static body has no receiver to name: `this` is an `E108` error there, as it is in any plain function.

Capability adoption uses `with` (traits) and `:` (interfaces); both are specified in [Traits and Interfaces](/manual/08-traits-and-interfaces):

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

## Destruction order

When the last handle releases, `deinit` runs first, then object fields release recursively. Locals drop at scope end under the same rules as every other value: copies move the single share, field loads retain, field stores retain the new value and release the old one. Cyclical shapes require handles, not strong fields; see [Cycles and Handles](/manual/12-cycles-and-handles).

## Construction and inheritance reference

| Rule | Shape | Error when broken |
|---|---|---|
| construction uses `new` | `new Meter(20)` | direct call `Meter(20)`: `E204` |
| one parent only | `class Child extends Base` | base a struct, enum, or interface: `E108`; cycles rejected |
| child `init` runs parent first | `super(x);` with base arguments | omitted `super(...)` with parameterized base: `E108` |
| no-arg base gets implicit `super()` | child omits the call | — |
| `super` only in method or `init` of a child | `super.method()` reaches parent | `super` elsewhere: `E108` |
| overrides match signatures | same name, same shape | mismatched signature: `E108` |
| fields are never redefined | child adds new names only | redefined field name: `E108` |
| `private` stays home | visible in defining class only | access outside: `E203` |
| static has no receiver | `Clock.virtual(0)` on the type | `this` in static: `E108`; bare sibling name: `E303` |

```rnx
class Base {
    let x: Int;
    init(x: Int) { this.x = x; }
    fn get(): Int { return this.x; }
}

class Child extends Base {
    let y: Int;
    init(x: Int, y: Int) { super(x); this.y = y; }
    fn get(): Int { return this.x + this.y; }
}

let c = new Child(20, 22);
assert(c.get() == 42, "override");
assert((c is Base) && (c is Child), "ancestors match");
```

## Summary

- `class`: heap identity under ARC, `new` construction.
- `extends` takes one parent; `deinit` plus a synthesized field-releasing destructor handle teardown.
- Methods bind an implicit `this`; `static` methods call on the type.
- Traits and interfaces adopt with `with` and `:`.
