---
title: Host resources
description: Read the temporary and home directories, size a worker pool from the CPU count, and check uptime.
section: Host resources
order: 30
---

# Host resources

A program often needs a place to put scratch files, a home directory for
per-user data, or a worker count that matches the machine. `OS` answers all
three without opening a file or creating a directory.

## Temporary and home directories

`OS.tmpdir()` checks `TMPDIR`, then `TEMP`, then `TMP`, then the OS default.
The first non-empty variable wins, so a shell override changes what the
program sees. The directory is not created; it is whatever the host already
reports.

`OS.homedir()` reads `HOME` or `USERPROFILE`. It returns `""` when neither
is set, which happens in stripped containers and daemons started without a
login environment. Treat the empty string as unknown, not as a usable path.

```rnx
import { OS } from "@std/os";

let home = OS.homedir();
if (home == "") {
    println!("no home directory in this environment");
} else {
    println!("home:", home);
}
```

Print the temporary directory the same way with `OS.tmpdir()`. Both values
are snapshots, so read them once and keep the string.

## Size a worker pool

`OS.cpuCount()` reports the logical CPU threads available to this process.
CPU affinity masks and container CPU quotas shrink it, so a machine with 16
hardware threads limited to 4 cores reports 4. It returns 1 when the host
cannot answer.

```rnx
import { OS } from "@std/os";

let workers = OS.cpuCount();
println!("worker slots:", workers);
```

Use the value to size a pool. Since it is never below 1, a loop that starts
`workers` tasks always starts at least one. Re-read it if the process can
move between cgroups, otherwise keep the value.

## Check uptime

`OS.uptime()` returns seconds since boot as a `Float`, not seconds since the
program started and not per-core time. It currently reads `/proc/uptime`, so
it returns 0.0 on macOS and Windows.

```rnx
import { OS } from "@std/os";

let seconds = OS.uptime();
if (seconds > 0.0) {
    println!("uptime seconds:", seconds);
} else {
    println!("uptime unavailable");
}
```

Test against 0.0 as unknown. A host that has just booted can report a small
positive value, so do not treat a low number as an error.

## Related API

`OS.tmpdir`, `OS.homedir`, `OS.cpuCount`, and `OS.uptime`. The
[overview](../overview) lists the platform and identity queries.
