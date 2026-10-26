# @std/os

Operating system queries and hardware introspection.

Every call in this module reads a snapshot from the host. Nothing here
changes system state: no environment writes, no file creation, no process
control. Each call costs a syscall or an environment lookup, so read what
you need once and keep the value.

```rnx
import { OS } from "@std/os";

print(OS.platform(), OS.arch());
```

## What is inside

- `OS`

## When to use

Reach for @std/os when a program adapts to where it runs: platform branches, CPU-count-sized pools, temp directories, or uptime telemetry. Pure queries, no handles to close.

It covers: Platform/arch queries; CPU and uptime; Standard directories.
