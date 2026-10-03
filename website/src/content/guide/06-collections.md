---
title: "Collections"
description: "Arrays, functional transforms, Map, and Set."
---

# Collections

Three workhorses cover nearly every program: `Array` for ordered sequences, `Map` for keyed lookup, `Set` for membership. All three are in the prelude or one import away, and all three iterate with `for..in`.

## Arrays

Array literals use square brackets. Indexing starts at zero, out-of-bounds access aborts (never undefined behavior), and `length()` reports the size. Printing an array renders its contents (`[10, 20, 30, 40]`, nesting deeper than 3 shows as `...`) — `join` renders the contents as flat text with a custom separator instead:

```rnx
let nums = [10, 20, 30];
print(nums[0], nums[2], nums.length());
nums.push(40);
print(nums);
print(nums.join(", "));
```

```text
10 30 3
[10, 20, 30, 40]
10, 20, 30, 40
```

> [!NOTE]
> `pop()` on an empty array returns `null` instead of aborting — it is one of the gentlest introductions to `T?`, covered in [Error Handling](/guide/07-error-handling).

## Functional transforms

Arrays ship a full functional toolkit. `map` builds a new array, `filter` keeps matching elements, `find` returns the first match as `T?` (`null` on miss), `some`/`every` test predicates, `reduce` folds, and `join` renders text:

```rnx
let nums = [1, 2, 3, 4, 5];
print(nums.map((n) => n * 2));
print(nums.filter((n) => n % 2 == 0));
print(nums.reduce(0, (acc, n) => acc + n));
print(nums.some((n) => n > 4));
print(nums.every((n) => n > 0));
```

`find` pairs naturally with `??` for defaults, and `join` turns string arrays into output:

```rnx
let words = ["write", "it", "once"];
print(words.join(" "));
let nums = [1, 2, 3, 4];
let big = nums.find((n) => n > 10) ?? -1;
print(big);
print(nums.reversed());
```

## Strings are collections too

`String` carries the same fluent style: `contains`, `startsWith`, `endsWith`, `split`, `replace`, `repeat`, and `indexOf` with an optional start position:

```rnx
let line = "player:ore:level:7";
let parts = line.split(":");
print(parts.join(","));
print(line.contains("ore"));
print("ab".repeat(3));
```

## Maps

`Map<K, V>` is an insertion-ordered hash table over any hashable key (`Int`, `Float`, `Bool`, `String`, or object identity). Missing lookups yield `null` (`V?`) — never an exception:

```rnx
import { Map } from "@std/collections";

let scores = new Map<String, Int>();
scores.set("ore", 7);
scores.set("al", 12);
print(scores.get("ore"));
print(scores.get("nobody") ?? 0);
print(scores.keys().join(","));
print(scores.len());
```

`keys()` and `values()` return arrays in insertion order, and `for k in map` iterates keys directly.

## Sets

`Set<T>` keeps distinct members with the same ordering guarantee. Inserting twice keeps one copy:

```rnx
import { Set } from "@std/collections";

let seen = new Set<String>();
seen.add("ore");
seen.add("ore");
print(seen.has("ore"), seen.len());
for member in seen {
    print(member);
}
```

Next: [Error Handling](/guide/07-error-handling) — `null` and `T?`, `Result`, `throws`, and the `??` / `?.` operators.
