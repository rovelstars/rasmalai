---
title: Getting started with @std/io
description: Import io, write a value, and read the current terminal size and color tier.
section: Getting started
order: 10
---

# Getting started with @std/io

The module ships inside the compiler. Import the namespace, and the color enum if you need it:

```rnx
import io, { ColorLevel } from "@std/io";
```

## Copy this

```rnx
import io, { ColorLevel } from "@std/io";

io.write("hello");
io.write([1, 2, 3]);
println!("width", io.width(), "height", io.height());
println!(io.colorProfile() == ColorLevel.Ascii);
```

Run it with `rnx run`. The first two lines are always:

```text
hello
[1, 2, 3]
```

The third line prints the terminal columns and rows, or `80` and `24` when the size cannot be read, which is what happens for a pipe. The last line is `true` when the environment resolves to plain output, for example with `NO_COLOR` set or `TERM=dumb`, and `false` otherwise.

Even though `io.write` is a function and `println!` is a macro, they share the same renderer. `io.write` adds one newline; `println!` joins its arguments with spaces and adds one newline.
