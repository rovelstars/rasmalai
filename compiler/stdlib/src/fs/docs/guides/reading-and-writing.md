---
title: One-shot reads and writes
description: Store whole files as text or bytes, choose a write mode, and move the work off the main thread.
section: Files
order: 20
---

# One-shot reads and writes

For a small file, a handle is extra bookkeeping. The one-shot functions take a path and do the whole operation: `readText`, `writeText`, `readBytes`, `writeBytes`, `copy`, `move`, `remove`, and their relatives. Each returns a `Result`, so a missing file becomes a value you can handle.

```rnx
import fs from "@std/fs";

fs.writeText("greeting.txt", "hello", .Overwrite);
println!(fs.readText("greeting.txt").unwrapOr("missing"));
println!(fs.writeText("greeting.txt", " world", .Append).unwrapOr(0));
println!(fs.readText("greeting.txt").unwrapOr(""));
```

That prints `hello`, `6`, and `hello world`. `.Append` writes at the end and returns the byte count of the new text.

## Write modes

`writeText` and `writeBytes` take a `WriteMode`:

- `.CreateNew` fails when the file already exists.
- `.Create` creates a missing file and opens an existing one without truncating it.
- `.Overwrite` creates a missing file and truncates an existing one.
- `.Append` creates a missing file and writes at the end.

The mode is required. There is no default, because overwriting a file by accident is worse than typing one more word.

Byte files use `ByteBuffer`. The buffer holds the bytes and the call returns the count written.

```rnx
import fs from "@std/fs";
import { ByteBuffer } from "@std/bytes";

let bytes = ByteBuffer.fromArray([1, 2, 3]);
fs.writeBytes("data.bin", bytes, .Overwrite);
let back = fs.readBytes("data.bin").unwrap();
println!(back.length(), back.readUInt8(0));
```

That prints `3 1`.

## Other one-shots

`mkdir(path)` creates one directory and fails when the parent is missing; `mkdirAll(path)` creates every missing parent. `remove(path)` removes a file, symlink, or empty directory, and `removeAll(path)` removes a tree. Both report `Ok(false)` when the path was already missing.

`copy(from, to, mode)` takes `.Overwrite` or `.SkipExisting`. `move(from, to)` falls back to copy plus remove across filesystems. `rename(from, to)` does not fall back, so it fails across devices. `truncate(path, len)` pads with zeros when growing. `chmod(path, mode)` sets Unix permission bits, for example `493` for `0o755`. `symlink(target, link)` stores `target` in a new link, and `readLink(path)` returns the stored path. `fsync(path)` forces the file to durable storage.

## Async twins

The blocking functions stop the calling thread. The `Async` versions return a `Promise` and run on a worker pool. At the top level of an entry file, `await` works directly.

```rnx
import fs from "@std/fs";

fs.writeText("report.txt", "ok", .Overwrite);
let text: Result<String, String> = await fs.readTextAsync("report.txt");
println!(text.unwrapOr("missing"));
```

The pool size is the CPU count capped at 8. `fs.setWorkers(n)` clamps `n` into `1..8` and sizes the pool for later work; call it only when no async operation is in flight, because resizing stops the old pool and queued work never settles. `fs.fsPoolSize()` reports the current size, and `fs.fsWorkers()` reports the count the next ensure will use.
