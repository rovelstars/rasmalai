---
title: "Async functions"
description: "Write async fn, await a promise, and know when to bridge with wait."
section: Async functions
order: 50
---

# Async functions

An `async fn` declares cooperative work. Its body runs as a promise, and `await` suspends until the promise settles. This is the one place the language suspends rather than blocks. `Thread.spawn` plus `join()` blocks the calling thread; `await` lets the current task resume when the value is ready.

## Declare and await

```rnx
async fn compute(): Promise<Int> {
    return 42;
}

async fn total(): Promise<Int> {
    let value: Int = await compute();
    return value + 1;
}

fn Main(): Int {
    println!(total().wait().unwrapOr(0));
    return 0;
}
```

The program prints `43`. `compute()` returns a `Promise<Int>`, and `await compute()` resumes with the `Int` payload. A rejection from `compute()` would reject `total()` in turn.

## Top-level await

The entry file may hold loose statements with no `fn Main` wrapper. They run in lexical order inside a synthesized `async fn Main(): Promise<Int>`, so top-level `await` works:

```rnx
async fn compute(): Promise<Int> {
    return 42;
}

println!(await compute());
```

The program prints `42`. A top-level `return <Int>` sets the process exit code, and falling through returns `0`. `Main` may itself be `async`, and the compiler uses its resolved value as the exit code. Files reached through `import` stay declarative, so an executable statement there is an error.

## When to use wait

`wait()` is the bridge from synchronous code. It blocks the calling OS thread and returns `Result<T, String>`. Inside an `async fn`, use `await` instead. Calling `wait()` there is a compile error, because parking an event-loop task would deadlock.

`await` outside an `async fn` is `E109`, except at the top level of the entry file. Direct `.poll()` calls are `E111`; the polling model is retired.

## Promise must come from the prelude

Async lowering needs the selected prelude to declare `Promise`. A prelude that omits it makes `async fn` and `await` fail with `E108`, naming the prelude and the missing name. The stock prelude provides it.
