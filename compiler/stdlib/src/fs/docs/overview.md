# @std/fs

Filesystem one-shots, file handles, and path helpers.

```rnx
import fs from "@std/fs";

fs.writeText("hello.txt", "hello", .Overwrite);
print(fs.readText("hello.txt").unwrapOr("missing"));
```

## What is inside

- `OpenMode`
- `WriteMode`
- `CopyMode`
- `Mmap`
- `FileStat`
- `File`
- `Path`
- `FsTextReq`
- `FsBytesReq`
- `FsWriteTextReq`
- `FsWriteBytesReq`
- `FsStatReq`
- `FsDirReq`
- `FsCopyReq`
- `FsMoveReq`
- free functions: `mkdir()`, `mkdirAll()`, `remove()`, `removeAll()`, `copy()`, `move()`, `rename()`, `truncate()`

## When to use

Reach for @std/fs when a program persists anything. Bare names are sync and blocking one-shots (readText/writeText/copy/remove) with *Async twins on the process-wide pool; File handles carry a cursor with seek/tell plus advisory locks; Path covers joins and queries; mmap maps files into unsafe memory.

It covers: One-shot read/write plus *Async twins; File handles with cursor and locks; Path helpers; Mmap address mapping.
