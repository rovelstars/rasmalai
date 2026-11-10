---
title: "Console streams"
description: Write to standard output and standard error from a WASI module.
section: "Console"
order: 20
---

# Console streams

A WASI command module has no terminal of its own. It writes bytes to file
descriptors through `fd_write`, and the host decides where those bytes go.
`@std/wasi` gives you descriptor 1 for standard output and descriptor 2 for
standard error, with macros for formatted lines and functions for strings.

## Write a line to each stream

`println!` and `eprintln!` join their arguments with a single space and end
the line with `\n`.

```rnx
fn Main(): Int {
    println!("out", 3, true);
    eprintln!("err", 2);
    return 0;
}
```

Standard output receives:

```text
out 3 true
```

Standard error receives:

```text
err 2
```

Keeping program data on stdout and diagnostics on stderr lets a caller pipe
one stream without the other. The host captures the two independently.

`wasi_println!` is the same macro as `println!` under a distinct name. Use it
when a file is shared with code that defines another `println!`.

## Write a string you already have

The call macros build an argument array. When the text is already a `String`,
the four functions write it directly. `printLine()` and `printErrorLine()`
add a newline; `printText()` and `printErrorText()` do not.

```rnx
fn Main(): Int {
    let name = "wasi";
    printText("hello ");
    printLine(name);
    printErrorLine("warning: " + name);
    return 3;
}
```

Standard output receives `hello wasi` on one line. Standard error receives
`warning: wasi`. The process exits with status 3 because `Main` returned 3.

## Formatting and limits

`Int`, `Bool`, `Float`, and `String` render their value, and `null` renders
as `null`. Container rendering falls back to a placeholder, so `Array`,
`Map`, and class instances are not pretty-printed yet. Print scalar fields
instead.

`@std/io`, `@std/log`, `@std/fs`, `@std/process`, `@std/net`, and
`@std/sync` are not part of the target prelude. The package has no terminal
size query, color control, raw mode, or input functions. A program that
needs those surfaces cannot build for this target yet.
