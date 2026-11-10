---
title: "Getting started with @std/async"
description: "Import Promise, await an async function, and bridge to synchronous code with wait."
section: Getting started
order: 10
---

# Getting started with @std/async

There is no install step. The package ships inside the compiler. Import it when you want the types by name, or rely on the stock prelude, which re-exports `Promise`, `PromiseResult`, and `PromiseWithResolvers`:

```rnx
import { Promise } from "@std/async";
```

## Copy this

Save it as the entry file of a project and run `rnx run`, or start with `rnx init` and replace `src/main.rnx`:

```rnx
import { Promise } from "@std/async";

async fn answer(): Promise<Int> {
    return 42;
}

fn Main(): Int {
    let value: Int = answer().wait().unwrapOr(0);
    println!(value);
    return 0;
}
```

## What to expect

The program prints `42` and exits with status 0. `answer()` returns a `Promise<Int>`. `wait()` parks the calling thread until the promise settles and returns `Result<Int, String>`. `unwrapOr(0)` takes the payload, or `0` if the promise rejected.

`wait()` is the bridge from ordinary code. Inside another `async fn`, use `await` instead.
