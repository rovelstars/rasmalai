---
title: "From Rust"
description: "Lifetimes and the borrow checker become lexical scopes and ARC."
sourceLang: "rust"
---

# From Rust: Systems Power Without Lifetime Puzzles

You trust the borrow checker but you are tired of negotiating with it over self-referential structs, and tired of pinning CPU cores for minutes per build. Rasmalai keeps deterministic memory and drops both taxes. Your ownership mental model transfers almost untouched — the annotations stay behind.

## Lifetimes vs lexical scopes

Rust names every borrow. Rasmalai ties each value to the block that owns it — the scope end is the free point, visible in the source:

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

let edges = [Edge("ore", 0), Edge("ore", 2)];
let total = 0;
for e in edges {
    total = total + e.weight;
}
print(total);
```

Shared structure that would cycle in Rust uses `GenRef<T>` handles here: copying one never touches atomics, and `.get()` yields `null` once the owner frees. No annotations, no borrowck fight.

> [!TIP]
> The direction rule fits on an index card: own **down** the tree with strong references, point **back up** with `GenRef`. The compiler warns (`W108`) when two objects hold each other strongly.

## Drop vs defer

Rust runs `Drop::drop` at scope end. Rasmalai runs `defer` blocks at scope end, in the open, in order:

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

Output order is `query ran`, `txn committed`, `conn closed`: last-in, first-out, exactly like stacked guards.

## Result vs throws

Throwing functions mark `throws`; callers use `try` or propagate with their own `throws`. Caught values dispatch with `switch` and `is` checks instead of combinator chains:

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

## What changes, what stays

| Rust habit | Rasmalai equivalent |
|---|---|
| `let` immutable, `mut` to opt in | `let` mutable, `const` to opt out |
| Lifetimes on every borrow | Lexical scopes own values |
| `Drop` impls | `defer` blocks at the use site |
| `Result<T, E>` plumbing | `throws` + `try`/`catch` |
| `cargo check` warmup | `rnx check` in milliseconds |

Your ownership mental model transfers almost untouched — the annotations stay behind. Next: the [Guide](/guide/01-introduction) from the top, or the [Manual](/manual/11-memory-and-arc) for exact ARC semantics.
