---
title: "Numbers and booleans"
description: "Work with Float, FastFloat, and Bool values and their built-in methods."
section: Numbers
order: 20
---

# Numbers and booleans

`Int` is the everyday whole number. `Float` is the strict IEEE-754 double for measurements and fractions. `FastFloat` is the same storage with relaxed optimization rules for hot numeric code. `Bool` is `true` or `false`. The four types need no import, and the methods below come with the language.

## Float round trips

A `Float` converts to text with `toString()`. `toBits()` gives the raw bit pattern as an `Int`, and `Float.fromBits(bits)` rebuilds the value.

```rnx
let c: Float = 10.5 * 2.0 + 20.25;
println!(c);
println!(Float.fromBits(c.toBits()) == c);
```

The first line prints `41.25`. The second prints `true`, because the bits rebuild the same value.

`Float.nan()` builds a NaN, and `Float.isNaN(x)` tests for one:

```rnx
let missing = Float.nan();
println!(Float.isNaN(missing));
println!(Float.isNaN(0.0));
```

The lines print `true` and `false`. `Float.fma(a, b, c)` computes `a * b + c` as one fused multiply-add.

## FastFloat needs a conversion

`Float` and `FastFloat` never mix in one expression. The mix is `E305`, so name the conversion you mean. `asFast()` goes to relaxed mode and `asStrict()` comes back.

```rnx
let c: Float = 10.5 * 2.0 + 20.25;
let scaled = c.asFast() * 2.0.asFast();
println!(scaled == 82.5.asFast(), scaled.asStrict());
```

The first line of output is `true`, because both sides are `FastFloat`. The second is the same value as a strict `Float`, `82.5`. A comparison between a `Float` and a `FastFloat` needs one side converted first.

## Booleans render as words

`Bool.toString()` prints `true` or `false` rather than a number:

```rnx
println!((1 < 2).toString());
println!((1 > 2).toString());
```

The lines print `true` and `false`.
