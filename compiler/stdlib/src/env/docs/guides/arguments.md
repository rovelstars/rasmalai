---
title: Arguments
description: Read the argument list with Env.args, or use the ambient rnx object.
section: Input
order: 20
---

# Arguments

A command-line program often needs to change behavior based on what the
caller passed. `Env.args()` returns the whole argument list as
`Array<String>`.

## Walk the user arguments

Index 0 is the program path, so user arguments start at index 1. This
program prints each one:

```rnx
import { Env } from "@std/env";

let args = Env.args();
let i = 1;
while i < args.len() {
    println!("argument", i, "is", args[i]);
    i = i + 1;
}
```

Run it with two values:

```sh
rnx run args.rnx -- alpha beta
```

The output is:

```text
argument 1 is alpha
argument 2 is beta
```

The array always holds at least one entry. Reading an index past the end is
a runtime error, not `null`, so test `args.len()` before indexing. The array
is a fresh copy each call; mutating it changes nothing about the process.

Under `rnx run`, index 0 is the script path as it was typed on the command
line. The values after it are whatever followed `--`.

## The ambient rnx object

The stock prelude re-exports `RnxHost`, so `rnx` is available with no
import. Its `args` field has a different origin than `Env.args()`: it holds
the CLI arguments after the script path, so `rnx.args[0]` is the first
argument the caller passed.

```rnx
println!("compiler:", rnx.version);
println!("script arguments:", rnx.args.length());
println!(rnx.cwd().length() > 0);
```

Run with `-- alpha beta` and the second line reports 2. `rnx.version` is the
compiler version string, and `rnx.cwd()` reads the working directory.

A local binding named `rnx` shadows the ambient object. A prelude without
`RnxHost` leaves `rnx` unresolved.

## Which one to use

Use `Env.args()` when you want the program path at index 0 and a plain
`Array<String>`. Use `rnx.args` when you want only the caller's arguments
and already rely on the standard prelude.

Both lists are snapshots. Changing the process argument list is not
possible, and neither call reads a file.

## Related API

`Env.args` returns the full list. `RnxHost.args` returns the list without
the program path, and `RnxHost.version` reports the compiler version. The
[overview](../overview) lists the other host calls.
