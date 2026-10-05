---
title: "Traits and Interfaces"
description: "Trait composition with with, interface dispatch, is and typeOf."
section: "Type System and Object Model"
icon: "Library"
---

# Traits and Interfaces

## Traits

A `trait` declares a method set with bodies. Classes adopt traits with `with`, letting generic code depend on behavior rather than concrete types. Methods the class does not define are mixed in from the trait; methods it does define override. Trait `let` fields with defaults are carried into each adopting class the same way. Multiple traits compose freely on one class alongside a single `extends` parent.

```rnx
trait Clickable {
    fn onClick() { print("clicked"); }
}

class Button with Clickable {
    fn onClick() { print("pressed"); }
}

let b = new Button();
b.onClick();
```

`Iterable<T>` and `Iterator<T>` are traits of the same kind: `Iterable<T>` requires `iterator(): Iterator<T>`; `Iterator<T>` requires `next(): T?`. The `for..in` desugaring over conforming types is specified in [Extensions and Operators](/manual/09-extensions-and-operators).

## Interfaces

An `interface` declares method signatures only. A class adopts one or more with `:` after its name (or with `with`); the compiler checks every method is implemented with a matching signature, whether the method is written in the class body, mixed in from a `with` trait, or inherited from an `extends` parent. Values can then be typed by the interface, and calls dispatch to the runtime implementation.

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

There are no fat pointers or vtables in the ABI: the compiler resolves the runtime class id to a direct static call per implementation, decided at compile time from the receiver's static class. Calls through an interface behave identically on every backend.

## is and typeOf

`is` tests interface or type membership and narrows the value inside the matching branch; no cast is required. `typeOf` on an interface-typed value reports the concrete class.

```rnx
let v: Any = 42;
if (v is Int) {
    print(v + 1);
}
print(typeOf(v));
print(v is String);
```

`case is Type:` arms apply the same narrowing inside `switch`; see [Control Flow](/manual/04-control-flow).

## Adoption reference

| Declaration | Adopts with | Checked at compile time |
|---|---|---|
| `trait` (methods with bodies) | `class B with Clickable` | always compiles; missing methods mix in from the trait |
| one interface | `class U : Summarizable` | every signature implemented (in body, mixed in, or inherited) |
| several interfaces | `class U : A, B` | every signature of every interface |
| trait plus interface | `class U with T : I` | trait fills defaults, interface checks signatures |
| `extends` plus traits | `class C extends Base with T` | one parent, any number of traits |

```rnx
trait Named {
    fn label(): String { return "unnamed"; }
}

interface Summarizable {
    fn summary(): String;
}

class User with Named : Summarizable {
    let name: String;
    init(name: String) { this.name = name; }
    fn summary(): String { return this.name; }
}

let u = new User("Al");
assert(u.label() == "unnamed", "trait default");
let s: Summarizable = u;
assert(s.summary() == "Al", "interface dispatch");
assert(s is Summarizable, "membership");
assert(typeOf(s) == "User", "concrete class");
```

Methods the class defines override trait defaults; trait `let` fields with defaults carry into each adopting class the same way. A class may adopt several traits and interfaces beside a single `extends` parent. Calls through an interface resolve the runtime class id to a direct static call per implementation — no vtables, identical on every backend.

## Summary

- `trait` declares a method set with bodies; classes adopt with `with`, mixing in defaults they do not override.
- `interface` declares signatures only; classes adopt with `:` (or `with`) and values type by the interface.
- Dispatch resolves class ids to static calls; no vtables.
- `is` tests and narrows; `typeOf` reports the concrete class.
