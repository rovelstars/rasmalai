---
title: Getting started with Process
description: Run one command, read its exit code, and print the captured output.
section: Getting started
order: 10
---

# Getting started with Process

The module ships inside the compiler:

```rnx
import { Process } from "@std/process";
```

## Copy this

Save this as `run-echo.rnx`:

```rnx
import { Process } from "@std/process";

let out = Process.run("echo", ["hello"]);
println!(out.exitCode);
println!(out.stdoutText());
println!(out.stderrText().length());
```

Run it:

```sh
rnx run run-echo.rnx
```

The output is:

```text
0
hello

0
```

The first `0` is the child exit code. The next line is the child's stdout,
which already ends in a newline, so `println!` adds a second one. The final
`0` is the length of stderr, because `echo` writes nothing there.

## What happened

`Process.run` starts the command, waits for it, captures both output
streams, and returns a `ProcessOutput`. It closes the child's stdin right
away, so a command that reads until end of input finishes instead of
waiting for a prompt.

The command is resolved through `PATH`. Arguments are passed as an array
and never go through a shell, so pass each argument as its own element. A
shell feature such as a pipe needs an explicit shell (`sh -c` on Unix). A
missing program aborts the process.

## Run it with a failure

Change the command to `sh` with a small script that writes to both streams
and exits with 3:

```rnx
import { Process } from "@std/process";

let out = Process.run("sh", ["-c", "echo hi; echo bad 1>&2; exit 3"]);
println!(out.exitCode);
println!(out.stdoutText());
println!(out.stderrText());
```

The output is `3`, `hi`, and `bad`, each on its own line. The exit code and
the two buffers are independent, so a failing command still gives you
whatever it wrote.
