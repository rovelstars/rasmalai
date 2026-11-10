# @std/log

`@std/log` writes leveled log lines to stderr. Each macro takes the same
arguments as `@std/io` formatting, so `{}` placeholders, literal braces,
and space-joined values behave the same way.

```rnx
info!("starting");
warn!("low disk: {}", 3);
error!("failed: {}", "timeout");
println!("after logging");
```

## Levels

- `debug!` writes below the default threshold.
- `info!` and `log!` write at the default threshold.
- `warn!` and `error!` write at higher severities.
- Each line is `<LEVEL> <message>` with no color prefix.

Set `RNX_LOG` to `error`, `warn`, `info`, `debug`, or `off`. The default is
`info`. A disabled level returns before formatting, so a missing `{}`
argument produces no warning when the level is hidden.

## What is inside

- `log`, `debug`, `info`, `warn`, `error` macros.

## When to use

Use `@std/log` when a program needs severity-filtered diagnostics on stderr
without redirecting stdout. The stock prelude re-exports the macros, so most
programs can call them without an import.
