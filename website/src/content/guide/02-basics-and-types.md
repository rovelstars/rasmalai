---
title: "Basics and Types"
description: "Bindings, unified 64-bit numbers, strings, and operators."
---

# Basics and Types

Two topics, learned together: giving names to values, and numbers that mean exactly what they say. Nearly every beginner surprise lives here, so read this chapter even if the rest of the guide feels familiar.

## Bindings with let and const

`let` declares a mutable binding, `const` an immutable one. Names must be declared before use — there are no implicit globals, so typos fail fast instead of becoming mystery state:

```rnx
let score = 0;
score = score + 10;
const label = "score:";
print(label, score);
```

Type annotations are optional on bindings but always allowed. Put them where they document intent:

```rnx
const maxRetries: Int = 3;
let attempt = 0;
while attempt < maxRetries {
    attempt = attempt + 1;
}
print("tried", attempt, "times");
```

> [!NOTE]
> `let` is mutable by default — the opposite of Rust. Reach for `const` when you mean immutable.

## One integer type: Int

`Int` is always 64 bits on every platform. There is no platform-dependent `int`, no `long` that changes size between operating systems, and no `u8`/`i32` family to juggle for everyday counting. The range is roughly +/-9.2e18 - if you outgrow that, you will know.

Arithmetic wraps on overflow with two's-complement semantics. Division or remainder by zero is a runtime fatal — never undefined behavior, never a wrong answer served quietly:

```rnx
let big: Int = 9_000_000_000_000_000_000;
print(big + 1);
print(17 / 5, 17 % 5);
print(0xFF, 0b1010);
```

Underscores group digits (`9_000`), `0x` writes hex, `0b` writes binary. The bitwise operators `&`, `|`, `^`, and unary `~` work on `Int`:

```rnx
let flags = 0b1010;
print(flags & 0b0010);
print(flags | 0b0101);
```

Shifts work on `Int` too: `<<` (left), `>>` (arithmetic right, sign-extending), `>>>` (logical right, zero-fill). All three bind tighter than comparisons but looser than `+`/`-`, and the shift amount masks to 6 bits (`x << 67` shifts by 3):

```rnx
print(1 << 3);
print((-8) >> 2);
print((-1) >>> 1);
```

Compound forms `<<=`, `>>=`, `>>>=` update in place.

## Two float types, on purpose

`Float` is strict IEEE-754 double precision: `0.1 + 0.2` behaves exactly as the standard says, on every backend. `FastFloat` is the same 64 bits with the optimizer allowed to reorder operations for speed. That freedom can change the last bit of a result, so the compiler refuses to mix the two without explicit consent:

```rnx
let strict: Float = 0.5;
let fast = strict.asFast();
print(fast * 2.0.asFast());
print(Int(3.9));
```

`.asFast()` and `.asStrict()` are zero-cost marker conversions — they change the type rules, not the bits. Converting between integers and floats uses `Int(x)` / `Float(x)`, which truncate toward zero. And one comfort: plain `Int` operands inside float arithmetic convert automatically:

```rnx
let base = 3;
let multiplier = 2.5;
print(base * multiplier);
```

> [!TIP]
> Default to `Float` everywhere. Convert to `FastFloat` with `.asFast()` at the boundary of hot numeric code, so the relaxed region is visible in review and everything else stays bit-exact.

## Strings and interpolation

`String` holds UTF-8 text. `${...}` inside a string evaluates an expression inline, and `+` concatenates. `{...}` interpolates the same way — the two forms are interchangeable, so pick one per file and stay with it. A literal brace is `\{`:

```rnx
let name = "ore";
let level = 7;
print("player ${name} reached level ${level + 1}");
print("sum {level + 1}");
print("\{\"title\": \"meeting\"}");
print("ab" + "cd");
```

`+` with a `String` on either side concatenates, converting the other operand: `"5" + 1` is `"51"`, not `6`. Convert explicitly before doing arithmetic on CLI arguments or parsed text — `Int()` on a `String` is an error, not a parse.

The full string toolkit (`contains`, `split`, `replace`, indexing) lives in [Collections](/guide/06-collections). For now, interpolation plus `print` covers most programs.

## Truth values

`Bool` has exactly two values, `true` and `false`, and conditions demand it — `if 1 { }` does not compile. Comparison operators (`==`, `!=`, `<`, `<=`, `>`, `>=`) produce `Bool`, and `&&`, `||`, `!` combine them:

```rnx
let hp = 30;
let shielded = false;
if hp > 0 && !shielded {
    print("vulnerable");
}
```

## Operators at a glance

| Group | Operators |
|---|---|
| Arithmetic | `+ - * / %` (with unary `-`) |
| Comparison | `== != < <= > >=` |
| Boolean | `&& \|\| !` |
| Bitwise | `& \| ^ ~` |
| Null-safe | `?? ?. postfix ?` (see [Error Handling](/guide/07-error-handling)) |
| Ternary | `cond ? a : b` (see [Control Flow](/guide/03-control-flow)) |

Next: [Control Flow](/guide/03-control-flow) — branches, loops, pattern matching with `switch`, and `defer`.
