---
title: Environment variables and the working directory
description: Read and write process variables, read the working directory, and exit with a status.
section: Environment
order: 30
---

# Environment variables and the working directory

Configuration often arrives as environment variables. `Env.get` reads one
by name, `Env.set` writes one into the running process, and `Env.cwd`
reports the directory the process is running in.

## Read and write

```rnx
import { Env } from "@std/env";

println!(Env.get("NOT_SET_ANYWHERE") == "");
Env.set("GREETING", "hi");
println!(Env.get("GREETING"));
Env.set("GREETING", "");
println!(Env.get("GREETING") == "");
```

This prints `true`, `hi`, and `true`. The first lookup finds nothing and
returns an empty string. The last write sets the variable to empty, which is
not the same as removing it.

Names match exactly and are case-sensitive. A value the host cannot decode
as UTF-8 also reads as an empty string. There is no `null` result, so compare
against `""` when you only need a usable value.

The value is a copy. Holding a string from `Env.get` does not observe a
later `Env.set`.

## Unset a variable

`Env.set` cannot remove a name. Setting `""` leaves the name in place with
an empty value. Use `Process.removeEnv` from @std/process when the name has
to disappear:

```rnx
import { Process } from "@std/process";

Process.setEnv("DEMO_GONE", "on");
Process.removeEnv("DEMO_GONE");
println!(Process.env("DEMO_GONE") == null);
```

This prints `true`. `Process.env` returns `String?`, so it distinguishes an
unset name from an empty string. Removing a name that was never set does
nothing.

## Child processes do not inherit writes

`Env.set` changes this process only. A child started afterwards with
`Process.spawn` or `Process.run` starts from a cleared environment. The
runtime forwards `PATH` and, on Windows, `SYSTEMROOT`, then adds the entries
in `SpawnOptions.env`. Anything else the child needs has to be listed
explicitly.

## The working directory

`Env.cwd()` returns an absolute path. A successful `Process.chdir` moves it.
If the directory is deleted out from under the process, the read fails and
returns `""`, so test the length before treating the result as a path.

```rnx
import { Env } from "@std/env";

let here = Env.cwd();
if (here == "") {
    println!("working directory unreadable");
} else {
    println!("working directory:", here);
}
```

## Exit with a status

`Env.exit(code)` ends the process. Buffered output is flushed first.
Nothing after the call runs: no `defer` block, no destructor, and no later
statement in the function.

```rnx
import { Env } from "@std/env";

println!("closing down");
Env.exit(0);
println!("never printed");
```

The program prints `closing down` and leaves with status 0. The host keeps
only the low 8 bits of the code, so a shell sees `300` as `44`. Use 0 for
success and small values from 1 to 125 for errors.

The ambient `rnx` object offers the same call as `rnx.exit(code)` and the
same directory read as `rnx.cwd()`.

## Related API

`Env.get`, `Env.set`, `Env.cwd`, and `Env.exit` are all static members of
`Env`. `Process.env`, `Process.setEnv`, and `Process.removeEnv` cover the
same process variables from the other package.
