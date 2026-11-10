---
title: Getting started with @std/log
description: Write your first leveled log lines and filter them with RNX_LOG.
section: Getting started
order: 10
---

# Getting started with @std/log

The module ships inside the compiler, and the stock prelude re-exports its macros. This page calls them without an import.

## Copy this

```rnx
info!("starting");
debug!("cache size: {}", 12);
warn!("retrying: {}", 2);
error!("failed: {}", "timeout");
println!("still on stdout");
```

Run it with `rnx run`. With no `RNX_LOG` set, stderr shows:

```text
INFO starting
WARN retrying: 2
ERROR failed: timeout
```

`debug!` is hidden at the default info threshold. `println!` still writes `still on stdout` to stdout.

## Change the filter

Set `RNX_LOG` for the run:

```sh
RNX_LOG=debug rnx run
RNX_LOG=warn rnx run
RNX_LOG=off rnx run
```

The first shows the debug line, the second hides `INFO` and `DEBUG`, and the third prints no log lines at all.

## Import explicitly

If you prefer an explicit dependency, import the macro by name:

```rnx
import { debug } from "@std/log";

debug!("detail: {}", 42);
```

The ambient and imported names are the same macros.
