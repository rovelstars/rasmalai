---
title: "Functions and Closures"
description: "Signatures, arrow lambdas, default arguments, and throwing functions."
icon: "Braces"
---

# Functions and Closures

Functions are where your code lives: how you reuse logic, how you test it, and how the compiler thinks about your program. This chapter covers declaring them, calling them, and the two compact forms — lambdas and generics — you will use daily.

## Declaring and calling

Parameters pair names with types, and the return type follows a colon. Leave it off and the function returns `Void` (nothing useful):

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

That prints `hello, ore` and `42`. Two rules to internalize now: arguments evaluate left to right, and a function declaring `: Int` must `return` an `Int` on every path — the checker catches mismatches before any code runs.

## Default values and named arguments

Trailing parameters can declare defaults, filled in when the caller omits them. Any argument can also be passed by name. Defaults compile away — the callee always receives plain positional values:

```rnx
fn configure(host: String, port: Int = 8080, secure: Bool = false): String {
    let scheme = secure ? "https" : "http";
    return "${scheme}://${host}:${port}";
}

print(configure("api.dev"));
print(configure("api.dev", secure: true));
print(configure("api.dev", port: 9000, secure: true));
```

That prints `http://api.dev:8080`, `https://api.dev:8080`, and `https://api.dev:9000`. Named arguments shine at call sites with several same-typed parameters — `port: 9000` cannot be silently swapped with another `Int` the way a bare positional can.

**Common mistake:** putting a mandatory parameter after a defaulted one, or misspelling a name. Both are compile errors — unknown or duplicate names fail loudly rather than binding to the wrong slot.

## Arrow lambdas

A lambda is `(params) => body`, or the single-identifier shorthand `x => body`. The body is one expression (implicit return) or a block. Closures capture their environment by value and are first-class — pass them around, store them, call them anywhere:

```rnx
let factor = 10;
let mult = (x: Int) => x * factor;
print(mult(5));
```

That prints `50`. Arrays consume closures directly — this is the shape you will use daily:

```rnx
let nums = [1, 2, 3, 4, 5];
let doubled = nums.map((x: Int) => x * 2);
let evens = nums.filter((x: Int) => x % 2 == 0);
print(doubled);
print(evens);
```

That prints `[2, 4, 6, 8, 10]` and `[2, 4]`. Parameter annotations can be omitted when the call site already knows the shape — `map` and `filter` infer the element type from the receiver:

```rnx
let nums = [1, 2, 3];
print(nums.map((n) => n * 2));
```

> [!NOTE]
> `fn(...) => ...` is rejected: `fn` declares named functions with block bodies only. Lambdas never use the `fn` keyword. If you see an error about `fn` where you wrote an arrow, delete the `fn`.

## Generic functions infer from arguments

A function can declare type parameters in angle brackets. At each call the compiler solves them from the argument types — one definition serves every type with no duplicate code:

```rnx
fn identity<T>(val: T): T {
    return val;
}

print(identity(42));
print(identity("hi"));
```

That prints `42` and `hi`. You do not write the type arguments at the call site; the compiler reads them off the values you pass.

## Throwing functions

A function that can fail marks `throws` — bare, with no error set. Callers must be inside `try` or declare `throws` themselves, and the checker enforces it. The full protocol is covered in [Error Handling](/guide/07-error-handling):

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

That prints `42` — the `catch` never fires. Flip the argument to `false` and you get `failed` instead.

Next: [Data Structures](/guide/05-data-structures) — structs, classes, records, and enums.
