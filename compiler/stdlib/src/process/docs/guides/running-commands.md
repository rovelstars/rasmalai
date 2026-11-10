---
title: Running commands
description: Capture exit code and output with Process.run, and wire streams or environment per call.
section: Running commands
order: 30
---

# Running commands

Most commands are short: start them, wait, and read what they wrote.
`Process.run` does that in one call. It closes the child's stdin, drains
stdout and stderr, waits for the exit, and returns a `ProcessOutput`.

## Read the result

```rnx
import { Process } from "@std/process";

let out = Process.run("sh", ["-c", "echo hi; echo bad 1>&2; exit 3"]);
println!(out.exitCode);
println!(out.stdoutText());
println!(out.stderrText());
```

This prints `3`, `hi`, and `bad`. The exit code is `0` to `255` normally.
On Unix a signal gives `128 + signal`, so a child killed by `SIGTERM`
reports 143.

`stdout` and `stderr` are `ByteBuffer` fields holding the exact bytes.
`stdoutText()` and `stderrText()` decode them as UTF-8. Only piped streams
are captured; an inherited or null stream contributes an empty buffer.

## Choose the wiring

`SpawnOptions` sets the working directory, the environment table, and each
of the three streams. All three default to `Stdio.Piped`. Pass `Stdio.Null`
to throw one away, or `Stdio.Inherit` to share the parent's descriptor.

```rnx
import { Process, SpawnOptions, Stdio } from "@std/process";

let opts = new SpawnOptions(null, null, Stdio.Piped, Stdio.Null, Stdio.Piped);
let out = Process.run("sh", ["-c", "echo hidden; echo shown 1>&2"], opts);
println!(out.stdoutText().length());
println!(out.stderrText());
```

This prints `0` and `shown`. Stdout was attached to the null device, so its
captured buffer is empty. Stderr was piped and captured as usual.

Arguments after `cwd` need their leading arguments too. To change only
stderr, spell out the defaults before it:

```rnx
import { Process, SpawnOptions, Stdio } from "@std/process";

let opts = new SpawnOptions(null, null, Stdio.Piped, Stdio.Piped, Stdio.Null);
let out = Process.run("sh", ["-c", "echo hidden; echo shown 1>&2"], opts);
println!(out.stdoutText());
println!(out.stderrText().length());
```

This prints `hidden` and `0`. Stdout was piped and captured, while stderr
went to the null device.

## Pass environment entries

A child starts from a cleared environment with `PATH` forwarded. Put
everything else in a `Map<String, String>`:

```rnx
import { Process, SpawnOptions, Stdio } from "@std/process";
import { Map } from "@std/collections";

let env = new Map<String, String>();
env.set("GREETING", "hi");
let opts = new SpawnOptions(null, env, Stdio.Piped, Stdio.Piped, Stdio.Piped);
let out = Process.run("sh", ["-c", "echo $GREETING"], opts);
println!(out.stdoutText());
```

This prints `hi`. A key mapped to null becomes `KEY=` with an empty value.

The first `SpawnOptions` argument is the child working directory. `null`
keeps the parent's directory. Passing `/tmp` runs the child there:

```rnx
import { Process, SpawnOptions } from "@std/process";

let opts = new SpawnOptions("/tmp");
let out = Process.run("pwd", [], opts);
println!(out.stdoutText());
```

The child prints `/tmp` on a system where that directory exists.

## Limits

Output is buffered whole, so a child that never stops writing grows the
host's memory. For long output, use `spawn` from [Spawning a child](spawning)
and read in chunks.

The command is resolved through `PATH` and the argument array never goes
through a shell. A missing program aborts the process. In a locked project, a
command outside the `sys:exec` grant stops with
`[S401]: Unapproved Child Process`.

## Related API

`Process.run` returns `ProcessOutput`, whose fields are `exitCode`,
`stdout`, and `stderr`. `SpawnOptions` takes `cwd`, `env`, `stdin`,
`stdout`, and `stderr`. `Stdio` has the three modes `Inherit`, `Piped`, and
`Null`.
