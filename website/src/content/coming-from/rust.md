---
title: "Coming from Rust: Systems Power Without Lifetime Puzzles"
description: "Deterministic memory, explicit cleanup, and millisecond builds for Rustaceans."
sourceLang: "rust"
---

# Coming from Rust: Systems Power Without Lifetime Puzzles

Rasmalai's memory model is reference counting with scope-end frees -
deterministic like Rust, but with no borrow checker, no lifetimes, and no
`Pin`/`unsafe` for self-referential data. Builds measure in milliseconds,
not minutes. This page covers what that trade costs: cycles need explicit
handling, and the compiler won't prove aliasing safety for you.

## Lifetimes vs lexical scopes

Rust names every borrow. Rasmalai ties each value to the block that owns it —
the scope end is the free point, visible in the source:

```rust
struct Edge<'a> {
    label: &'a str,
    weight: i64,
}

fn main() {
    let label = String::from("ore");
    let edges = [
        Edge { label: &label, weight: 0 },
        Edge { label: &label, weight: 2 },
        Edge { label: &label, weight: 4 },
        Edge { label: &label, weight: 6 },
    ];
    let mut total = 0;
    for e in edges {
        total += e.weight;
    }
    println!("{total}");
}
```

```rnx
struct Edge {
    let label: String;
    let weight: Int;
}

let edges = [Edge("ore", 0), Edge("ore", 2), Edge("ore", 4), Edge("ore", 6)];
let total = 0;
for e in edges {
    total = total + e.weight;
}
print(total);
```

Shared structure that would cycle in Rust uses `GenRef<T>` handles here:
copying one never touches atomics, and `.get()` yields `null` once the owner
frees. No annotations, no borrowck fight.

## Drop vs defer

Rust runs `Drop::drop` at scope end. Rasmalai runs `defer` blocks at scope
end, in the open, in order:

```rust
struct Conn;
struct Txn;

impl Drop for Conn {
    fn drop(&mut self) {
        println!("conn closed");
    }
}

impl Drop for Txn {
    fn drop(&mut self) {
        println!("txn committed");
    }
}

fn main() {
    let _txn = Txn;
    let _conn = Conn;
    println!("query ran");
}
```

```rnx
defer { print("conn closed"); }
defer { print("txn committed"); }
print("query ran");
```

Output order is `query ran`, `txn committed`, `conn closed`: last-in,
first-out, exactly like stacked guards.

## Pattern matching

Both languages match by structure. Rasmalai spells variants with a leading
dot and needs no `default` when the match is exhaustive:

```rust
enum Shape {
    Circle(f64),
    Rect(f64, f64),
    Point,
}

fn area(s: Shape) -> f64 {
    match s {
        Shape::Circle(r) => 3.14 * r * r,
        Shape::Rect(w, h) => w * h,
        Shape::Point => 0.0,
    }
}

fn main() {
    println!("{}", area(Shape::Rect(3.0, 4.0)));
}
```

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

## Error handling

Throwing functions mark `throws`; callers use `try` or propagate with their
own `throws`. Caught values dispatch with `switch` and `is` checks instead of
combinator chains:

```rust
fn risky(ok: bool) -> Result<i64, i64> {
    if ok {
        Ok(1)
    } else {
        Err(0)
    }
}

fn main() {
    match risky(true) {
        Ok(v) => println!("{v}"),
        Err(_) => println!("failed"),
    }
}
```

```rnx
fn risky(ok: Bool): Int throws {
    if ok { return 1; }
    throw 0;
}

try {
    print(risky(true));
} catch (err) {
    print("failed");
}
```

## Builds

`cargo check` warms up; `rnx check` finishes. The dev loop runs
unoptimized LLVM codegen: a cold dev build lands in 35-60 ms on the
current baseline
(`benches/data/benchmarks.json`, 2026-09-29, seven workloads), a hot-reload
swap turns around in about 7 ms, and `rnx check` typechecks with no codegen
at all. Release costs no extra build time and buys the optimized binary; the
proving-ground chart plots both as `rnx dev` and `rnx release`. Keep your mental
model of ownership. Lose the wait.
