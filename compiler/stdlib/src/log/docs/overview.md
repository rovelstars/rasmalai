---
title: Leveled logging
description: Severity-filtered log lines on stderr, with a threshold selected by RNX_LOG.
section: Overview
order: 0
---

# Leveled logging

`@std/log` writes diagnostic lines to stderr and filters them by severity. Normal program output stays on stdout, so a caller can pipe or capture it without the log text. Each line has the form `<LEVEL> <message>`, for example `WARN low disk: 3`.

The macros are `debug!`, `log!`, `info!`, `warn!`, and `error!`. `log!` and `info!` share the info threshold. `debug!` is below it, and `warn!` and `error!` are above it. The `RNX_LOG` environment variable selects the threshold:

- `off` hides every level.
- `error` shows only `error!`.
- `warn` shows `warn!` and `error!`.
- `info` shows `info!`, `log!`, `warn!`, and `error!`. This is the default.
- `debug` shows every level.

Any other value, including an unset variable, selects `info`. The values are lowercase.

Formatting follows `@std/io`: `{}` placeholders, `{{` and `}}` for literal braces, and space-joined arguments when there is no placeholder.

```rnx
info!("starting");
warn!("low disk: {}", 3);
error!("failed: {}", "timeout");
println!("program output");
```

With the default threshold, stderr gets `INFO starting`, `WARN low disk: 3`, and `ERROR failed: timeout`, while stdout gets `program output`.

The stock prelude re-exports the macros, so most files can call them with no import. Import from `@std/log` when you want the dependency to be explicit.

## Guide

[Levels and output](guides/levels-and-output) covers thresholds, formatting, and what a disabled level skips.
