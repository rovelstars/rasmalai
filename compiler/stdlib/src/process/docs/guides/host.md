---
title: Host process
description: Read arguments, manage environment variables, change directory, and exit with a status.
section: Host process
order: 20
---

# Host process

`Process` also exposes the state of the program that is already running.
These calls do not start a child. Use them to read configuration, report
identity, move the working directory, or finish with a chosen status.

## Arguments and environment

`Process.args()` returns the full argument list with the program path at
index 0. `Process.env` reads one variable and reports `null` when the name
is unset, which is the difference from `Env.get` in @std/env.

```rnx
import { Process } from "@std/process";

Process.setEnv("DEMO", "on");
println!(Process.env("DEMO"));
println!(Process.pid() > 0);
println!(Process.cwd().length() > 0);
```

This prints `on`, `true`, and `true`. `setEnv` changes this process
immediately. A child started later does not inherit the value, because
children start from a cleared environment.

`Process.allEnv()` returns a fresh `Map<String, String>` snapshot. Editing
the map does not touch the real environment; use `setEnv` for that. Entries
whose names contain `=` are skipped, since they cannot be split
unambiguously.

```rnx
import { Process } from "@std/process";

Process.setEnv("COUNTED", "1");
println!(Process.allEnv().get("COUNTED"));
```

`removeEnv` unsets a name. Removing a name that was never set does nothing.

## Working directory

`Process.cwd()` returns the absolute working directory. `Process.chdir`
changes it and returns a boolean instead of failing the run:

```rnx
import { Process } from "@std/process";

let before = Process.cwd();
println!(Process.chdir("/tmp"));
println!(Process.chdir(before));
```

Both calls print `true` on a machine where `/tmp` exists and is readable.
A missing or unreadable directory returns false and leaves the process
where it was. Relative paths resolve against the current directory.

## Identity and exit

`Process.pid()` returns the host process id, always positive. It is the id
of the running program, not of any child.

`Process.exit(code)` terminates immediately. Deferred blocks do not run, and
nothing after the call executes. The default code is 0.

```rnx
import { Process } from "@std/process";

println!("closing down");
Process.exit(0);
println!("never printed");
```

This prints `closing down` and leaves the process with status 0.

## Related API

`Process.args`, `Process.env`, `Process.setEnv`, `Process.removeEnv`,
`Process.allEnv`, `Process.cwd`, `Process.chdir`, `Process.pid`, and
`Process.exit`. The [overview](../overview) lists the child-process side.
