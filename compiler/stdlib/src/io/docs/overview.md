---
title: Terminal input and output
description: Standard streams, pretty-printed output, line input, terminal size, colors, and raw mode.
section: Overview
order: 0
---

# Terminal input and output

`@std/io` owns the terminal. It writes values to stdout and stderr, reads a line or the rest of stdin, reports the terminal size, picks a color level, and toggles raw mode. File and directory work stays in `@std/fs`.

Import the namespace for the free functions, and the enum when you compare color levels:

```rnx
import io, { ColorLevel } from "@std/io";
```

The print macros `print!`, `println!`, `eprint!`, and `eprintln!` are declared in this package and re-exported by the stock prelude, so most programs call them without an import. There is no `print` function and no `Console` class. The function form is `io.write(value)`, which prints one value with a trailing newline. `ColorLevel` has four values: `TrueColor`, `Ansi256`, `Ansi16`, and `Ascii`.

`io.write` renders containers with the shared pretty-printer: arrays as `[a, b]`, maps as `{k: v}`, class instances as `Name{field: value}`, and results as `Ok(v)` or `Err(e)`. Nesting deeper than 3 becomes `...`, and a value that contains itself becomes `<cycle>`. Strings print as-is, without quotes. Colors appear only when stdout is a terminal and the color profile allows them.

```rnx
import io, { ColorLevel } from "@std/io";

io.write("hello");
println!("width", io.width(), "height", io.height());
println!(io.colorProfile() == ColorLevel.Ascii);
```

The first line is fixed. The other two report the terminal and the environment, so their values differ between a terminal, a pipe, and the playground.

## Guides

[Getting started](getting-started) has a first program. [Writing output](guides/writing-output) covers the value renderer, raw bytes, stderr, and colors. [Reading input](guides/reading-input) covers lines, whole input, and raw mode. [Print formatting](guides/formatting) covers placeholders, braces, and argument joining.
