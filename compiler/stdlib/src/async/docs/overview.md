# @std/async

`@std/async` owns `Promise`, `PromiseWithResolvers`, and `PromiseResult`.
`Promise` is the type an `async fn` returns and the value `await` settles.
The stock prelude re-exports all three, so programs on the desktop prelude
can use `Promise` without an import.

A prelude that does not declare `Promise` disables async lowering. The
compiler reports `E108` naming the prelude and the missing name.

```rnx
async fn answer(): Promise<Int> {
    return 42;
}

fn Main(): Int {
    println!(answer().wait().unwrapOr(0));
    return 0;
}
```

## What is inside

- `Promise`
- `PromiseWithResolvers`
- `PromiseResult`

## When to use

Use `@std/async` when a function returns a value later: an `async fn`
returns a `Promise`, `await` settles it, and the combinators in this package
run several promises together. The stock prelude re-exports the types, so
desktop programs need no import.
