---
title: "Iteration"
description: "Drive for..in with Iterator, Iterable, and ArrayIter."
section: Iteration
order: 60
---

# Iteration

A `for..in` loop needs a source of items. `Iterator<T>` models a pull source with one method, `next()`, which returns the next item or `null` at the end. `Iterable<T>` models a value that can open a fresh iterator through `iterator()`. `ArrayIter<T>` is the ready-made iterator over an array.

## Pull one item at a time

```rnx
let it = new ArrayIter<Int>([10, 20]);
println!(it.next() ?? 0);
println!(it.next() ?? 0);
println!(it.next() == null);
```

That prints `10`, `20`, and `true`. `ArrayIter` holds the array and a cursor. Each `next()` reads the element at the cursor and steps it on. Once the cursor passes the end, every call returns `null`.

## Make a type iterable

```rnx
class Countdown with Iterable<Int> {
    let start: Int;

    init(start: Int) {
        this.start = start;
    }

    fn iterator(): Iterator<Int> {
        return new ArrayIter<Int>([this.start, this.start - 1, 0]);
    }
}

for n in new Countdown(3) {
    println!(n as Int);
}
```

The loop prints `3`, `2`, and `0`. Calling `iterator()` returns a new `ArrayIter` each time, so two loops over the same value do not share a cursor. A value with an `iterator()` method or a `next()` method is enough to drive `for..in`. A bare `Iterator` works when its static type keeps the method.

## A real case

```rnx
class Chunks with Iterable<String> {
    let text: String;
    let size: Int;

    init(text: String, size: Int) {
        this.text = text;
        this.size = size;
    }

    fn iterator(): Iterator<String> {
        let parts: Array<String> = [];
        let i = 0;
        while i < this.text.length() {
            parts.push(this.text.slice(i, i + this.size));
            i = i + this.size;
        }
        return new ArrayIter<String>(parts);
    }
}

for part in new Chunks("abcdefg", 3) {
    println!(part);
}
```

The loop prints `abc`, `def`, and `g`. The class builds the pieces up front and hands back a fresh `ArrayIter` each time `iterator()` is called.
