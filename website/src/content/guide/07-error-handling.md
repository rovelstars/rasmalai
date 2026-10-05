---
title: "Error Handling"
description: "Absence with null and T?, Result, throws and try/catch, and the null-safe operators."
icon: "TriangleAlert"
---

# Error Handling

There is no doubt you will hit failures while writing rnx: missing files, missed lookups, bad input. While failure values are good at telling you something went wrong, many people are stumped by the three different channels and when to use which. Don't worry — this chapter covers all of it: diagnosing failures, identifying where they come from, and handling them.

## Kinds of failure

Not every failure deserves the same machinery. rnx has three channels, smallest first:

- **Absence (`null` / `T?`)** — a lookup missed, an optional config is unset. No stack, no handler; the type itself says "might be missing". Use this when missing is ordinary.
- **`Result<T, E>`** — a computation failed but the program continues: parsing, validation, division. The error travels as a value you inspect. Use this when the caller decides what to do.
- **`throws` + `try`/`catch`** — something exceptional crossed several layers: I/O blew up, a resource vanished. Control jumps to the nearest handler. Use this when intermediate layers have nothing useful to add.

On top of all three sit hard fatals — division by zero, out-of-bounds indexing, a failed `assert`. Fatals abort the program by design: they mean a bug, not a condition to handle. Don't try to catch them; fix them.

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

That prints `10`, `8080`, and `-1`. `lhs ?? rhs` yields `lhs` when it is not `null` and evaluates `rhs` otherwise — `rhs` never runs when `lhs` is present, so fallbacks chain: `a ?? b ?? 8080`.

`?.` reaches through a chain, short-circuiting the whole thing on `null`:

```rnx
record Profile(name: String)
record User(profile: Profile?)

let u = User(Profile("Al"));
print(u?.profile?.name ?? "anonymous");
let ghost = User(null);
print(ghost?.profile?.name ?? "anonymous");
```

That prints `Al` and `anonymous`. The chain yields `null` (not a wrapper) when any link is missing, so the final `?? default` is the idiomatic landing.

> [!NOTE for TypeScript Devs]
> `?.` looks like optional chaining and mostly behaves like it — except there is no `Option` type and no `Some`/`None`. Write the nullable type directly (`String?`, `Int?`, `Profile?`).

Test for presence with `!= null`, which narrows the binding to the payload type inside the block:

```rnx
let s: String? = "hi";
if s != null {
    print(s.length());
}
```

That prints `2` — inside the branch, `s` is a plain `String`. The early-return form is `guard let`, which binds the payload or diverges through `else`:

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

That prints `8`, then `missing` and `-1`. After the `guard`, `v` is a plain `Int` for the rest of the function — no nesting, no pyramid.

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

That prints `5`, `true`, and `-1`. `unwrap` yields the payload and aborts on `Err` — only call it when you have checked first. `unwrapOr` yields the payload or the fallback. Postfix `?` unwraps inside a function that itself returns a `Result`, returning the `Err` early to the caller:

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

That prints `6` — the `?` unwrapped `5`, added one, re-wrapped. `?` binds tighter than binary operators. Using it on a non-`Result` value, or inside a function returning something else, is a compile error.

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

That prints `42` and then `failed`. `throw` carries any value. A bare `throw;` rethrows the current error, and is legal only directly inside `catch`.

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

That prints `parse problem`. Not every function should handle what it can fail at — declaring `throws` and letting the value propagate keeps intermediate layers free of plumbing they have nothing to add to.

## Common errors and how to fix them

**A nullable where a plain value was expected.** You passed a `T?` to something wanting `T`. The fix is almost always `?? default`, `guard let`, or an `if x != null` check at the boundary — not sprinkling `unwrap()` everywhere. Reserve `unwrap` for places you have proven non-null.

**`?` outside a `Result` function.** Postfix `?` only works inside a function returning `Result` with a matching error type. Anywhere else — including `fn Main(): Int` — it is a compile error. Handle the `Result` with `unwrapOr` or `if r.isOk()` instead.

**Catching fatals.** Division by zero, out-of-bounds indexing, failed asserts — these abort, full stop. If your program dies on one, the fix is in your logic (check the divisor, check the length), not in a handler.

**Swallowing errors silently.** An empty `catch (err) { }` hides the very information you need next week. At minimum, `print` the error; better, handle it or propagate it. Future you will thank present you.

Next: [Modules and Packages](/guide/08-modules-and-packages) — imports, the standard library tour, and the project manifest.
