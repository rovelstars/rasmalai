---
title: "Collections"
description: "Arrays, functional transforms, Map, and Set."
icon: "Type"
---

# Collections

Three workhorses cover nearly every program: `Array` for ordered sequences, `Map` for keyed lookup, `Set` for membership. All three are in the prelude or one import away, and all three iterate with `for..in`.

## Arrays

Array literals use square brackets. Indexing starts at zero, out-of-bounds access aborts (never undefined behavior), and `length()` reports the size. Printing an array renders its contents. `join` renders them as flat text with a separator instead:

```rnx
let nums = [10, 20, 30];
println!(nums[0], nums[2], nums.length());
nums.push(40);
println!(nums);
println!(nums.join(", "));
```

That prints:

```text
10 30 3
[10, 20, 30, 40]
10, 20, 30, 40
```

> [!NOTE]
> `pop()` on an empty array returns `null` instead of aborting, one of the gentlest introductions to `T?`, covered in [Error Handling](/guide/07-error-handling). Reading past the end with `[i]`, on the other hand, throws `index out of bounds`, so check the length first.

## Functional transforms

Arrays ship a full functional toolkit. `map` builds a new array, `filter` keeps matching elements, `find` returns the first match as `T?` (`null` on miss), `some`/`every` test predicates, `reduce` folds, and `join` renders text:

```rnx
let nums = [1, 2, 3, 4, 5];
println!(nums.map((n) => n * 2));
println!(nums.filter((n) => n % 2 == 0));
println!(nums.reduce(0, (acc, n) => acc + n));
println!(nums.some((n) => n > 4));
println!(nums.every((n) => n > 0));
```

That prints `[2, 4, 6, 8, 10]`, `[2, 4]`, `15`, `true`, `true`. `find` pairs naturally with `??` for defaults, and `reversed()` returns a reversed copy:

```rnx
let words = ["write", "it", "once"];
println!(words.join(" "));
let nums = [1, 2, 3, 4];
let big = nums.find((n) => n > 10) ?? -1;
println!(big);
println!(nums.reversed());
```

That prints `write it once`, `-1` (nothing matched, so the fallback kicked in), and `[4, 3, 2, 1]`.

## Strings are collections too

`String` carries the same fluent style: `contains`, `startsWith`, `endsWith`, `split`, `replace`, `repeat`, and `indexOf` with an optional start position:

```rnx
let line = "player:ore:level:7";
let parts = line.split(":");
println!(parts.join(","));
println!(line.contains("ore"));
println!("ab".repeat(3));
```

That prints `player,ore,level,7`, `true`, and `ababab`. String indexes count characters, not bytes. A multi-byte character is one step like any other.

## Maps

`Map<K, V>` is an insertion-ordered hash table. Missing lookups yield `null` (`V?`), not an exception:

```rnx
import { Map } from "@std/collections";

let scores = new Map<String, Int>();
scores.set("ore", 7);
scores.set("al", 12);
println!(scores.get("ore"));
println!(scores.get("nobody") ?? 0);
println!(scores.keys().join(","));
println!(scores.len());
```

That prints `7`, `0`, `ore,al`, and `2`. `keys()` and `values()` return arrays in insertion order; `for k in m` walks the keys, and `m.get(k)` fetches each value. A few rules worth knowing now: keys must be `Int`, `Float`, `Bool`, `String`, or a heap object compared by identity. Two class instances with equal fields are different keys. `Int` and `Float` never mix as keys, and `Array` or `null` keys abort.

## Sets

`Set<T>` keeps distinct members with the same ordering guarantee. Inserting twice keeps one copy:

```rnx
import { Set } from "@std/collections";

let seen = new Set<String>();
seen.add("ore");
seen.add("ore");
println!(seen.has("ore"), seen.len());
for member in seen {
    println!(member);
}
```

That prints `true 1` and then `ore`. Sets iterate members directly with plain `for..in`.

Next: [Error Handling](/guide/07-error-handling) covers `null` and `T?`, `Result`, `throws`, and the `??` / `?.` operators.
