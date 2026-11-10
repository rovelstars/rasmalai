---
title: OS
description: Read the platform, architecture, CPU count, directories, and uptime of the host.
section: Overview
order: 0
---

# OS

@std/os answers questions about the machine that runs the program. Every
call reads a snapshot from the host. Nothing here changes system state: no
environment writes, no file creation, and no process control.

All members are static. Call them as `OS.platform()` without constructing
anything. Values come from the running process, and the directory queries
follow its environment. A `TMPDIR` override changes what `tmpdir()` reports,
for example.

The surface is one class:

- `OS.platform()`: `"linux"`, `"macos"`, `"windows"`, or another Rust
  target-OS name.
- `OS.arch()`: `"x86_64"`, `"aarch64"`, or another Rust target-arch name.
- `OS.hostname()`: the system hostname, or `"unknown"`.
- `OS.tmpdir()`: the temporary directory.
- `OS.homedir()`: the current user's home directory, or `""`.
- `OS.cpuCount()`: CPU threads available to the process, at least 1.
- `OS.uptime()`: seconds since boot as a `Float`, or `0.0`.
- `OS.eol()`: the platform end-of-line marker as a `String`.

A first look at the host:

```rnx
import { OS } from "@std/os";

println!(OS.platform(), OS.arch());
println!(OS.cpuCount());
println!(OS.tmpdir());
```

On a Linux x86_64 machine this prints `linux x86_64`, the scheduler-visible
CPU count, and the temporary directory, usually `/tmp`.

## Fallbacks are values, not errors

A query that cannot be answered returns a documented fallback instead of
throwing. `hostname()` returns `"unknown"`, `homedir()` returns `""`,
`cpuCount()` returns 1, and `uptime()` returns 0.0. Check the value when the
difference matters.

Each call costs a syscall or an environment lookup. Read what you need once
and keep the value instead of querying inside a hot loop.

## Guides

- [Platform and identity](guides/platform) covers platform, architecture,
  hostname, and the end-of-line marker.
- [Host resources](guides/resources) covers directories, CPU count, and
  uptime.

Start with [Getting started](getting-started) for a program to run.
