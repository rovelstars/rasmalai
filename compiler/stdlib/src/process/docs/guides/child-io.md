---
title: Child pipes
description: Read and write exact bytes with ByteBuffer, and avoid blocking on a full pipe.
section: Child I/O
order: 50
---

# Child pipes

Some commands speak in bytes, not whole lines. A child may need a binary
request on stdin, or it may produce a frame whose length matters. The pipe
methods take and fill `ByteBuffer` values from @std/bytes, so you control
the offset and the length.

## Read into a buffer

```rnx
import { Process } from "@std/process";
import { ByteBuffer } from "@std/bytes";

let child = Process.spawn("echo", ["abc"]);
let buf = ByteBuffer.allocate(3);
println!(child.readStdout(buf));
println!(buf.readString(0, 3));
println!(child.wait());
```

This prints `3`, `abc`, and `0`. `readStdout` blocks until the child writes
or closes the stream, then returns the number of bytes placed in the buffer.
A return of 0 means end of input. A return of -1 means stdout was not piped
or the pipe was already consumed by `wait`.

The buffer is written from `offset`, and `len` limits how much of it to use.
Pass -1 to fill the rest of the buffer.

## Read stderr the same way

```rnx
import { Process } from "@std/process";
import { ByteBuffer } from "@std/bytes";

let child = Process.spawn("sh", ["-c", "echo bad 1>&2"]);
let buf = ByteBuffer.allocate(16);
println!(child.readStderr(buf));
println!(child.wait());
```

This prints `4` and `0`, the byte count for `bad` plus its newline, then the
exit code. `readStderr` is the stderr twin of `readStdout`.

Reading both streams needs two buffers and careful ordering. A child that
fills one pipe blocks until that pipe is drained, so a parent that waits on
the other stream can deadlock. `Process.run` drains stdout on a helper
thread while it drains stderr inline, which keeps both moving.

## Write exact bytes

```rnx
import { Process } from "@std/process";

let child = Process.spawn("cat", []);
println!(child.writeStdinText("hi"));
child.closeStdin();
println!(child.readStdoutText());
println!(child.wait());
```

This prints `2`, `hi`, and `0`. `writeStdinText` encodes the string as UTF-8
and writes it in one call. `writeStdin` takes a `ByteBuffer` and the same
optional offset and length.

A short write is normal on a pipe with a small buffer. The return value is
the number of bytes accepted, and -1 means stdin is closed or was not piped.
Loop until the total matches the bytes you meant to send, then close the
pipe. A child that reads to end of input sees EOF after `closeStdin`.

## Whole streams and chunks

`readStdoutText` and `readStderrText` loop in 4096-byte chunks until the
pipe closes and decode the result as UTF-8. They block for as long as the
child keeps the stream open.

Use the buffer methods when the data is binary, when you need to cap how
much you read, or when you want to process a chunk before reading the next
one. Use the text methods when the child writes text and you want it all.

## Related API

`ChildProcess.writeStdin`, `writeStdinText`, `readStdout`, `readStderr`,
`readStdoutText`, `readStderrText`, and `closeStdin`. `ByteBuffer` provides
`allocate`, `readString`, and the numeric lane readers.
