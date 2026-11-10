# @std/prelude

Names every compilation unit sees without an import.

An explicit declaration or an import always shadows a prelude name. The
foundation types come from `@std/foundation`: `Int`, `Float`, `FastFloat`,
`Bool`, `String`, `Char`, `Array`, `Map`, `Set`, `Error`, `Any`, `Void`,
`GenRef`, `Result`, `Iterator`, `Iterable`, and `ArrayIter`, plus the Array
and String extension methods. This package re-exports those names and adds
`Date` from `@std/time`, `Promise` and friends from `@std/async`, `RnxHost`
from `@std/env`, the print and log macros, and the `IO`, `Thread`, and
`ThreadPool` catalog stubs.

`Map` and `Set` are catalog stubs here; their bodies live in
`@std/collections`. A target prelude can replace this package and provide
only the catalog it supports.

```rnx
println!("no import needed");
let found = [1, 2, 3].find((n) => n > 1);
println!(found ?? -1, typeOf(found));
```

## What is inside

Foundation re-exports:

- `Int`, `Float`, `FastFloat`, `Bool`, `String`, `Char`
- `Array`, `Map`, `Set`, `Error`, `Any`, `Void`, `GenRef`
- `Result`, `Iterator`, `Iterable`, `ArrayIter`

Prelude-only names:

- `Date`
- `Promise`, `PromiseResult`, `PromiseWithResolvers`
- `RnxHost`
- `print`, `println`, `eprint`, `eprintln`
- `log`, `debug`, `info`, `warn`, `error`
- `IO`, `Thread`, `ThreadPool`

## When to use

The prelude is already imported everywhere: every file sees the foundation
types, `Date`, `Promise`, the Array and String extension methods, `assert`,
and `typeOf` with no import. Reference `@std/prelude` explicitly only to
disambiguate a shadowed name.

A barebones target ships its own prelude package and selects it through the
project alias or the target descriptor. The catalog names must still be
present; `@uefi/prelude` is the worked example.
