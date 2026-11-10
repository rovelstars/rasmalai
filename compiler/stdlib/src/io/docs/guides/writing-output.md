---
title: Writing output
description: Print values to stdout or stderr, write exact bytes, and control color.
section: Output
order: 20
---

# Writing output

A program has three output paths, and they are not interchangeable. Normal results go to stdout, diagnostics go to stderr, and terminal control sequences need exact bytes with no newline added. `@std/io` gives each one a separate call.

```rnx
import io from "@std/io";
import { ByteBuffer } from "@std/bytes";

io.write("one line");
io.write([1, 2, 3]);
io.writeError("to stderr");
io.writeRaw(ByteBuffer.fromString("AB"));
```

That prints `one line` and `[1, 2, 3]` on stdout, `to stderr` on stderr, and `AB` with no trailing newline. Redirect stdout and stderr separately to see the split.

`io.write(value)` pretty-prints one value and appends a newline. `io.writeError(value)` does the same on descriptor 2. `io.writeRaw(bytes)` writes the buffer unchanged: no newline, no formatting, no color.

> [!NOTE]
> Native backends write exactly the bytes you pass to `writeRaw`. The interpreter console is line-oriented and terminates each flush with a newline, so a progress bar that uses carriage returns needs a native build.

## The renderer

Arrays render as `[a, b]`, maps as `{k: v}` in insertion order, class instances as `Name{field: value}` in declaration order, and results as `Ok(v)` or `Err(e)`. Strings print without quotes. A value nested deeper than 3 levels renders its inner part as `...`, and a cycle renders as `<cycle>`.

```rnx
import io from "@std/io";

class Point {
    let x: Int;
    let y: Int;
    init(x: Int, y: Int) {
        this.x = x;
        this.y = y;
    }
}

io.write(new Point(1, 2));
io.write([new Point(1, 2), new Point(3, 4)]);
```

That prints `Point{x: 1, y: 2}` and `[Point{x: 1, y: 2}, Point{x: 3, y: 4}]`. Only fields render; methods are not listed.

## Colors

`io.colorProfile()` returns a `ColorLevel`: `TrueColor`, `Ansi256`, `Ansi16`, or `Ascii`. The check order is: a non-empty `NO_COLOR` forces `Ascii`; `COLORTERM` of `truecolor` or `24bit` gives `TrueColor` on a terminal and `Ansi16` otherwise; `TERM` containing `256color` gives `Ansi256` on a terminal and `Ansi16` otherwise; `TERM` empty or `dumb` gives `Ascii`; any other terminal gives `Ansi16`; everything else gives `Ascii`.

A non-terminal stdout caps the result at `Ansi16`. `io.write` and the print macros emit no escape sequences for `Ascii`. Set `NO_COLOR` to any non-empty value to force plain output; an empty value reads the same as unset because `Env.get` cannot tell the two apart.

```rnx
import io, { ColorLevel } from "@std/io";

let level = io.colorProfile();
println!(level == ColorLevel.Ascii || level == ColorLevel.Ansi16);
```

## Clearing

`io.clear()` clears the screen and homes the cursor. It is a no-op when stdout is not a terminal, so it is safe to call in a script that may be piped.
