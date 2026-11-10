---
title: Files and paths
description: One-shot file reads and writes, cursor handles, directory listing, path helpers, and memory maps.
section: Overview
order: 0
---

# Files and paths

`@std/fs` is the filesystem layer of the standard library. It reads and writes whole files, lists directories, matches globs, reports metadata, creates symlinks, changes permission bits, locks paths, and maps memory. Path string helpers live in the same package, so a program does not need a second import for `Path.join`.

Most names are synchronous and blocking. The async versions that exist return a `Promise`: `readTextAsync`, `writeTextAsync`, `readBytesAsync`, `writeBytesAsync`, `statAsync`, `readDirAsync`, `globAsync`, `copyAsync`, `moveAsync`, and `renameAsync`. They run on a process-wide worker pool.

Fallible operations return `Result<T, String>`. A missing file is an `Err` with a message, not a trap. The exception is `File.open` and `File.create`: a refused open returns a handle whose `isOpen` is `false`, so check that field before reading or writing.

The main types are `File` for an open handle with a cursor, `FileStat` for metadata, `Path` for string helpers and read-only queries, and `Mmap` for mapped memory. The mode enums are `OpenMode`, `WriteMode`, and `CopyMode`.

```rnx
import fs, { Path } from "@std/fs";

let dir = Path.join("notes", "today.txt");
fs.mkdirAll(Path.dir(dir));
println!(fs.writeText(dir, "hello", .Overwrite).unwrapOr(0));
println!(fs.readText(dir).unwrapOr("missing"));
println!(fs.exists(dir));
fs.removeAll("notes");
```

That prints `5`, `hello`, and `true`.

## Guides

[Getting started](getting-started) has the first runnable program. [One-shot reads and writes](guides/reading-and-writing) covers whole-file text and bytes, write modes, and the async twins. [File handles](guides/file-handles) covers cursors, seeking, appending, locks, and memory maps. [Paths and globs](guides/paths-and-globs) covers `Path` and pattern matching.
