---
title: "Prelude and Intrinsics"
description: "The implicit environment: resolution order, root intrinsics, and core types."
section: "Syntax and Primitives"
icon: "Sparkles"
---

# Prelude and Intrinsics

## The implicit environment

Every Rasmalai source file compiles inside an implicit environment: names resolve through local scopes first, then package globals and compiler builtins, then `@std/prelude`. No import is required for any of these names. Explicit declarations and imports always shadow prelude names with no error.

```rnx
struct Map {
    let x: Int;
}

let o = Map(1);
print(o.x);
```

The local `struct Map` replaces the prelude `Map` for the declaring scope. Shadowing applies per scope; other files are unaffected.

## Root intrinsics

Three functions are compiler builtins, available without declaration or import:

| Intrinsic | Signature | Effect |
|---|---|---|
| `print` | `print(...values)` | Formats each value and writes the line to standard output. |
| `assert` | `assert(condition: Bool, message: String)` | Aborts with file and line diagnostics when `condition` is `false`. |
| `typeOf` | `typeOf(value)` | Returns the runtime type tag, or the class identifier for objects. |

```rnx
print("values:", 40 + 2, true);
assert(1 + 1 == 2, "arithmetic");
print(typeOf(42), typeOf("s"));
```

Conditional aborts use the `PanicIf!` macro form, which panics with the message when the condition holds:

```rnx
PanicIf!(false, "unreached");
print("alive");
```

There is no bare `panic(...)` function; calling one is an `E303` error. Failure inside fallible functions uses `throw`; see [Functions and Closures](/manual/05-functions-and-closures).

`print` is a prelude shim over `io.write`: every argument runs through the `@std/io` pretty-printer — see [Hardware and FFI](/manual/13-hardware-and-ffi) — so existing programs keep working with no import.

## Foundation types

These types resolve from the prelude in every file:

| Type | Meaning |
|---|---|
| `Int` | 64-bit signed integer. |
| `Float` | Strict IEEE-754 double. |
| `FastFloat` | Same 64 bits with reassociation permitted. |
| `Bool` | `true` or `false`. |
| `String` | UTF-8 text; literals are immortal. |
| `Char` | Single scalar view into a `String`. |
| `Array` | Dense growable sequence. |
| `Map` | Insertion-ordered key-value table. |
| `Set` | Distinct-member collection. |
| `Date` | Calendar view over the wall clock. |
| `Error` | Error value. |
| `Any` | Untyped escape hatch. |
| `Void` | Absence of a value. |
| `GenRef` | Generational reference for cyclic edges. |

Numeric semantics are specified in [Numeric Model](/manual/02-numeric-model); memory behavior in [Memory and ARC](/manual/11-memory-and-arc).

## Nullable values and Result

There is no `Option` type. A missing value is `null` (type `Null`), and a nullable slot is written `T?`: `Map.get(key)` yields `V?`, `Array.find(pred)` yields `T?`, and an empty `pop()` yields `null`. `??` substitutes a fallback:

```rnx
let port: Int? = null;
assert((port ?? 8080) == 8080, "fallback");
let ok: Result<Int, String> = Result.Ok(42);
print(ok.unwrap());
```

`Result` is a real generic enum merged into every program: `Ok` carries a payload and `Err` carries an error. Its query methods (`isOk`, `isErr`, `unwrap`, `unwrapOr`) lower to discriminant checks. The `?`, `??`, and `?.` channels over nullables and `Result` are specified in [Functions and Closures](/manual/05-functions-and-closures).

## Iteration traits

```rnx
interface Iterator<T> { fn next(): T?; }
interface Iterable<T> { fn iterator(): Iterator<T>; }
```

`for..in` over any `Iterable` desugars to a `while` loop over `next()`; see [Extensions and Operators](/manual/09-extensions-and-operators).

## Array extensions

Arrays carry compiler-provided core methods plus prelude extension methods, all without import. `length` works as a call and as a property.

```rnx
let arr = [10, 20, 30];
arr.push(40);
print(arr.length, arr.pop());
let doubled = arr.map((x: Int) => x * 2);
let evens = arr.filter((x: Int) => x % 2 == 0);
print(doubled.length, evens.length);
```

Search and fold helpers:

```rnx
let numbers = [10, 20, 30, 40, 50];
assert(numbers.find((n) => n > 25) == 30, "find");
assert(numbers.findIndex((n) => n == 40) == 3, "findIndex");
assert(numbers.some((n) => n == 30), "some");
assert(numbers.every((n) => n > 0), "every");
assert(numbers.reduce(0, (acc, n) => acc + n) == 150, "reduce");
assert(numbers.join(",") == "10,20,30,40,50", "join");
assert(numbers.reversed()[0] == 50, "reversed");
```

`find` returns the first match as `T?` (`null` when nothing matches); `findIndex` returns the index or `-1`; `some`/`every` short-circuit (`some` is `false` and `every` is `true` on empty arrays); `reduce<U>` folds left with an independent accumulator type; `join` stringifies with a separator; `reversed` returns a reversed copy.

## Array method reference

