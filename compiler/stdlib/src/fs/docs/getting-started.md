---
title: Getting started with @std/fs
description: Create a directory, write and read a file, and check its size.
section: Getting started
order: 10
---

# Getting started with @std/fs

The module ships inside the compiler. Import the namespace for the free functions and the classes you need:

```rnx
import fs, { Path } from "@std/fs";
```

## Copy this

```rnx
import fs, { Path } from "@std/fs";

let path = Path.join("demo", "hello.txt");
fs.mkdirAll("demo");
fs.writeText(path, "hello", .Overwrite);
println!(fs.readText(path).unwrapOr("missing"));
println!(fs.stat(path).unwrap().size);
fs.removeAll("demo");
```

Run it with `rnx run`. The output is:

```text
hello
5
```

`mkdirAll` creates `demo` and any missing parent. `writeText` needs a write mode; `.Overwrite` truncates an existing file and creates a missing one. `readText` returns a `Result`, and `unwrapOr` supplies a fallback. `stat` returns a `FileStat`, whose `size` is the byte count.

## Cleanup

The last line removes the directory tree. The call returns a `Result`; this example ignores it because the path is known and the program is about to exit. In longer programs, check `isOk()` or keep the value.
