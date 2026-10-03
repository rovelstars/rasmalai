---
title: "Control Flow"
description: "if and else, loops, switch matching, defer, and guard — with exact body rules."
section: "Syntax and Primitives"
icon: "Split"
---

# Control Flow

## if and else

`if` requires a `Bool` condition. Bodies take two forms: a brace block (any statements, including declarations) or a single braceless statement. A lexical declaration (`let`) directly inside a braceless body is an `E108` error; the fix is braces.

```rnx
let score = 42;
if score > 40 {
    print("high");
} else {
    print("low");
}
if score > 0 print("positive");
else print("non-positive");
```

Test for presence with `!= null`, which narrows the binding inside the block, or diverge early with `guard let ... else`. There are no if-expressions: `if` never yields a value. The conditional expression is the ternary `cond ? a : b`.

```rnx
let acc = "";
let i = 0;
while i < 3 {
    acc = i % 2 == 0 ? acc + "e" : acc + "o";
    i = i + 1;
}
print(acc);
```

## for..in

`for x in ...` is the only loop with a header. It iterates arrays, ranges, and any value with an `.iterator()` method. Strings are not iterable: `for c in "hello"` is an `E108` error, so walk one character at a time with `for c in s.split("")`, which yields one `String` per character. There is no C-style `for (init; cond; step)`; writing one is an `E110` error suggesting `for x in range` or `while`.

```rnx
let total = 0;
for x in [10, 20, 30] {
    total = total + x;
}
print(total);
```

The binding destructures: `for (a, b) in pairs` pulls array elements by index, or record and struct elements by field name. `for (a, b)` over a range is a compile error. `Map` iterates keys, `Set` iterates members. Iteration over `.iterator()` types desugars to a `while` loop over `next()` with identical `break`/`continue` semantics; arrays and ranges keep zero-allocation fast paths.

```rnx
import { Map } from "@std/collections";

let map = new Map<String, Int>();
map.set("a", 1);
map.set("b", 2);
let count = 0;
for (k in map) {
    count = count + 1;
}
assert(count == 2, "iter keys");
```

## while and do..while

Conditions and custom steps belong in `while`. `do..while` evaluates the body once before testing the condition. `break` leaves the loop; `continue` starts the next iteration; both run pending `defer` blocks on the way out.

```rnx
let i = 0;
while i < 3 {
    i = i + 1;
}
print(i);
let j = 0;
do {
    j = j + 1;
} while j < 3;
print(j);
```

## switch

`switch` matches a scrutinee against patterns: literals, `a..=b` ranges, `is Type` checks, and enum variants with a leading dot. Guards (`if` after a pattern) narrow further and observe payload bindings. Each case terminates implicitly; crossing into the next case requires an explicit `fallthrough;`.

```rnx
fn level(score: Int): String {
    switch score {
        case 0..=10: return "low";
        case 11..=90: return "mid";
        default: return "high";
    }
}

print(level(42));
```

`is` tests a value against a type and narrows it inside the matching arm; no cast is required. `typeOf` returns the runtime type name. Identity checks compare integer tags, never strings: primitives test the `Any` discriminant, objects compare the class id in the instance header.

```rnx
let v: Any = 42;
if (v is Int) {
    print(v + 1);
}
print(typeOf(v));
print(v is String);
```

`switch` also serves as an expression: each arm yields a value, arms agree on one type, and the match is exhaustive or carries `default`.

```rnx
let code = 200;
let status = switch code {
    case 200: "OK",
    case 404: "Not Found",
    default: "Error",
};
print(status);
```

Enum matching has its own exhaustiveness rule; see [Enums and Matching](/manual/10-enums-and-matching).

## defer

`defer` schedules a block to run when the current scope exits: normal return, early `return`, `throw`, `break`, or `continue` out of loops. Multiple defers run last-in, first-out. A `defer` inside a branch registers only when the branch executes.

```rnx
defer { print("closed"); }
defer { print("flushed"); }
print("working");
```

Output order is `working`, `flushed`, `closed`. A single statement is accepted without braces.

```rnx
let trail = "";
defer print("closed");
defer trail = trail + "!";
print("working");
```

## guard

`guard let name = expr else { ... }` binds `name` for the rest of the scope when `expr` is non-`null`, and executes the `else` block when `expr` is `null`. The `else` block must diverge (`return`, `throw`, `break`, or `continue`); a non-diverging `else` is an `E108` error. `guard` tests for `null` only.

```rnx
fn pick(flag: Bool): Int {
    let v: Any = flag ? 1 : null;
    guard let x = v else { return -1; }
    return 1;
}

print(pick(true));
print(pick(false));
```

## Summary

- `if`/`else` with block or single-statement bodies; declarations in braceless bodies are `E108`.
- One `for` form (`for x in ...`); `while` and `do..while` for the rest; no C-style `for`.
- `switch` with literal, range, `is`, and variant patterns, guards, implicit termination, and an expression form.
- `defer` runs LIFO at scope exit including single-statement form; `guard` binds-or-diverges on `null`.
