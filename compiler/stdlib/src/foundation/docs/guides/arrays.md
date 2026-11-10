---
title: "Arrays"
description: "Grow, index, map, filter, search, fold, and join an Array."
section: Arrays
order: 30
---

# Arrays

An `Array<T>` is a dense sequence with `Int` indexes. Building one and calling `length`, `push`, `pop`, `map`, or `filter` needs no import: those are compiler builtins. The search and fold helpers are extension methods declared in `@std/foundation`, and the stock prelude also brings them into scope.

## The built-in operations

```rnx
let xs = [1, 2, 3];
xs.push(4);
println!(xs.length());
println!(xs.map((x: Int): Int => x * 10).join(","));
println!(xs.filter((x: Int): Bool => x > 2).join(","));
```

That prints `4`, then `10,20,30,40`, then `3,4`. `map` builds a new array from the closure's return value. `filter` keeps the elements the closure accepts and keeps their original type. `length()` and `len()` do the same thing, and both also read as properties.

`pop()` removes the last element and returns it. On an empty array it returns `null`:

```rnx
let xs = [1, 2, 3];
println!(xs.pop() ?? 0);
println!(xs.length());
```

The first line prints `3` and the second prints `2`. `isEmpty()` answers whether the length is zero.

An index past the end throws `index out of bounds`. Check the length first when an index comes from user input or a loop counter.

## Search, fold, and join

```rnx
let scores = [4, 9, 16];

println!(scores.find((x: Int): Bool => x > 5) ?? -1);
println!(scores.findIndex((x: Int): Bool => x > 5));
println!(scores.some((x: Int): Bool => x == 9));
println!(scores.every((x: Int): Bool => x > 0));
println!(scores.reduce<Int>(0, (acc: Int, x: Int): Int => acc + x));
println!(scores.reversed().join(" | "));
```

The lines print `9`, `1`, `true`, `true`, `29`, and `16 | 9 | 4`. `find` returns the first match or `null`. `findIndex` returns its index or `-1`. `some` is false for an empty array and `every` is true for one. `reduce` takes an explicit result type, so write `reduce<Int>` when the accumulator starts as `0`.

`join` takes an optional separator and defaults to an empty string. `reversed` returns a new array and leaves the original alone.

## A real case

```rnx
fn labelFor(scores: Array<Int>, target: Int): String {
    let at = scores.findIndex((x: Int): Bool => x == target);
    if at == -1 {
        return "missing";
    }
    return "found at " + at;
}

println!(labelFor([4, 9, 16], 9));
println!(labelFor([4, 9, 16], 7));
```

The first call prints `found at 1` and the second prints `missing`. `findIndex` removes the need to write the search loop by hand, and the `-1` result is the signal for an absent value.
