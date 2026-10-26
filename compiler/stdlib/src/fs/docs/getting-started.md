# Getting started with @std/fs

No install step: the module ships inside the compiler. Import what you need:

```rnx
import fs, { Path } from "@std/fs";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import fs, { Path } from "@std/fs";

fs.writeText("hello.txt", "hello", .Overwrite);
print(fs.readText("hello.txt").unwrapOr("missing"));
print(Path.join("a", "b"));
print(fs.exists("hello.txt"));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
