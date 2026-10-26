# Getting started with @std/process

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { Process } from "@std/process";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { Process } from "@std/process";

let out = Process.run("echo", ["hello"]);
print(out.exitCode);
print(out.stdoutText().contains("hello"));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
