---
title: "Console output"
description: Write lines and partial lines to the UEFI console and the serial port.
section: "Console"
order: 20
---

# Console output

There is no stdout in a UEFI application. The firmware owns the display, and
the only way to show text is through its console protocol. `@std/uefi`
wraps that protocol and mirrors every write to the serial port when one is
available, so a qemu capture shows what a person at the screen would see.

## Print a line

`println!` joins its arguments with a single space, renders each value, and
ends the line with a newline.

```rnx
fn Main(): Int {
    println!("ready", 3, true);
    return 0;
}
```

```text
ready 3 true
```

`Int`, `Bool`, `Float`, and `String` render the way they do on the host.
Other values render as a placeholder until the UEFI renderer covers them, so
print scalar fields rather than a whole object.

`uefi_println!` is the same macro under a distinct name. Use it when a file
is shared with code that defines another `println!`.

## Print without a macro

When the text is already a `String`, call `printLine()` or `printText()`.
`printLine()` adds the newline; `printText()` writes exactly the string it is
given, so a following call continues the same line.

```rnx
fn Main(): Int {
    let name = "uefi";
    printText("booting ");
    printLine("hello " + name);
    uefi_println!("bye");
    return 0;
}
```

```text
booting hello uefi
bye
```

The two functions take one `String`. They do not format or join arguments.
Build the string first when a line has several values.

## Where the bytes go

Each write goes through ConOut, which the firmware draws, and then to COM1
when the platform exposes it. The serial copy is why the qemu command in
[Getting started](../getting-started) uses `-serial stdio`: the captured text
is the console text.

The console has no color, cursor, or raw byte API in this package. It has no
input either. A program that needs a keyboard prompt has to talk to the
firmware protocols itself.
