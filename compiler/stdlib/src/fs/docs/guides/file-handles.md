---
title: File handles
description: Read part of a file, append, seek, lock a path, and map anonymous pages.
section: Handles
order: 30
---

# File handles

Some work does not fit a whole-file read. You may need the header of a large file, a cursor that advances across several writes, or an advisory lock shared by threads. A `File` keeps the OS handle, the path, and the cursor.

Open an existing file with `File.open(path, mode)` and create or truncate one with `File.create(path, mode)`. `OpenMode` offers `.Read`, `.Write`, `.Append`, and `.ReadWrite`. Both calls return a handle; a refused open leaves `isOpen` false instead of trapping.

```rnx
import fs, { File } from "@std/fs";
import { ByteBuffer } from "@std/bytes";

fs.writeText("log.txt", "first\nsecond\n", .Overwrite);
let f = File.open("log.txt", .Read);
println!(f.isOpen);
println!(f.seek(6).unwrapOr(0));
println!(f.tell());
let buf = ByteBuffer.allocate(6);
println!(f.readBytes(buf).unwrapOr(0));
println!(buf.readString(0, 6));
f.close();
```

That prints `true`, `6`, `6`, `6`, and `second`. `seek` moves the cursor to a byte offset, `tell` reports it, and `readBytes` fills a buffer from the cursor and returns the byte count. At end of file it returns `0`.

## Writing and flushing

`File.create(path, .Overwrite)` truncates the file and leaves the cursor at the start. `.Append` opens for writing at the end. `writeText` and `writeBytes` write at the cursor and return the byte count.

```rnx
import fs, { File } from "@std/fs";

let out = File.create("log.txt", .Append);
out.writeText("third\n");
out.flush();
out.close();

println!(fs.readText("log.txt").unwrapOr(""));
```

`flush()` pushes buffered writes to the OS. `sync()` does that and then asks the OS to make the file durable; it returns a `Result`. `len()` flushes and reports the file size. `truncate(n)` resizes the file and pads with zeros when it grows.

## Refused opens and closed handles

A missing file opened with `.Read` gives `isOpen == false`. Every operation on that handle, or on a handle after `close()`, returns an `Err` instead of touching the disk. Check `isOpen` once after the open, then handle operation errors as values.

`close()` is safe to call twice. `File.fromHandle(fd, label, mode)` wraps an existing descriptor, which is how `@std/io` builds `stdin()`, `stdout()`, and `stderr()`. A from-handle file never owns the descriptor: closing it detaches the handle while the descriptor stays usable.

## Advisory locks

`lockSync()` blocks until the lock for this path is free, `tryLockSync()` returns `true` when it takes the lock and `false` when another handle holds it, and `unlockSync()` releases it. The lock is shared by handles on the same path across threads. It is not reentrant, so do not lock twice from the same thread without unlocking. The ids hash the path, so two unrelated paths can rarely share a lock. Treat it as advisory coordination, not a security boundary.

```rnx
import fs, { File } from "@std/fs";

let counter = File.create("counter.txt", .Overwrite);
counter.lockSync();
counter.writeText("1");
counter.unlockSync();
counter.close();
println!(fs.readText("counter.txt").unwrapOr(""));
```

## Memory maps

`fs.mmapAnon(len)` maps zeroed pages backed by nothing on disk. `fs.mmap(path, mode)` maps a file, which is an unsafe call because the address is raw memory. Both return `Result<Mmap, String>`. `len()` reports the mapped size, `flush()` writes dirty pages of a file map back to disk (it is a no-op for anonymous and read-only maps), and `close()` unmaps the region. After `close()`, every pointer taken from `address()` is invalid.

```rnx
import fs from "@std/fs";

let pages = fs.mmapAnon(4096).unwrap();
println!(pages.len(), pages.isOpen);
pages.flush();
pages.close();
println!(pages.isOpen);
```

That prints `4096 true` and `false`.

A file map must exist and must not be empty; size it with `writeText`, `writeBytes`, or `truncate` first. Never truncate a file while it is mapped: the OS can deliver SIGBUS on POSIX, which no `Result` catches. WebAssembly has no mapping support, so both map calls return an error there.
