---
title: "Functions and Closures"
description: "Signatures, arrow lambdas, default arguments, and throwing functions."
icon: "Braces"
---

# Functions and Closures

Functions are where your code lives: how you reuse logic, how you test it, and how the compiler thinks about your program.

## Declaring and calling

Parameters pair names with types, and the return type follows a colon. Omitting it means the function returns `Void` (nothing useful):

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

Arguments evaluate left to right. A function that declares `: Int` must `return` an `Int` on every path — the checker catches mismatches before codegen.

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

A mandatory parameter after a defaulted one is an error, as are unknown or duplicate names.

## Arrow lambdas

A lambda is `(params) => body`, or the single-identifier shorthand `x => body`. The body is one expression (implicit return) or a block. Closures capture their environment by value and are first-class — pass them around, store them, call them anywhere:

```rnx
let factor = 10;
let mult = (x: Int) => x * factor;
print(mult(5));
```

Arrays consume closures directly — this is the shape you will use daily:

```rnx
let nums = [1, 2, 3, 4, 5];
let doubled = nums.map((x: Int) => x * 2);
let evens = nums.filter((x: Int) => x % 2 == 0);
print(doubled);
print(evens);
```

> [!NOTE]
> `fn(...) => ...` is rejected: `fn` declares named functions with block bodies only. Lambdas never use the `fn` keyword.

Parameter annotations can be omitted when the call site already knows the shape — `map` and `filter` infer the element type from the receiver:

```rnx
let nums = [1, 2, 3];
print(nums.map((n) => n * 2));
```

## Generic functions infer from arguments

A function can declare type parameters in angle brackets. At each call the compiler solves them from the argument types — one definition serves every type with no duplicate code:

```rnx
fn identity<T>(val: T): T {
    return val;
}

print(identity(42));
print(identity("hi"));
```

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

Next: [Data Structures](/guide/05-data-structures) — structs, classes, records, and enums.
