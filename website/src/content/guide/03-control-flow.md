---
title: "Control Flow"
description: "Branches, loops, switch patterns, and defer."
icon: "Split"
---

# Control Flow

You only need a handful of shapes to direct a program: `if`, one kind of `for`, `while`, and `switch`, plus `defer` for cleanup. Each one does a single job.

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

Test for presence with `!= null`, which narrows the binding to the payload type inside the block:

```rnx
let v: Int? = 3;
if v != null {
    print("got {v}");
} else {
    print("empty");
}
```

## for is the only loop with a header

`for x in ...` iterates arrays. There is intentionally no C-style `for (init; cond; step)` — writing one is a compile error that suggests `for x in range` or `while`:

```rnx
let total = 0;
for x in [10, 20, 30] {
    total = total + x;
}
print(total);
```

Anything with an `.iterator()` method works too — maps iterate keys, sets iterate members:

```rnx
let seen = 0;
for x in [1, 2, 3].reversed() {
    seen = seen + x;
}
print(seen);
```

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

Switching over an enum with every variant covered needs no `default` — the checker knows the match is exhaustive. A non-exhaustive enum `switch` without `default` is a compile error naming the missing variants:

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

## defer runs at scope exit

`defer` schedules a block to run when the current scope exits — normal return, early `return`, or `throw`. Multiple defers run last-in, first-out:

```rnx
defer { print("closed"); }
defer { print("flushed"); }
print("working");
```

Output order is `working`, `flushed`, `closed`. If you come from Go, this is the `defer` you know; if you come from Rust, it is `Drop` written out in the open, in order.

> [!TIP]
> Use `defer` for every acquire/release pair — files, locks, timers. The cleanup sits next to the acquisition instead of pages away at the function end.

Next: [Functions and Closures](/guide/04-functions-and-closures) — signatures, lambdas, defaults, and throwing functions.