| Method | Signature | Empty-array result | Short-circuit |
|---|---|---|---|
| `find` | `find(pred: fn(T): Bool): T?` | `null` | stops at first match |
| `findIndex` | `findIndex(pred: fn(T): Bool): Int` | `-1` | stops at first match |
| `some` | `some(pred: fn(T): Bool): Bool` | `false` | stops at first `true` |
| `every` | `every(pred: fn(T): Bool): Bool` | `true` | stops at first `false` |
| `reduce<U>` | `reduce<U>(initial: U, f: fn(U, T): U): U` | `initial` | never; visits all |
| `join` | `join(separator: String = ""): String` | `""` | never; visits all |
| `reversed` | `reversed(): Array<T>` | `[]` | n/a (copy) |

```rnx
let empty: Array<Int> = [];
assert(empty.find((n) => n > 0) == null, "find empty");
assert(empty.findIndex((n) => n > 0) == -1, "findIndex empty");
assert(empty.some((n) => true) == false, "some empty");
assert(empty.every((n) => false) == true, "every empty");
assert(empty.reduce(9, (acc, n) => acc + n) == 9, "reduce empty");
assert(empty.join(",") == "", "join empty");
assert([3, 1].reversed()[0] == 1, "reversed");
assert([1, 2, 3].join() == "123", "join default");
```

The predicate runs once per element in index order. `reduce<U>` takes an independent accumulator type `U`, so the accumulator need not match the element type.

```rnx
let words = ["a", "bb"];
assert(words.reduce(0, (acc, w) => acc + w.length()) == 3, "fold types");
```

## Result method reference

| Method | Signature | `Ok(v)` | `Err(e)` |
|---|---|---|---|
| `isOk` | `isOk(): Bool` | `true` | `false` |
| `isErr` | `isErr(): Bool` | `false` | `true` |
| `unwrap` | `unwrap(): T` | `v` | aborts with a fatal diagnostic |
| `unwrapOr` | `unwrapOr(fallback: T): T` | `v` | `fallback` |

```rnx
let ok: Result<Int, String> = Result.Ok(42);
assert(ok.isOk() && !ok.isErr(), "ok flags");
assert(ok.unwrap() == 42, "unwrap ok");
assert(ok.unwrapOr(-1) == 42, "unwrapOr ok");
let bad: Result<Int, String> = Result.Err("down");
assert(bad.isErr() && !bad.isOk(), "err flags");
assert(bad.unwrapOr(-1) == -1, "unwrapOr err");
```

## Intrinsic reference

| Intrinsic | Signature | Failure mode |
|---|---|---|
| `print(...values)` | any count, any types | never fails; joins with spaces plus trailing newline |
| `assert(cond, msg)` | `assert(Bool, String)` | aborts with file and line when `cond` is `false` |
| `typeOf(value)` | any value | never fails; returns the runtime type tag or class identifier |
| `PanicIf!(cond, msg)` | `PanicIf!(Bool, String)` | aborts when `cond` is `true` |

```rnx
print("one", 2, false);
PanicIf!(1 + 1 == 3, "math broke");
print(typeOf(42));
print(typeOf("s"));
```

## String extensions

Core methods (`length()`, `slice`, `indexOf`, `trim`, `concat`, `charCodeAt`) are specified in [Lexicon and Structure](/manual/01-lexicon-and-structure). Search and transform helpers arrive via the prelude with no import:

```rnx
let greeting = "hello world from rasmalai";
assert(greeting.contains("world"), "contains");
assert(greeting.startsWith("hello"), "prefix");
assert(greeting.endsWith("rasmalai"), "suffix");
let parts = greeting.split(" ");
assert(parts.length() == 4 && parts[0] == "hello", "split");
assert("foo bar foo".replace("foo", "baz") == "baz bar foo", "replace");
assert("foo bar foo".replaceAll("foo", "baz") == "baz bar baz", "replaceAll");
assert("na ".repeat(3) == "na na na ", "repeat");
assert("hello".toUpperCase() == "HELLO", "toUpperCase");
assert("HeLLo".toLowerCase() == "hello", "toLowerCase");
```

`split("")` splits into one string per character; `replaceAll` with an empty target returns the string unchanged. `toUpperCase` and `toLowerCase` fold ASCII letters only, so digits and punctuation stay put and accented Latin passes through untouched: `"héllo".toUpperCase()` gives `"HéLLO"`.

## Scope-builtin types

`IO`, `Thread`, `ThreadPool`, `Pointer`, and `Address` resolve as compiler builtins alongside the prelude. Threading is specified in [Concurrency and Threads](/manual/14-concurrency-and-threads); raw pointers and C-ABI in [Hardware and FFI](/manual/13-hardware-and-ffi).

## Summary

- Resolution order: local scopes, then globals and builtins, then `@std/prelude`; explicit declarations shadow prelude names.
- Root intrinsics: `print`, `assert`, `typeOf`, plus the `PanicIf!` macro form. No bare `panic()` exists.
- Foundation types, `null`/`T?` and `Result`, and iteration traits resolve everywhere.
- Array and String extensions (`find`, `reduce`, `contains`, `split`, and siblings) require no import.
