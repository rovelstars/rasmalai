---
title: "Error Handling"
description: "Absence with null and T?, Result, throws and try/catch, and the null-safe operators."
---

# Error Handling

Programs fail — files go missing, lookups miss, saves corrupt. Rasmalai treats failure as typed values flowing through declared channels: never integer codes to squint at, never exceptions arriving from nowhere.

## Absence is null

A missing value is `null` (type `Null`). A type written `T?` holds either a `T` or `null`, so absence is visible in the type. Map lookups, `find`, and `pop` all return nullables:

```rnx
let nums = [10, 20];
let first = nums.find((n) => n > 5) ?? -1;
print(first);
let missing: Int? = null;
print(missing ?? 8080);
let empty: Array<Int> = [];
print(empty.pop() ?? -1);
```

`lhs ?? rhs` yields `lhs` when it is not `null` and evaluates `rhs` otherwise — `rhs` never runs when `lhs` is present, so fallbacks chain: `a ?? b ?? 8080`.

`?.` reaches through a chain, short-circuiting the whole thing on `null`:

```rnx
record Profile(name: String)
record User(profile: Profile?)

let u = User(Profile("Al"));
print(u?.profile?.name ?? "anonymous");
let ghost = User(null);
print(ghost?.profile?.name ?? "anonymous");
```

> [!NOTE for TypeScript Devs]
> `?.` looks like optional chaining and mostly behaves like it — except the chain yields `null` (not a wrapper) when any link is missing, so the final `?? default` is the idiomatic landing. There is no `Option` type and no `Some`/`None`: write the nullable type directly (`String?`, `Int?`, `Profile?`).

Test for presence with `!= null`, which narrows the binding to the payload type inside the block:

```rnx
let s: String? = "hi";
if s != null {
    print(s.length());
}
```

The early-return form is `guard let`, which binds the payload or diverges through `else`:

```rnx
import { Map } from "@std/collections";

fn greet(m: Map<String, Int>, key: String): Int {
    guard let v = m.get(key) else {
        print("missing");
        return -1;
    }
    return v + 1;
}

fn Main(): Int {
    let m = new Map<String, Int>();
    m.set("ore", 7);
    print(greet(m, "ore"));
    print(greet(m, "dust"));
    return 0;
}
```

## Failure is Result

A function that can fail returns `Result<T, E>`: `Ok` carries the value, `Err` carries the error. Both live in the prelude, so no import is needed:

```rnx
fn div(a: Int, b: Int): Result<Int, String> {
    if b == 0 {
        return Result.Err("zero");
    }
    return Result.Ok(a / b);
}

let r = div(10, 2);
if r.isOk() {
    print(r.unwrap());
}
let bad = div(1, 0);
print(bad.isErr());
print(bad.unwrapOr(-1));
```

`unwrap` yields the payload and aborts on `Err`; `unwrapOr` yields the payload or the fallback. Postfix `?` unwraps inside a function that itself returns a `Result`, returning the `Err` early to the caller:

```rnx
fn div(a: Int, b: Int): Result<Int, String> {
    if b == 0 {
        return Result.Err("zero");
    }
    return Result.Ok(a / b);
}

fn calc(): Result<Int, String> {
    let v = div(10, 2)?;
    return Result.Ok(v + 1);
}

print(calc().unwrapOr(-1));
```

`?` binds tighter than binary operators and never steals ternary colons. Using it on a non-`Result` value, or inside a function returning something else, is a compile error.

## Failure is throws

A function that can fail marks `throws` — bare, with no error set. Callers must be inside `try` or declare `throws` themselves, and the checker enforces it:

```rnx
fn load(ok: Bool): Int throws {
    if ok { return 42; }
    throw 0;
}

fn Main(): Int {
    try {
        print(load(true));
    } catch (err) {
        print("failed");
    }
    try {
        print(load(false));
    } catch (err) {
        print("failed");
    }
    return 0;
}
```

`throw` carries any value. A bare `throw;` rethrows the current error, and is legal only directly inside `catch`.

> [!NOTE]
> `try`/`catch`/`throw` lower identically on every backend: a `throws` function called from another function propagates through a per-thread error slot, and the nearest enclosing `catch` receives it. Thrown class instances keep their identity, so `is` dispatch and field access after narrowing work in the handler. An error that escapes all handlers fails the entry call (`rnx run` reports it on every backend; `rnx build` binaries print `Uncaught exception: <message>` on stderr and exit 1).

Handle the error at the boundary where a useful decision exists (retry, default, abort) and propagate everywhere else. Dispatch on the caught value with `is` checks, which narrow the binding to the tested type inside the arm:

```rnx
class ParseError {
    let message: String;
    init(message: String) { this.message = message; }
}

fn first(): String throws {
    throw new ParseError("bad header");
}

fn Main(): Int {
    try {
        print(first());
    } catch (err) {
        if err is ParseError {
            print("parse problem");
        } else {
            print("unknown");
        }
    }
    return 0;
}
```

Not every function should handle what it can fail at. Declaring `throws` and letting the value propagate keeps intermediate layers free of plumbing they have nothing to add to.

Next: [Modules and Packages](/guide/08-modules-and-packages) — imports, the standard library tour, and the project manifest.
