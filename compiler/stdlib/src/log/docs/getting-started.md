# Getting started with @std/log

No install step: the module ships inside the compiler. The stock prelude
re-exports the macros, so this page uses them with no import.

## Copy this

```rnx
fn Main(): Int {
    info!("starting");
    warn!("low disk: {}", 3);
    error!("failed: {}", "timeout");
    return 0;
}
```

All three lines go to stderr. Set `RNX_LOG=warn` to hide the `info!` line,
or `RNX_LOG=off` to hide everything.

## Import explicitly

```rnx
import { debug } from "@std/log";

fn Main(): Int {
    debug!("detail: {}", 42);
    return 0;
}
```

## Keep going

Read the [overview](overview) for the level order and the `RNX_LOG` values,
then open the API reference for exact signatures.
