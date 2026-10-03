---
title: "Data Structures"
description: "Structs, classes, records, and enums."
---

# Data Structures

Sooner or later you need to keep related values together. Rasmalai gives you four shapes: `struct` for data with behavior, `class` for shared identity, `record` for plain values, and `enum` for closed alternatives.

## Structs group fields and methods

A `struct` declares fields and the methods that operate on them, with a free memberwise initializer in field order:

```rnx
struct Vec3 {
    let x: Float;
    let y: Float;
    let z: Float;
}

let v = Vec3(1.0, 2.0, 3.0);
print(v.x, v.y, v.z);
```

`init(params)` runs at construction when you need more than the memberwise default — validation, derived fields, resource acquisition — and `deinit` runs at destruction for manual teardown.

## Classes add identity

`class` is the reference type: instances live on the heap under ARC, `extends` takes one parent class, and `with` mixes in traits. Reach for `class` when instances need shared identity (two handles, one object) or polymorphism; reach for `struct` for plain data:

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

Functions defined inside a type take an implicit receiver and are called with dot syntax. Inside the body, `this` refers to the receiver. `static` methods call on the type itself with no receiver, like `Vec4f.splat(0.0)` or `AtomicInt.byId(1)`.

## Records are values, plain and simple

A `record` declares a value type in one line: positional construction, copy-by-value assignment, the same field access as `struct`, but no methods, no `init`, zero ARC overhead:

```rnx
record Point(x: Float, y: Float)

let a = Point(1.0, 2.0);
let b = a;
print(b.x, b.y);
```

Use records for coordinates, colors, messages — data that flows through functions without ever needing behavior attached. Anonymous records skip the declaration entirely:

```rnx
let user = { id: 1, name: "Al" };
print(user.id, user.name);
```

## Enums and pattern matching

An `enum` lists closed alternatives, each optionally carrying a payload. `switch` over an enum with every variant covered needs no `default` — the checker proves exhaustiveness:

```rnx
enum Shape { Circle(Float), Rect(Float, Float), Point }

fn area(s: Shape): Float {
    switch s {
        case .Circle(r): return 3.14 * r * r;
        case .Rect(w, h): return w * h;
        case .Point: return 0.0;
    }
}

print(area(Shape.Rect(3.0, 4.0)));
```

Variants construct with a dot (`Shape.Rect(...)`) and match with a leading dot (`.Rect(w, h)`), binding payloads inline.

## Traits and extensions

A `trait` declares a method set that classes adopt with `with` — generic code depends on behavior, not concrete types. An `extension` block attaches methods to an existing type from outside, with zero runtime cost:

```rnx
extension String {
    fn double(): String {
        return this + this;
    }
}

print("hi".double());
```

Types also overload operators by defining `op_add`, `op_sub`, `op_mul`, `op_div`, `op_index`, and friends — `a + b` desugars statically to `a.op_add(b)`. The exact desugaring table is normative in the [Language Manual](/manual/09-extensions-and-operators).

Next: [Collections](/guide/06-collections) — arrays, functional transforms, maps, and sets.
