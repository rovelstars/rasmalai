---
title: "Structs and Records"
description: "Value types: struct layout and lifecycle, positional records, anonymous records."
section: "Type System and Object Model"
icon: "Boxes"
---

# Structs and Records

## Structs

A `struct` declares fields and the methods operating on them. Construction is a direct call with positional arguments in field order (the free memberwise initializer); there is no `new` keyword. Assignment copies by value.

```rnx
struct Vec3 {
    let x: Float;
    let y: Float;
    let z: Float;
}

let v = Vec3(1.0, 2.0, 3.0);
print(v.x, v.y, v.z);
```

Fields default to package visibility. `private` restricts a field to its enclosing scope; reading a `private` field from outside is an `E203` error. Structs accept type parameters in angle brackets; parameters erase to `Any` in field slots, so one declaration serves every element type with a single shared layout.

```rnx
struct Box<T> {
    let value: T;
}

let b = Box(42);
print(b.value);
```

## init and deinit

`init(params)` runs at construction when the memberwise default is insufficient: validation, derived fields, resource acquisition. `deinit` runs at destruction for manual teardown. `.init()` is never written; the construction call invokes it.

```rnx
struct Meter {
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

let m = Meter(20);
print(m.bump(22));
```

## Records

A `record` declares a value type in one line: positional construction, copy-by-value assignment, the same field access as `struct`, but no methods and no `init`. Records carry zero ARC overhead.

```rnx
record Point(x: Float, y: Float)

let a = Point(1.0, 2.0);
let b = a;
print(b.x, b.y);
```

Records serve coordinates, colors, and messages: data flowing through functions without attached behavior.

## Anonymous records

Anonymous records skip the declaration: `{ id: 1, name: "Al" }` builds a record value inline with field access and destructuring. A `...rest` binding collects every field not named explicitly.

```rnx
let user = { id: 1, name: "Al", role: "admin" };
let { id, ...meta } = user;
print(id, meta.name);
```

## Summary

- `struct`: fields, methods, free memberwise construction, `init`/`deinit` lifecycle, generic parameters erased to `Any`.
- `record`: one-line positional value types with copy semantics and no methods.
- Anonymous records build inline and destructure with `...rest` collection.
