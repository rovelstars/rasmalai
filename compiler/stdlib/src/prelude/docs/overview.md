# @std/prelude

Names every compilation unit sees without an import.

An explicit declaration or an import always shadows a prelude name. The
class stubs below are catalog-only: the frontend parses them once per
session as a fallback scope and emits no code for them. Three items are
real, though. `Result` is a generic enum merged into every program as
`std.prelude.Result`, `Promise` and `ArrayIter` carry their bodies here,
and the `Array` and `String` extensions below are compiled into every
program too. `Map` and `Set` are stubs; their bodies live in
`@std/collections`.

```rnx
print("no import needed");
let found = [1, 2, 3].find((n) => n > 1);
print(found.unwrap(), typeOf(found));
```

## What is inside

- `Int`
- `Float`
- `FastFloat`
- `Bool`
- `String`
- `Char`
- `Array`
- `Map`
- `Set`
- `Date`
- `Error`
- `Any`
- `Void`
- `GenRef`
- `Result`
- `ArrayIter`
- `PromiseResult`
- `PromiseWithResolvers`
- `Promise`
- `RnxHost`

## When to use

The prelude is already imported everywhere: every file sees Option, Result, Array, Map, print, assert, and the Array/String extension methods with no import. Reference @std/prelude explicitly only to disambiguate a shadowed name.

It covers: Int/Float/Bool/String/Array/Map/Set; print/assert/typeOf intrinsics; Option/Result/Iterator; Array/String extensions.
