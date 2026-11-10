---
title: Spawning a child
description: Keep a child alive with Process.spawn, read its pipes, poll it, and stop it.
section: Spawning
order: 40
---

# Spawning a child

`Process.run` is a one-shot call. When the parent has to write input, read
output while the child is still running, poll for completion, or stop the
child early, use `Process.spawn` instead.

`spawn` returns a `ChildProcess` immediately. The handle is a slot in one
host-wide table, not the OS process id. Use the `pid` field when something
outside the program needs the id.

## Read a short command

```rnx
import { Process } from "@std/process";

let child = Process.spawn("echo", ["streamed"]);
println!(child.readStdoutText());
println!(child.wait());
```

This prints `streamed` and `0`. `readStdoutText` drains the pipe to end of
input, and `wait` blocks until the child exits and reaps it.

With no options, all three streams are piped. That makes `wait` alone unsafe
for a chatty child: the child can fill the stdout pipe and block while the
parent waits for it to exit. Drain the pipes first, or use
`Process.run`, which drains both streams on your behalf.

## Write to stdin

A child that filters its input needs a write, then an end-of-input marker:

```rnx
import { Process } from "@std/process";

let child = Process.spawn("cat", []);
child.writeStdinText("ping\n");
child.closeStdin();
println!(child.readStdoutText());
println!(child.wait());
```

This prints `ping` and `0`. `closeStdin` sends EOF. Call it before `wait`,
or a program such as `cat` waits for input that never arrives.

`writeStdinText` returns the number of bytes written, and `writeStdin` takes
a `ByteBuffer`. A short write is normal on a pipe with a small buffer; loop
until the total matches the bytes you meant to send. Both calls report -1
when stdin is not piped or the pipe is already closed.

## Poll instead of block

`tryWait` checks whether the child has finished. It returns the exit code
after the child is reaped and `null` while it is still running.

```rnx
import { Process } from "@std/process";

let child = Process.spawn("sh", ["-c", "echo done"]);
println!(child.readStdoutText());
while child.tryWait() == null {
}
println!(child.wait());
```

This prints `done` and `0`. Read the output first. Then poll until the child
is reaped. The code stays available, so `wait` after a successful `tryWait`
returns the same exit code.

## Stop a child

`kill` sends signal 15 (`SIGTERM`) by default. Signal 9 (`SIGKILL`) does not
let the child run cleanup handlers. On Windows the signal number is ignored
and the child is force killed.

```rnx
import { Process } from "@std/process";

let child = Process.spawn("sh", ["-c", "sleep 5"]);
println!(child.kill());
println!(child.wait());
```

This prints `true` and `143`, because the signal ended the child. `kill`
returns false when the child has already been reaped or the handle is gone.
You still owe the child a `wait` after killing it.

## Dropping a handle

Letting a `ChildProcess` drop detaches it. The parent stops holding the
pipes, so the child sees EOF on stdin and may fail on a full pipe. On Unix
nobody reaps the child afterwards, so it stays a zombie until the host
exits. Detach only for children you deliberately do not wait on.

## Related API

`Process.spawn` returns `ChildProcess`. Its fields are `handle` and `pid`.
Its methods are `writeStdin`, `writeStdinText`, `closeStdin`, `readStdout`,
`readStderr`, `readStdoutText`, `readStderrText`, `tryWait`, `wait`, and
`kill`.
