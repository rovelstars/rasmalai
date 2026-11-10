---
title: Reading input
description: Read a line with a prompt, read all of stdin, and switch the terminal to raw mode.
section: Input
order: 30
---

# Reading input

Input is more awkward than output because the amount is not known in advance. A prompt is a line, a configuration blob is everything until end of input, and an interactive key press is one byte with no echo. `@std/io` has a call for each shape.

```rnx
import io from "@std/io";

let name = io.readLine("name: ");
println!(name.unwrapOr("(no input)"));
```

`readLine(prompt)` writes the prompt to stdout with no newline, reads until a newline or end of input, and strips the trailing newline. A carriage return before it is stripped too, so CRLF input reads cleanly. The prompt defaults to `""`, so `io.readLine()` reads without printing anything. The call reads one byte at a time, which is why a second `readLine` does not lose bytes that arrived after the first newline.

The result is `Result<String, String>`. `Err` means stdin is not available, a read failed, or no bytes were read before end of input.

## Reading everything

`io.read()` reads from stdin to end of input and returns `Ok(text)` or `Err(message)`.

```rnx
import io from "@std/io";

let all = io.read();
println!(all.unwrapOr("").length());
```

That prints the character count of the input. When stdin is a pipe, this waits for the writer to close it.

## Terminal checks

`io.isTTY()` reports whether stdin is a terminal. `io.width()` and `io.height()` report the console size and fall back to `80` and `24` when the size cannot be read. A pipe, a redirected file, and most CI environments are not terminals, so guard interactive behavior with `isTTY()`.

## Raw mode

Raw mode turns off line buffering and echo so a program can react to single key presses. `setRawMode(true)` returns `Ok(true)` on success and `Err(message)` when stdin is not a terminal or the OS call fails. Always turn it off again; the runtime also restores the saved mode when the process exits.

```rnx
import io from "@std/io";
import { ByteBuffer } from "@std/bytes";

let raw = io.setRawMode(true);
if raw.isOk() {
    let tty = io.stdin();
    let buf = ByteBuffer.allocate(1);
    tty.readBytes(buf, 0, 1);
    tty.close();
    println!(buf.readUInt8(0));
}
io.setRawMode(false);
```

Each `io.stdin()`, `io.stdout()`, or `io.stderr()` call duplicates the descriptor, so closing the handle does not close the process stream. That is why the example can close `tty` and still print afterward.
