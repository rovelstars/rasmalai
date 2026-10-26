# Getting started with @std/os

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { OS } from "@std/os";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { OS } from "@std/os";

print(OS.platform(), OS.arch());
print(OS.cpuCount());
print(OS.tmpdir());
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
