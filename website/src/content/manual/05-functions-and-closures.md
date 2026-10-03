---
title: "Functions and Closures"
description: "Signatures, defaults, generics, lambdas, methods, throws, and failure channels."
---

# Functions and Closures

## Declaration and calls

Parameters pair names with types; the return type follows a colon. A function without a return type returns `Void`. Arguments evaluate left to right. A function declaring `: Int` returns an `Int` on every path; any other type is an `E304` error with `expected`/`got` labels.

```rnx
fn add(a: Int, b: Int): Int {
    return a + b;
}

fn greet(name: String) {
    print("hello,", name);
}

greet("ore");
print(add(20, 22));
```

Return type inference applies where the body determines the type; explicit annotations are required on `pub` signatures and recommended everywhere a mismatch would otherwise surface at a distance.

## Default values and named arguments

Trailing parameters declare defaults, filled in when the caller omits them. Any argument may pass by name after the positional ones. A mandatory parameter after a defaulted one is an error, as are unknown or duplicate names. Methods follow the same rules. Defaults compile away: the callee always receives plain positional values.

```rnx
fn configure(host: String, port: Int = 8080, secure: Bool = false): String {
    let sec = "http";
    if (secure) {
        sec = "https";
    }
    return "${sec}://${host}:${port}";
}

print(configure("api.dev"));
print(configure("api.dev", secure: true));
print(configure("api.dev", port: 9000, secure: true));
```

## Generic functions

Type parameters in angle brackets solve from argument types at each call, or pass explicitly. Parameters erase to a shared representation, so one definition serves every type. A parameter that cannot be inferred, or two arguments forcing different types onto one parameter, is a compile error naming the parameter.

```rnx
fn identity<T>(val: T): T {
    return val;
}

print(identity(42));
print(identity("hi"));
```

## Lambdas and closures

A lambda is `(params) => body`, with the single-identifier shorthand `x => body`. The body is one expression (implicit return) or a block. `fn(...) => ...` is rejected with `E105`: `fn` declares named functions with block bodies only.

```rnx
let factor = 10;
let mult = (x: Int) => x * factor;
assert(mult(5) == 50, "capture");
```

Closures capture their environment by value and are first-class: they pass as arguments, store in fields, and call anywhere. Captures keep heap values alive. Parameter annotations may be omitted when the call site determines the shape: `map`/`filter` infer the element type from the receiver, and plain functions infer from `fn`-typed parameters.

```rnx
fn apply(x: Int, f: fn(Int): Int): Int {
    return f(x);
}

let nums = [1, 2, 3];
assert(nums.map((n) => n * 2)[2] == 6, "inferred map");
assert(apply(5, (n) => n * 2) == 10, "inferred HOF");
```

A closure stored on its own object requires `fn decay(this)`; see [Cycles and Handles](/manual/12-cycles-and-handles).

## Methods

Functions defined inside a `class` or `struct` take an implicit receiver and call with dot syntax. Inside the body, `this` refers to the receiver. `init(params)` runs at construction (`new Meter(20)` calls it; `.init()` is never written). `static` methods call on the type itself with no receiver.

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

## throws and try..catch

A function that can fail marks `throws`, bare, with no error set. Callers are inside `try` or declare `throws` themselves; the checker enforces the contract. `throw` carries any value. A bare `throw;` rethrows the current error and is legal only directly inside `catch`.

```rnx
fn load(ok: Bool): Int throws {
    if ok { return 42; }
    throw 0;
}

try {
    print(load(true));
} catch (err) {
    print("failed");
}
```

Calling a `throws` function from another function propagates on every backend through a per-thread error slot; the nearest enclosing `catch` receives the payload.

`try` accepts an optional single `catch (id)` plus an optional `finally`. The `finally` block runs on normal exit, error return, and nested rethrow alike; it desugars to a synthetic `defer` and shares those exact semantics. `defer` blocks registered between a `throw` and its `catch` run first, innermost first.

```rnx
fn Main(): Int {
    let log: Array<String> = [];
    try {
        defer log.push("inner");
        throw "down";
    } catch (err) {
        log.push("caught: " + err);
    }
    print(log[0]);
    print(log[1]);
    return 0;
}
```

A `throw` propagates on every backend, across function boundaries, through a per-thread error slot. Thrown class instances keep their identity: `is` checks match (including ancestors) and narrow the binding, so fields are accessible in the arm. An error that escapes all handlers fails the entry call. `switch` over the caught value with `case is Type:` arms dispatches the same way.

```rnx
class IoError {
    let message: String;
    init(message: String) { this.message = message; }
}

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
        switch err {
            case is IoError:
                print("caught io");
                pass;
            case is ParseError:
                print("caught parse");
                pass;
            default:
                print("caught other");
                pass;
        }
    }
    return 0;
}
```

Functions that cannot usefully decide an error declare `throws` and propagate; handling belongs at the boundary where retry, default, or abort-with-context is available.

## Nullable and Result channels: ? ?? ?.

`Result` values unwrap with postfix `?`. `Ok` yields the payload; `Err` returns early carrying the error, so the enclosing function must return a `Result`. `?` binds tighter than binary operators and never steals ternary colons. Applying `?` to a non-`Result` value, or inside a function returning something else, is a compile error.

```rnx
fn popIt(): Result<Int, String> {
    let arr = [10, 20];
    let v = arr.pop() ?? -1;
    return Result.Ok(v + 1);
}

print(popIt().unwrap());
```

`lhs ?? rhs` yields `lhs` when it is not `null` and evaluates `rhs` otherwise; `rhs` never runs when `lhs` is present. Either side may be nullable, so fallbacks chain.

```rnx
let port: Int? = null;
assert((port ?? 8080) == 8080, "fallback");
let a: Int? = null;
let b: Int? = 42;
assert((a ?? b ?? 8080) == 42, "chain");
```

`target?.field` and `target?.method(args)` short-circuit a chain: a `null` anywhere yields `null` without touching the rest. `?.` must directly touch its operands.

```rnx
record Profile(name: String)
record User(profile: Profile?)

let user = User(Profile("Al"));
assert(user?.profile?.name == "Al", "chain");
let ghost = User(null);
assert((ghost?.profile?.name ?? "anonymous") == "anonymous", "fallback");
```

## Test functions

Functions marked `test fn` take no parameters, return nothing, and are stripped from normal builds. The test runner discovers them, times them, and reports each one. `assert` takes a `Bool` and a message. Test execution is specified in [Project and Toolchain](/manual/16-project-and-toolchain).

```rnx
test fn adds_up() {
    assert(1 + 1 == 2, "math");
}
```

## Summary

- `fn name(params): Ret` with block bodies and `return`; `(params) => body` lambdas capturing by value.
- Trailing defaults with positional-or-named calls; generics solved per call.
- Methods use `this`; `init` constructs; `static` needs no receiver.
- `throws` marks fallibility; `try`/`catch`/`finally` handle; `throw` carries values.
- `?` early-returns through `Result`; `??` falls back; `?.` reaches through.
- `test fn` marks tests; mismatched returns are `E304`.
