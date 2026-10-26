# @std/env

Process arguments, environment variables, and the working directory.

Everything here reads or writes the environment of the running process
itself. Nothing is read from a `.env` file and nothing is cached to disk:
what you see is what the process inherited at start, plus whatever you
write with `Env.set` after that.

```rnx
import { Env } from "@std/env";

Env.set("GREETING", "hi");
print(Env.get("GREETING"));
```

## What is inside

- `Env`

## When to use

Reach for @std/env when a program reads its configuration from the outside: CLI arguments, environment variables, or the current directory. Process.run covers spawning; Env covers reading.

It covers: argv access; Environment get/set; Working directory.
