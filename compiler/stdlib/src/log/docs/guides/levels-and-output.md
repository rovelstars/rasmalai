---
title: Levels and output
description: How RNX_LOG chooses the threshold, how each line is formatted, and what a hidden level skips.
section: Levels
order: 20
---

# Levels and output

A log line is either useful or noise depending on who is reading. During development you want detail; in production you usually want warnings and errors. The threshold is one environment variable, so the same binary can do both without a rebuild.

The levels, from lowest to highest, are `debug`, `info`, `warn`, and `error`. `log!` and `info!` both write at info. `RNX_LOG` selects which macros are visible:

| RNX_LOG | Visible macros |
| --- | --- |
| `off` | none |
| `error` | `error!` |
| `warn` | `warn!`, `error!` |
| `info` | `info!`, `log!`, `warn!`, `error!` |
| `debug` | `debug!`, `info!`, `log!`, `warn!`, `error!` |

An unset variable or an unrecognized value selects `info`, so a typo in the variable name does not silence the program. Use lowercase values.

```rnx
info!("starting");
debug!("cache size: {}", 12);
warn!("retrying: {}", 2);
error!("failed: {}", "timeout");
```

At the default threshold, stderr gets `INFO starting`, `WARN retrying: 2`, and `ERROR failed: timeout`. With `RNX_LOG=debug`, the `DEBUG` line appears between the first two. With `RNX_LOG=warn`, the `INFO` line disappears.

## Formatting

Each line is the level name, one space, and the formatted message. There is no color prefix and no timestamp. Formatting is the `@std/io` renderer: `{}` placeholders, `{{` and `}}` for literal braces, and space-joined values when the first string has no placeholder.

```rnx
warn!("low disk: {}", 3);
info!("literal braces: {{}}");
```

That writes `WARN low disk: 3` and `INFO literal braces: {}`. The same pretty-printer is used for arrays, maps, results, and class instances, with the depth cap and cycle guard described in [Writing output](/docs/@std/io/guides/writing-output).

## Hidden levels skip the work

A level above the threshold returns before the arguments are formatted. A `{}` with no matching argument therefore produces no warning when that level is hidden. When the level is visible, the placeholder stays in the text and a warning goes to stderr, matching `println!`.

One call writes one line, because the writer appends a newline. Keep messages short and put structured values in the arguments rather than building strings by hand.
