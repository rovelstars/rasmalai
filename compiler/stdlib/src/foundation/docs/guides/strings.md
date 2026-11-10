---
title: "Strings"
description: "Measure, slice, search, split, and transform a String."
section: Strings
order: 40
---

# Strings

`String` is the canonical UTF-8 text type. Every index counts characters, not bytes, so a multi-byte character is one step like any other. `length()`, `len()`, `slice`, `indexOf`, `trim`, `concat`, and `charCodeAt` are compiler builtins. The search and transform helpers are extension methods from `@std/foundation`.

## Measure and slice

```rnx
let s = "rasmalai";
println!(s.length());
println!(s.slice(0, 3));
println!(s.indexOf("mal"));
println!("  pad  ".trim());
```

That prints `8`, `ras`, `3`, and `pad`. `length` and `len` also read as properties. `slice(start, end)` clamps both bounds to the length and returns `""` once the start reaches the end. `indexOf` has a second form that takes a start position:

```rnx
let path = "src/lib/main.rnx";
println!(path.indexOf("/"));
println!(path.indexOf("/", 4));
```

The first call prints `3`. The second starts at index 4 and finds the next slash, so it prints `7`. Both forms return `-1` when there is no match. An empty needle answers with the start it was given.

`charCodeAt(index)` returns the Unicode code point of the character at that index, or `-1` past the end. A `Char` is treated as a `String`, so `charCodeAt` is how you read the scalar value.

## Split and transform

```rnx
let path = "rasmalai/compiler";
println!(path.contains("compiler"));
println!(path.startsWith("ras"));
println!(path.endsWith("ler"));
println!(path.split("/").length());
println!(path.replace("/", "-"));
println!("a-b-c".replaceAll("-", "+"));
println!("ab".repeat(3));
println!("hello".toUpperCase());
```

The lines print `true`, `true`, `true`, `2`, `rasmalai-compiler`, `a+b+c`, `ababab`, and `HELLO`. `replace` swaps only the first occurrence, while `replaceAll` swaps every one. `split` cuts around every delimiter, and `split("")` gives one element per character. `repeat(0)` or a negative count gives `""`.

`toUpperCase` and `toLowerCase` change only ASCII `a` to `z` and `A` to `Z`. Accented letters pass through untouched, so `"héllo".toUpperCase()` gives `"HéLLO"`.

## A real case

```rnx
fn normalizePath(path: String): String {
    let trimmed = path.trim();
    let slug = trimmed.replaceAll("/", "-");
    return slug.toLowerCase();
}

println!(normalizePath("  Rasmalai/Compiler  "));
```

It prints `rasmalai-compiler`. The chain trims the padding, swaps every separator, and folds the ASCII letters. Each helper returns a new string, so the original value is never changed.
