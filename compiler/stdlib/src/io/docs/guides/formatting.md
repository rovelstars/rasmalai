---
title: Print formatting
description: Fill placeholders, escape braces, and know when arguments are joined with spaces.
section: Formatting
order: 40
---

# Print formatting

The print macros take a list of values. When the first value is a string that contains a placeholder or an escaped brace pair, it is a format string and the remaining arguments fill the placeholders in order. Otherwise every argument is rendered and joined with spaces. That one rule explains most of the surprising output.

```rnx
println!("name", "ore", 7);
println!("name = {}, level = {}", "ore", 7);
println!("{{}} is literal braces");
```

That prints:

```text
name ore 7
name = ore, level = 7
{} is literal braces
```

The first line has no placeholder, so all three values are joined with spaces. The second fills two placeholders. The third uses `{{` and `}}` for literal braces, which is how you print JSON or a code sample.

## Extra and missing arguments

Extra arguments are appended after the filled string, separated by spaces. A missing argument keeps the `{}` text and prints a warning to stderr.

```rnx
println!("{}", 1, 2, 3);
println!("{} {}", 1);
```

The first line prints `1 2 3`. The second prints `1 {}` and warns that the format string has more placeholders than arguments. `{{` and `}}` also count as a signal that the first string is a format string, so `println!("{{literal}}", 2)` prints `{literal} 2`.

## Values inside the line

Each argument goes through the same renderer as `io.write`. Numbers, booleans, arrays, maps, results, and class instances keep their structural form, and strings print without quotes.

```rnx
println!("row", [1, 2, 3]);
```

That prints `row [1, 2, 3]`. A class instance with fields `x` and `y` renders as `Name{x: 1, y: 2}`. The renderer stops at depth 3 with `...` and prints `<cycle>` for a value that contains itself, so logging a large object graph cannot loop forever.

## Streams

`print!` and `println!` write to stdout. `eprint!` and `eprintln!` write to stderr. Use the `e` forms for diagnostics so a caller can redirect normal output without losing the message. All four use the same formatting rules.
