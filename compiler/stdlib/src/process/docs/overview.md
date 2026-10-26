# @std/process

Host process control and child process lifecycle.

`Process` covers the running program: command-line arguments, the
environment, the working directory, the process id, and exit. Two
ways to start a child:

```rnx
import { Process } from "@std/process";

let out = Process.run("echo", ["hi"]);
print(out.exitCode);
```

## What is inside

- `Stdio`
- `ProcessOutput`
- `SpawnOptions`
- `ChildProcess`
- `Process`

## When to use

Reach for @std/process to control the host (args, env, cwd, exit) or to run children: Process.run captures a command to completion, while spawn plus ByteBuffer pipes drives interactive children. Always wait() what you spawn.

It covers: Host control; spawn/run lifecycles; ByteBuffer pipes.
