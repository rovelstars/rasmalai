---
title: Env
description: Read command-line arguments, environment variables, and the working directory of the running process.
section: Overview
order: 0
---

# Env

@std/env reads and writes the environment of the process itself. It covers
the argument list the program was started with, the environment variables it
inherited or set, the directory it is running in, and the call that ends it.

Nothing here reads a `.env` file and nothing is cached to disk. What you see
is what the process inherited at start, plus whatever you write with
`Env.set` after that.

The package has two faces:

- `Env` is a class with static members. Call `Env.get`, `Env.set`,
  `Env.args`, `Env.cwd`, and `Env.exit` directly.
- `RnxHost` backs the ambient `rnx` object. `rnx.version` is the compiler
  version, `rnx.args` holds CLI arguments after the script path, `rnx.cwd()`
  reads the working directory, and `rnx.exit(code)` terminates. The stock
  prelude re-exports `RnxHost`, so no import is needed for `rnx`.

A first program:

```rnx
import { Env } from "@std/env";

Env.set("GREETING", "hi");
println!(Env.get("GREETING"));
println!(Env.args().len() > 0);
println!(Env.cwd().len() > 0);
```

This prints `hi`, `true`, and `true`.

## Values are strings

Every lookup returns a `String`. A name that is not set reads as an empty
string, and so does a value the host cannot decode as UTF-8. Nothing here
returns `null`.

Because of that, `Env.get` cannot tell an unset variable from one set to the
empty string. Compare against `""` when you only need to know whether a
usable value is present. `Process.env` from @std/process returns `String?`
instead, so it can report the difference.

`Env.set` writes a value into this process. Setting an empty string sets the
name to empty; it does not remove it. `Process.removeEnv` is the call that
unsets a name.

Values do not reach child processes. A child started with `Process.spawn` or
`Process.run` starts from a cleared environment, so pass what the child
needs through `SpawnOptions.env`.

## Guides

- [Arguments](guides/arguments) covers `Env.args` and the ambient `rnx`
  object.
- [Environment variables and the working directory](guides/environment)
  covers `Env.get`, `Env.set`, `Env.cwd`, and `Env.exit`.

Start with [Getting started](getting-started) for a copyable program.
