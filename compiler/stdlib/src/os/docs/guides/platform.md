---
title: Platform and identity
description: Branch on the platform, check the architecture, read the hostname, and build correct line endings.
section: Platform
order: 20
---

# Platform and identity

Programs that run on more than one system need to know where they are.
`OS.platform()` and `OS.arch()` name the target, `OS.hostname()` identifies
the machine, and `OS.eol()` gives the line ending that system expects.

## Branch on the platform

`OS.platform()` returns `"linux"`, `"macos"`, `"windows"`, or the Rust
target-OS name for another target such as `"freebsd"` or `"solaris"`.
Compare against the exact string instead of assuming one of the three
common values.

```rnx
import { OS } from "@std/os";

if (OS.platform() == "windows") {
    println!("windows path separator rules");
} else {
    println!("posix path separator rules");
}
```

This prints the POSIX line on Linux and macOS and the Windows line on
Windows. A target with a different name takes the POSIX branch, so check
for the values you actually support when the difference matters.

## Check the architecture

`OS.arch()` reports the architecture of the running program, not of the
physical machine. An x86_64 build under emulation reports `"x86_64"`.

```rnx
import { OS } from "@std/os";

let is64 = OS.arch() == "x86_64" || OS.arch() == "aarch64";
println!(is64);
```

This is a quick check for the two 64-bit targets the standard library ships
for. For anything more detailed, compare the exact arch name your code
supports.

## Read the hostname

`OS.hostname()` checks the `HOSTNAME` variable, then `/etc/hostname`, then
`COMPUTERNAME` on Windows. A container with none of those set reports
`"unknown"` rather than an empty string.

```rnx
import { OS } from "@std/os";

println!(OS.hostname() != "unknown");
```

Use the value for logs and diagnostics. Do not use it as a stable machine
identity, since it can change when a container moves.

## Build platform line endings

`OS.eol()` returns a string, not a character. Its length is 2 on Windows and
1 everywhere else. Use it for text you write to files or send over a
protocol that expects the host convention.

```rnx
import { OS } from "@std/os";

let header = "name" + OS.eol() + "value" + OS.eol();
println!(header);
```

Hardcoding `"\n"` leaves stray carriage returns when the same file is read
on Windows. `OS.eol()` keeps the convention in one place.

## Related API

`OS.platform`, `OS.arch`, `OS.hostname`, and `OS.eol`. The
[overview](../overview) lists the directory and resource queries.
