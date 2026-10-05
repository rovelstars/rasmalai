---
title: "Control Flow"
description: "Branches, loops, switch patterns, and defer."
icon: "Split"
---

# Control Flow

You only need a handful of shapes to direct a program: `if`, one kind of `for`, `while`, and `switch`, plus `defer` for cleanup. Each one does a single job — once you have seen all five, you have seen them all.

## if and the ternary

`if` takes a `Bool` — never an integer, never a nullable. For value-level choices, the ternary `cond ? a : b` keeps one-liners flat:

```rnx
let score = 42;
if score > 40 {
    print("high");
} else {
    print("low");
}
let verdict = score > 90 ? "S" : "A";
print(verdict);
```

That prints `high` and then `A`. Test for presence with `!= null`, which narrows the binding to the payload type inside the block:

```rnx
let v: Int? = 3;
if v != null {
    print("got {v}");
} else {
    print("empty");
}
```

Inside the `if` branch, `v` is a plain `Int` — you can call methods on it without unwrapping. (The `Int?` syntax means "an `Int` or `null`"; the full story is in [Error Handling](/guide/07-error-handling).)

## for is the only loop with a header

`for x in ...` iterates arrays — and anything with an `.iterator()` method, like map keys or set members. There is intentionally no C-style `for (init; cond; step)`:

```rnx
let total = 0;
for x in [10, 20, 30] {
    total = total + x;
}
print(total);
```

That prints `60`. Need a range or a stride? Ranges are values too:

```rnx
for i in 0..10 {
    print(i);
}
for i in (0..10).stride(2) {
    print(i);
}
```

**Common mistake:** writing `for (let i = 0; i < 3; i += 1) { }` out of habit. The compiler rejects it and suggests `for x in range` or `while` — take the suggestion. If you need the index alongside the element, iterate a range and index in:

```rnx
let names = ["al", "ore", "dust"];
for i in 0..names.length() {
    print("${i}: ${names[i]}");
}
```

Destructuring works when the elements are pairs — `for (k, v) in m.items()` walks a map entry by entry (see [Collections](/guide/06-collections)).

## while for everything else

Arbitrary conditions and custom steps belong in `while`. `break` leaves the loop, `continue` starts the next iteration, and both run any pending `defer` blocks on the way out:

```rnx
let i = 0;
while true {
    i = i + 1;
    if i % 2 == 0 {
        continue;
    }
    if i >= 5 {
        break;
    }
    print(i);
}
```

That prints `1` and `3`: when `i` reaches `5` the `break` fires before the `print`. Trace it by hand if the order surprises you — following values through a loop on paper is a skill worth building early.

## switch matches patterns

`switch` matches values against literals, ranges, `is Type` checks, and enum variants with a leading dot. Each case auto-breaks — crossing into the next case needs an explicit `fallthrough;`, so the classic C fallthrough bug cannot happen by accident:

```rnx
fn level(score: Int): String {
    switch score {
        case 0..=10: return "low";
        case 11..=90: return "mid";
        default: return "high";
    }
}

print(level(42));
print(level(5));
```

That prints `mid` and `low`. Switching over an enum with every variant covered needs no `default` — the checker knows the match is exhaustive. A non-exhaustive enum `switch` without `default` is a compile error naming the missing variants, which is genuinely helpful: add a variant to the enum and the compiler lists every `switch` you must update.

```rnx
enum Shape { Circle(Float), Point }

fn describe(s: Shape): String {
    switch s {
        case .Circle(r): return "circle r=${r}";
        case .Point: return "point";
    }
}

print(describe(Shape.Circle(2.0)));
```

Variants construct with a dot (`Shape.Circle(...)`) and match with a leading dot (`.Circle(r)`), binding payloads inline. Guards refine a case further: `case is Foo if x > 1` only matches when both hold.

## defer runs at scope exit

`defer` schedules a block to run when the current scope exits — normal return, early `return`, or `throw`. Multiple defers run last-in, first-out:

```rnx
defer { print("closed"); }
defer { print("flushed"); }
print("working");
```

The relative order is `working`, `flushed`, `closed` — defers wait until the scope ends, then run back-to-back in reverse. If you come from Go, this is the `defer` you know; if you come from Rust, it is `Drop` written out in the open, in order. You will use it for files, locks, and anything else that must be released no matter how the scope ends.

Next: [Functions and Closures](/guide/04-functions-and-closures) — signatures, lambdas, defaults, and generics.
