---
title: "@std/async"
description: "One-shot promises, settlement handles, chaining helpers, and combinators."
section: Overview
order: 0
---

# @std/async

`@std/async` owns the `Promise<T>` type, the manual settlement handles from `Promise.withResolvers`, and the `PromiseResult` records returned by `Promise.allSettled`. A promise settles once. Fulfillment carries a payload. Rejection carries a string reason.

`Promise` is the type an `async fn` returns and the value `await` settles. The stock prelude re-exports `Promise`, `PromiseResult`, and `PromiseWithResolvers`, so programs on the stock prelude need no import. A prelude that does not declare `Promise` disables async lowering, and the compiler reports `E108` naming the prelude and the missing name.

```rnx
async fn answer(): Promise<Int> {
    return 42;
}

fn Main(): Int {
    println!(answer().wait().unwrapOr(0));
    return 0;
}
```

The program prints `42`. `answer()` returns a fulfilled promise, `wait()` blocks for the settlement, and `unwrapOr(0)` supplies a default for a rejection.

The package has three jobs. It creates promises through `Promise.resolve`, `Promise.reject`, and `Promise.withResolvers`. It chains work with `then`, `catchReject`, and `finallyDo`. It runs several promises together with `all`, `allSettled`, `race`, and `any`. Every combinator returns a new promise and leaves its inputs untouched.

## Guides

[Settlement](guides/settlement) covers the promise states, manual handles, `attach`, and `wait`. [Chaining](guides/chaining) covers `then`, `catchReject`, and `finallyDo`. [Combinators](guides/combinators) covers `all`, `allSettled`, `race`, and `any`. [Async functions](guides/async-functions) covers `async fn`, `await`, and the language rules around them.
