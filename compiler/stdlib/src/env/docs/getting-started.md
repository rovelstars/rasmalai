---
title: Getting started with Env
description: Read arguments, write a variable, and print the working directory from one small program.
section: Getting started
order: 10
---

# Getting started with Env

The module ships inside the compiler, so there is no install step:

```rnx
import { Env } from "@std/env";
```

## Copy this

Save this as `hello-env.rnx`:

```rnx
import { Env } from "@std/env";

let args = Env.args();
println!("program:", args[0]);
println!("arguments:", args.len() - 1);
Env.set("MODE", "demo");
println!("mode:", Env.get("MODE"));
println!("cwd:", Env.cwd());
```

Run it:

```sh
rnx run hello-env.rnx
```

The output looks like this, with your own path in the first line:

```text
program: hello-env.rnx
arguments: 0
mode: demo
cwd: /home/you/project
```

`Env.args()` returns every command-line argument, with the program path at
index 0. The count after index 0 is the number of user arguments.
`Env.set("MODE", "demo")` writes a variable into this process, and the next
`Env.get` sees it immediately. `Env.cwd()` returns an absolute path.

Pass arguments after `--` and run it again:

```sh
rnx run hello-env.rnx -- alpha beta
```

The second line becomes `arguments: 2`.

## What to expect

The program prints four lines each run. There is no setup file and no
cleanup. A variable written with `Env.set` lives for the rest of the process
and does not change what a child process inherits.
