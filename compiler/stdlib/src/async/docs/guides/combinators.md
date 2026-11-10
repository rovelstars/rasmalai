---
title: "Combinators"
description: "Run several promises with all, allSettled, race, and any."
section: Combinators
order: 40
---

# Combinators

A combinator takes an array of promises and returns one promise for the combined outcome. The inputs are left untouched, so the same promises can feed more than one combinator. The four combinators answer different questions: did every input succeed, what did each input do, which input settled first, and which input succeeded first.

## Wait for every payload

`Promise.all<T>(promises)` fulfills with the payloads in input order, whatever order the inputs settle in. The first rejection rejects the result, and later settlements are ignored. An empty array fulfills with an empty array.

```rnx
let ps: Array<Promise<Int>> = [Promise.resolve(1), Promise.resolve(2)];
let all: Result<Array<Int>, String> = Promise.all<Int>(ps).wait();
let got: Array<Int> = all.unwrap();
println!(got.join(","));
```

The program prints `1,2`. The order comes from the input array, not from settlement order.

## Inspect every outcome

`Promise.allSettled<T>(promises)` never rejects. It fulfills with one `PromiseResult<T>` per input, in input order. Each record has a `status` of `"fulfilled"` or `"rejected"`, a `value` payload, and a `reason` message. A fulfilled record uses an empty reason, and a rejected record uses `null` for its value.

```rnx
let ps: Array<Promise<Int>> = [Promise.resolve(5), Promise.reject<Int>("bad")];
let all: Result<Array<PromiseResult<Int>>, String> = Promise.allSettled<Int>(ps).wait();
let got: Array<PromiseResult<Int>> = all.unwrap();
println!(got[0].status);
println!(got[1].reason);
```

The program prints `fulfilled` and then `bad`. An empty array fulfills with an empty array.

## Take the first settlement

`Promise.race<T>(promises)` settles with whichever input settles first, fulfillment or rejection alike. Later settlements are ignored. The losing inputs keep running, so do not hand `race` work that must stop when it loses.

```rnx
let ps: Array<Promise<Int>> = [Promise.resolve(1), Promise.resolve(2)];
println!(Promise.race<Int>(ps).wait().unwrap());
```

The program prints `1`. An empty array never settles, so `wait()` on the result returns `Err` after its five-second timeout.

## Take the first payload

`Promise.any<T>(promises)` fulfills with the first payload to arrive. It rejects only after every input has rejected, and then with `"AggregateError: all promises rejected"`. An empty array rejects at once with `"AggregateError: no promises"`.

```rnx
let mixed: Array<Promise<Int>> = [Promise.reject<Int>("no"), Promise.resolve(5)];
println!(Promise.any<Int>(mixed).wait().unwrap());
```

The program prints `5`, because the rejected input does not stop the fulfilled one from winning.
