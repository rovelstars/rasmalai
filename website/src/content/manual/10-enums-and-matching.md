---
title: "Enums and Matching"
description: "Tagged unions, payload binding, and exhaustive pattern matching."
section: "Type System and Object Model"
icon: "Dices"
---

# Enums and Matching

## Declaring variants

Each variant names a shape, optionally carrying payloads in parentheses. Type parameters are permitted and are erased at runtime.

```rnx
enum Shape { Circle(Float), Rect(Float, Float), Point }
```

Construction mirrors the declaration: `Shape.Circle(2.0)`, `Color.Green`, `Result.Err("x")`. Unit variants are bare idents; payload variants call like functions. Qualified construction always uses a dot: `Result.Ok` and `Result.Err`, never `::`.

Two positions already name the enum, so only the variant has to follow the leading dot: a variable annotation, and a call argument whose parameter type the signature writes.

```rnx
let r: Result<Int, String> = .Err("bad");
print(r.isErr());
```

Argument inference reaches the same way through a method, a `static fn`, and an `init` parameter:

```rnx
enum Color { Red, Green, Blue }

fn paint(c: Color): Int {
    switch c {
        case .Red: return 1;
        case .Green: return 2;
        case .Blue: return 3;
    }
}

class Canvas {
    let mode: Color;
    init(c: Color) { this.mode = c; }
    fn setMode(c: Color): Int { return paint(c); }
    static fn withColor(c: Color): Int { return paint(c); }
}

let canvas = new Canvas(.Blue);
print(paint(.Green), canvas.setMode(.Red), Canvas.withColor(.Green));
```

That prints `2 1 2`. A name the enum does not declare is a compile-time `error[E108]: unknown variant`, and the message names both the variant and its enum. `let bg: Color = .Nope;` and `paint(.Nope)` both fail that way, with a pointer at the dot.

Library enums infer the same way. File modes in `@std/fs` take `OpenMode` and `WriteMode`, so call sites pass only the variant:

```rnx
import { File } from "@std/fs";

let w = File.create("/tmp/rnx-dot.txt", .Overwrite);
w.writeText("hi");
w.close();
let r = File.open("/tmp/rnx-dot.txt", .Read);
print(r.readText().unwrapOr("?"));
r.close();
```

That prints `hi`.

## Matching exhaustively

`switch` over an enum value pairs each shape with behavior. Variant patterns lead with a dot and bind payloads to names. Covering every variant with unguarded cases needs no `default`: the checker proves exhaustiveness, and adding a variant later without updating the `switch` fails compilation naming the missing case.

```rnx
enum Shape { Circle(Float), Rect(Float, Float), Point }

fn area(s: Shape): Float {
    switch s {
        case .Circle(r): return 3.14 * r * r;
        case .Rect(w, h): return w * h;
        case .Point: return 0.0;
    }
}

print(area(Shape.Circle(2.0)));
print(area(Shape.Rect(3.0, 4.0)));
```

Guards narrow a case further (`case .Circle(r) if r > 0`) and observe payload bindings; `default` catches everything the listed patterns miss. Payload arity is checked at construction: `Circle(1.0, 2.0)` against a one-payload variant fails immediately.

## Matching beyond enums

The same statement matches literal ranges and dispatches on error values. Type tests use `if value is Type`, which branches on the runtime type — including the payload bound in a `catch` block, where a passing test narrows the binding so fields are accessible:

```rnx
fn kind(code: Int): String {
    switch code {
        case 0..=99: return "info";
        case 200..=299: return "ok";
        default: return "other";
    }
}

print(kind(42), kind(404));
```

```rnx
class IoError {
    let message: String;
    init(message: String) { this.message = message; }
}

fn Main(): Int {
    let e = new IoError("disk gone");
    if e is IoError {
        print("io");
    }
    try {
        throw new IoError("disk gone");
    } catch (err) {
        switch err {
            case is IoError:
                print("caught io");
                pass;
            default:
                print("caught other");
                pass;
        }
    }
    return 0;
}
```

## Destructuring in for bindings

`for (a, b) in pairs` destructures each element: array elements by index, record and struct elements by field name. Anything else at runtime is a fatal error, and `for (a, b)` over a range is a compile error rather than a silent misread.

## Enum reference

Leading-dot inference applies in exactly these positions — everywhere else, qualify with the enum name:

| Position | Example | Rule |
|---|---|---|
| variable annotation | `let r: Result<Int, String> = .Err("bad");` | annotation names the enum |
| call argument | `paint(.Green)` | parameter type names the enum |
| method argument | `canvas.setMode(.Red)` | receiver signature names the enum |
| `static fn` argument | `Canvas.withColor(.Green)` | signature names the enum |
| `init` argument | `new Canvas(.Blue)` | `init` parameter names the enum |
| library enum argument | `File.create(p, .Overwrite)` | `OpenMode`/`WriteMode` parameter names the enum |

Exhaustiveness rules:

| Match shape | Needs `default`? | Rule |
|---|---|---|
| every variant, unguarded cases | no | checker proves it; adding a variant later fails compilation naming the missing case |
| every variant, one case guarded | yes | a guard may reject, so the match is no longer proven |
| subset of variants | yes | `default` catches the rest |
| non-enum scrutinee (literals, ranges) | yes | always carry `default` |

```rnx
enum Shape { Circle(Float), Rect(Float, Float), Point }

fn area(s: Shape): Float {
    switch s {
        case .Circle(r): return 3.14 * r * r;
        case .Rect(w, h): return w * h;
        case .Point: return 0.0;
    }
}

assert(area(Shape.Circle(1.0)) > 3.0, "payload bind");
assert(area(Shape.Point) == 0.0, "unit variant");
```

Payload arity is checked at construction: `Circle(1.0, 2.0)` against the one-payload variant fails immediately, and a name the enum does not declare fails as `error[E108]: unknown variant` naming both the variant and its enum. Query methods on `Result` (`isOk`, `isErr`, `unwrap`, `unwrapOr`) are specified in [Prelude and Intrinsics](/manual/01a-prelude-and-intrinsics).

## Summary

- Enums declare shapes with payloads; construction mirrors declaration with dot qualification.
- `switch` with leading-dot patterns, guards, auto-break, and provable exhaustiveness.
- `is Type` tests for mixed values; destructuring `for` bindings for pairs.
