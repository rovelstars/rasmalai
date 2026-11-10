---
title: Paths and globs
description: Join and split paths without touching the disk, then list or match real files.
section: Paths
order: 40
---

# Paths and globs

String concatenation is a poor path builder. It misses the separator, keeps trailing slashes, and cannot tell a directory from a file name. `Path` does the small pure operations, and `fs.readDir` and `fs.glob` answer the questions that need the disk.

```rnx
import { Path } from "@std/fs";

println!(Path.join("assets", "tile.png"));
println!(Path.dir("assets/tile.png"));
println!(Path.base("assets/tile.png"));
println!(Path.ext("assets/tile.png"));
println!(Path.isAbs("/tmp"));
```

That prints `assets/tile.png`, `assets`, `tile.png`, `.png`, and `true`.

`join` adds a `/` between the two segments and returns the other side when one is empty. It does not normalize `.` or `..`, and it does not check whether the path exists. `dir`, `base`, and `ext` ignore trailing slashes. `dir` returns `.` when there is no separator and `/` for a top-level absolute path. `ext` includes the dot and returns an empty string when the final name has no dot past its first character. `isAbs` checks for a leading `/`. On Windows, these helpers still use `/`, so do not use them to build native `C:\` paths.

`Path.exists`, `Path.isFile`, and `Path.isDir` do touch the disk. They are read-only checks and return `Bool`.

For more detail, `fs.stat(path)` returns a `FileStat` with `size`, `isFile`, `isDir`, `isLink`, `modified`, and `mode`. A symlink reports itself, so `isLink` is true and `size` is the stored target path length. `modified` is seconds since the Unix epoch, or `0` when unknown, and `mode` holds Unix permission bits, or `0` on platforms without them.

## Listing a directory

`readDir(path)` returns the entry names, sorted lexicographically. The sort is deliberate: directory order is not stable across filesystems, and a build that walks entries should produce the same output each time.

```rnx
import fs, { Path } from "@std/fs";

fs.mkdirAll("src/util");
fs.writeText("src/main.rnx", "", .Overwrite);
fs.writeText("src/util/strings.rnx", "", .Overwrite);
let names = fs.readDir("src").unwrap();
println!(names.join(","));
```

That prints `main.rnx,util`. The names are entries, not full paths, so join them with the directory when you need a path.

## Matching a glob

`glob(pattern)` returns matching paths, sorted. `*` and `?` match within one path component, and `**` matches any depth of directories. A pattern with no match returns an empty array, not an error.

```rnx
import fs from "@std/fs";

let top = fs.glob("src/*.rnx").unwrap();
let nested = fs.glob("src/**/strings.rnx").unwrap();
println!(top.join(","));
println!(nested.join(","));
```

That prints `src/main.rnx` and `src/util/strings.rnx`.

Both `readDir` and `glob` have async twins returning `Promise<Result<Array<String>, String>>`: `readDirAsync` and `globAsync`.
