---
title: "Bindings and Scope"
description: "let and const bindings, lexical scopes, shadowing, tuples, and destructuring."
---

# Bindings and Scope

## let and const

`let` declares a mutable binding; `const` declares an immutable one. Reassignment of a `const` binding is a compile error. There is no `mut` keyword: mutability is the default for `let`.

```rnx
let score = 0;
score = score + 10;
const maxRetries: Int = 3;
print(score, maxRetries);
```

Compound assignment (`+=`, `-=`, `*=`, `/=`, `%=`, `<<=`, `>>=`, `>>>=`) updates a binding in place and is available on variables, fields, and index targets: `score += 10` adds and stores back. Prefix and postfix `++`/`--` adjust `Int` and `Float` operands by `1` (`1.0` for `Float`); the prefix form yields the new value while the postfix form yields the old one:

```rnx
let n = 10;
let old = n++;
print(old, n);
print(++n, n--, n);
```

The operand must be a valid assignment target — a variable, a field (`obj.count++`), an index (`buf[i] += step`), or a dereferenced pointer (`(*p)++` inside `unsafe`). Mutating a `const` binding or incrementing a non-target such as `(a + b)` is an `E108` error. Compound index and member targets evaluate their base and index exactly once, so `items[next()]++` calls `next()` a single time.

Names resolve lexically, top to bottom: a name must be declared before use. Reference to a name with no visible declaration is an `E303` error pointing at the exact column. There are no implicit globals; a misspelled assignment target fails instead of creating state.

Top-level statements run inside the implicit entrypoint, so a top-level `let` is local to it: a named `fn` cannot read it, and the compiler says so with an `E303` that names sharing as the fix. A value shared across functions is a `const` — immutable, visible in every function body, and restricted to constant initializers (`Int`, `Float`, `Bool`, and plain-text `String` literals, optionally composed from earlier consts):

```rnx
const dbPath = "notes.txt";

fn load(): String {
    return dbPath;
}
print(load());
```

Type annotations (`: Int`) are optional on bindings. Annotations on `const` items and function signatures convert would-be runtime confusion into compile-time errors.

## Lexical scopes

Every block introduces a scope. A binding is visible from its declaration to the end of the enclosing block. Bindings in inner blocks shadow outer bindings of the same name for the remainder of the inner block; the outer binding is unaffected.

```rnx
let x = 1;
if true {
    let x = 2;
    print(x);
}
print(x);
```

Function parameters behave as bindings scoped to the function body.

## Tuples

A tuple groups a fixed number of values without declaring a type: `(100, "items")` is a pair holding an `Int` and a `String`. Elements are addressed positionally with `.0`, `.1`, and unpacked with a pattern. Tuples cross function boundaries as parameters and returns, passed by value with no heap allocation.

```rnx
fn div_rem(num: Int, den: Int): (Int, Int) {
    return (num / den, num % den);
}

let pair = (100, "items");
let (total, unit) = pair;
print(total, pair.1);
let (q, r) = div_rem(10, 3);
print(q, r);
```

## Destructuring

Records and structs destructure by field name, with `: alias` renames. A `...rest` binding collects every field not named explicitly.

```rnx
record User(name: String, age: Int)

let user = User("Al", 30);
let { name, age: userAge } = user;
print(name, userAge);
```

```rnx
let user = { id: 1, name: "Al", role: "admin" };
let { id, ...meta } = user;
print(id, meta.name);
```

Arrays destructure positionally with an optional trailing rest binding. Short arrays yield typed defaults for missing positional slots (`0`, `false`, `""`) and an empty rest.

```rnx
let a = [1, 2];
let full = [0, ...a, 3];
let [first, ...rest] = full;
print(first, rest.length);
```

`...` also spreads one array into another at construction. Destructuring in `for` bindings follows the same rules; see [Control Flow](/manual/04-control-flow).

## Arrays

`[10, 20, 30]` builds a reference-counted array; `Array<Int>` annotates the element type when inference needs help. Elements are addressed with `arr[i]`, assigned with `arr[i] = v`, counted with `arr.length`, extended with `arr.push(v)`, and removed with `arr.pop()`.

Array element types are invariant when widening: a bare `Array` stores boxed elements while `Array<Int>` stores raw values, so passing a typed array to a bare-`Array` parameter (or binding one to a bare-`Array` variable) is an `E108` error — annotate both sides with the same element type. Narrowing from a bare `Array` to `Array<Int>` compiles but is unchecked, like an `as` cast: only do it when the array actually holds values of that type.

```rnx
let arr = [10, 20, 30];
arr.push(40);
arr[0] = 11;
print(arr.length, arr[0], arr[3]);
print(arr.pop(), arr.length);
```

Out-of-bounds access aborts. Indexing with a range slices: arrays yield arrays, strings yield strings, and out-of-range bounds clamp. Printing an array renders its contents (`[10, 20, 30]`); `join` renders contents as flat text with a custom separator.

```rnx
let a = [10, 20, 30, 40];
print(a[1..3].length);
print(a[..2].length);
print("hello"[1..4]);
```

## Summary

- `let` mutates, `const` does not; undeclared names are `E303` errors.
- Blocks scope bindings; inner bindings shadow outer ones.
- Tuples group positionally; records, structs, and arrays destructure by name or position with `...rest` collection.
- Arrays index, slice, and spread as language operations. The functional suite (`find`, `reduce`, and siblings) belongs to the implicit prelude and is specified in [Prelude and Intrinsics](/manual/01a-prelude-and-intrinsics).
