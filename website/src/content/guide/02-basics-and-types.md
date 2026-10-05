---
title: "Basics and Types"
description: "Bindings, unified 64-bit numbers, strings, and operators."
icon: "Hash"
---

# Basics and Types

Two topics, learned together: giving names to values, and numbers that mean exactly what they say. Nearly every beginner surprise lives in this chapter, so read it even if the rest of the guide feels familiar.

## Naming values with let and const

`let` declares a binding you can reassign; `const` declares one you cannot. Names must be declared before use — there are no implicit globals, so a typo fails fast instead of becoming mystery state:

```rnx
let score = 0;
score = score + 10;
const label = "score:";
print(label, score);
```

That prints `score: 10`. Type annotations are optional on bindings but always allowed — put them where they document intent:

```rnx
const maxRetries: Int = 3;
let attempt = 0;
while attempt < maxRetries {
    attempt = attempt + 1;
}
print("tried", attempt, "times");
```

> [!NOTE]
> `let` is mutable by default — the opposite of Rust. Reach for `const` when you mean it: the compiler rejects any later assignment, which turns whole classes of accidents into compile errors.

**Common mistake:** reassigning a `const`, or assigning a different type to a `let`. Bindings keep the type they are born with — `let a = 2; a = "hello";` does not compile. When you see a type error on assignment, check the first line, not the failing one.

## One integer type: Int

`Int` is always a signed 64-bit whole number on every platform. There is no platform-dependent `int`, no `long` that changes size between operating systems, no `u8`/`i32` family to juggle for everyday counting. The range is roughly ±9.2e18 — if you outgrow that, you will know.

Arithmetic wraps on overflow with two's-complement semantics. Division or remainder by zero is a runtime fatal — never undefined behavior, never a quietly wrong answer:

```rnx
print(41 + 1);
print(7 / 2);
print(7 % 3);
```

That prints `42`, `3` (integer division truncates), and `1`. Convert a float with `Int(x)`, which truncates toward zero, and render a number with `toString()`:

```rnx
print(Int(2.9));
print(Int(-2.9));
```

Both print as you would expect: `2` and `-2`.

> [!WARNING]
> `Int()` on a `String` is an error, not a parse. Convert explicitly before doing arithmetic on CLI arguments or parsed text — there is no silent string-to-number coercion anywhere in the language.

## Two float types, on purpose

`Float` is an IEEE-754 double — the strict, predictable default. `FastFloat` is the same storage with relaxed optimization rules for hot numeric code. You opt in explicitly, and the two never mix in one expression:

```rnx
let c: Float = 10.5 * 2.0 + 20.25;
print(c);
let scaled = c.asFast() * 2.0.asFast();
print(scaled == 82.5.asFast(), scaled.asStrict());
```

`asFast()` moves to relaxed mode, `asStrict()` comes back. If the compiler complains about mixing `Float` and `FastFloat` (error `E305`), name the conversion you mean instead of guessing — that explicitness is the whole point.

Handy extras on `Float`: `Float.nan()`, `Float.isNaN(x)`, `Float.fma(a, b, c)` for a fused multiply-add, and `toBits()` / `Float.fromBits(bits)` for bit-level round trips.

## Strings and interpolation

`String` holds UTF-8 text. `${...}` inside a string evaluates an expression inline, and `+` concatenates. `{...}` interpolates the same way — the two forms are interchangeable, so pick one per file and stay with it. A literal brace is `\{`:

```rnx
let name = "ore";
let level = 7;
print("player ${name} reached level ${level + 1}");
print("sum {level + 1}");
print("ab" + "cd");
```

`+` with a `String` on either side concatenates, converting the other operand: `"5" + 1` is `"51"`, not `6`. Useful measurements on strings: `length()` (characters, not bytes), `slice(start, end)`, `indexOf(needle)`, `trim()`, and `charCodeAt(index)`. The full toolkit (`contains`, `split`, `replace`, and friends) lives in [Collections](/guide/06-collections) — for now, interpolation plus `print` covers most programs.

## Truth values

`Bool` has exactly two values, `true` and `false`, and conditions demand it. Comparison operators (`==`, `!=`, `<`, `<=`, `>`, `>=`) produce `Bool`, and `&&`, `||`, `!` combine them:

```rnx
let hp = 30;
let shielded = false;
if hp > 0 && !shielded {
    print("vulnerable");
}
```

**Common mistake:** writing `if 1 { }` or `if name { }` out of habit from C or Python. rnx conditions take a `Bool` and nothing else — say what you mean (`if hp > 0`, `if name != ""`).

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
