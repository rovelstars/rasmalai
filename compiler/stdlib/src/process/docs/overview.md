---
title: Process
description: Control the host process and run child processes, with captured output or live pipes.
section: Overview
order: 0
---

# Process

@std/process covers two jobs that belong together: reading and changing the
state of the running program, and starting child processes. Host state
includes the argument list, environment, working directory, process id, and
exit. Children come in two shapes.

`Process.run` waits for the child, captures stdout and stderr, and returns
the exit code with both buffers. `Process.spawn` returns immediately with a
`ChildProcess` handle, and the caller owns waiting, reading, and killing.

The package has five public types:

- `Stdio`: per-stream wiring, one of `Stdio.Inherit`, `Stdio.Piped`, or
  `Stdio.Null`.
- `ProcessOutput`: the exit code plus buffered stdout and stderr from
  `Process.run`.
- `SpawnOptions`: child working directory, environment table, and the three
  stream modes.
- `ChildProcess`: a live child with pipe reads, pipe writes, `wait`, and
  `kill`.
- `Process`: static host calls plus `run` and `spawn`.

A first command:

```rnx
import { Process } from "@std/process";

let out = Process.run("echo", ["hello"]);
println!(out.exitCode);
println!(out.stdoutText());
```

This prints `0` and then `hello`. `stdoutText()` decodes the captured bytes
as UTF-8 and keeps the newline the command wrote.

## Children start clean

A child does not inherit the parent environment. Only `PATH` and, on
Windows, `SYSTEMROOT` are forwarded. Everything else goes in the
`SpawnOptions.env` table. The same options set the child working directory
and wire each of stdin, stdout, and stderr separately.

With no options, all three streams are piped. That is convenient for
`Process.run` and dangerous for `spawn`, because a child that fills a pipe
blocks until the parent drains it. Read the output before `wait`, or let
`Process.run` interleave the two streams.

## Spawning needs approval

In a locked project, spawning is subject to the `sys:exec` grant. Without
it the runtime stops the program with `[S401]: Unapproved Child Process`. A
command that cannot be started aborts the process. Add a
`sys:exec:<binary>` grant to the project when the program needs to run that
binary.

## Guides

- [Host process](guides/host) covers arguments, environment, directory,
  process id, and exit.
- [Running commands](guides/running-commands) covers `Process.run` and
  captured output.
- [Spawning a child](guides/spawning) covers `spawn`, `wait`, polling, and
  signals.
- [Child pipes](guides/child-io) covers `ByteBuffer` reads and writes.

Start with [Getting started](getting-started) for a program to run.
