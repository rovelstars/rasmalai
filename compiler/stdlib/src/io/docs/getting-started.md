# Getting started with @std/io

No install step: the module ships inside the compiler. Import what you need:

```rnx
import io, { ColorLevel } from "@std/io";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import io, { ColorLevel } from "@std/io";

io.write("hello");
print(io.colorProfile() == ColorLevel.Ascii);
print(io.width() > 0);
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
