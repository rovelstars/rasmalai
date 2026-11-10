---
title: Getting started with OS
description: Print the host platform, architecture, CPU count, and temporary directory.
section: Getting started
order: 10
---

# Getting started with OS

The module ships inside the compiler:

```rnx
import { OS } from "@std/os";
```

## Copy this

Save this as `host.rnx`:

```rnx
import { OS } from "@std/os";

println!(OS.platform(), OS.arch());
println!(OS.cpuCount());
println!(OS.tmpdir());
```

Run it:

```sh
rnx run host.rnx
```

On a Linux x86_64 machine the output is three lines:

```text
linux x86_64
16
/tmp
```

The CPU count and temporary directory differ by machine. `OS.cpuCount()`
reports the threads this process can use, so a container with a CPU quota
reports the quota, not the physical core count.

## What to expect

All three calls are read-only. They print values and change nothing. There
is no handle, no close step, and nothing to clean up.

`OS.platform()` and `OS.arch()` are never empty. The other calls have
documented fallbacks: `hostname()` returns `"unknown"`, `homedir()` returns
`""`, `cpuCount()` returns 1, and `uptime()` returns 0.0.
