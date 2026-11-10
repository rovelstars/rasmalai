---
title: "Chaining"
description: "Transform a fulfillment, recover from a rejection, and run cleanup with then, catchReject, and finallyDo."
section: Chaining
order: 30
---

# Chaining

A promise chain lets each step work on the value the previous step produced. `then` transforms a fulfillment. `catchReject` turns a rejection back into a value. `finallyDo` runs cleanup on either outcome. Each method returns a new promise and leaves the original alone, so a chain settles independently at every link.

## Transform the payload

```rnx
let a: Result<Int, String> = Promise.resolve(20).then<Int>((v: Int): Int => v + 1).wait();
println!(a.unwrap());

let b: Result<Int, String> = Promise.reject<Int>("bad").then<Int>((v: Int): Int => v + 1).wait();
println!(b.isErr());
```

The first line prints `21`. The second prints `true`. `then` runs its closure on a fulfillment and settles the new promise with the closure's result. A rejection skips the closure and reaches the new promise unchanged, which is why the second chain is still rejected.

## Recover from a rejection

`catchReject` is the counterpart. A fulfillment passes through with its payload. A rejection runs the closure, and the new promise fulfills with the closure's result.

```rnx
let r: Result<Int, String> = Promise.reject<Int>("x").catchReject<Int>((e: String): Int => 99).wait();
println!(r.unwrap());
```

The program prints `99`. The chain recovered instead of staying rejected. Use it at the point where a failure has a sensible fallback.

## Run cleanup

`finallyDo` runs one closure on either outcome and carries the original settlement forward. The closure runs before the new promise settles, so its output lands before the value you read from `wait()`.

```rnx
let r: Result<Int, String> = Promise.resolve(7).finallyDo(() => println!("done")).wait();
println!(r.unwrap());
```

The program prints `done` and then `7`.

## Put the three together

```rnx
async fn load(): Promise<Int> {
    return 20;
}

fn Main(): Int {
    let doubled: Promise<Int> = load().then<Int>((v: Int): Int => v * 2);
    let recovered: Promise<Int> = doubled.catchReject<Int>((e: String): Int => 0);
    let cleaned: Promise<Int> = recovered.finallyDo(() => println!("finished"));
    let r: Result<Int, String> = cleaned.wait();
    println!(r.unwrapOr(-1));
    return 0;
}
```

The chain doubles `20`, keeps a fallback for a rejection, prints `finished`, and then prints `40`. The original `Promise<Int>` from `load()` is untouched; each method builds a new promise for the next link.
