---
title: "Hardware and FFI"
description: "unsafe boundaries, from-native imports, C-ABI export, files, and process control."
section: "Memory, Systems, and Concurrency"
icon: "Cpu"
---

# Hardware and FFI

## What unsafe means

Safe Rasmalai covers applications, servers, simulations, and tooling. The remainder — raw pointers, hardware registers, C libraries — is spelled `unsafe` and `native`. `unsafe` does not suspend checking; it assigns responsibility for three specific operations: taking addresses (`&x`), pointer arithmetic on `Pointer`-typed values, and calling `unsafe fn` functions. Bounds, types, and ownership keep working inside `unsafe` exactly as outside it.

```rnx
unsafe fn Raw(x: Int): Int { return x * 2 }

unsafe { print(Raw(21)); }
```

Calling an `unsafe fn` outside an `unsafe` block is an `E201` error; address-of or `Pointer` arithmetic outside one is `E202`. As an expression, `unsafe { ... }` runs the block and yields `null`, so it slots into statement position without affecting surrounding code.

Deliberately excluded from `unsafe`: intentional reference cycles (they waste memory at worst — see [Cycles and Handles](/manual/12-cycles-and-handles)) and data-race-free threading through `@std/sync`. `unsafe` is reserved for raw memory, C-ABI calls, and genuine data races.

> [!CAUTION]
> Keep `unsafe` blocks small: address-taking, pointer math, and foreign calls only. Business logic inside the block belongs outside it.

## Native declarations

Foreign functions declare inline with `from native "lib"`: plain signatures using `Int`, `Float`, `Byte`, `Bool`, `Pointer<T>`, and `: Ret`. No headers, no build-script bindings. At `rnx build` time the linker appends `-l<lib>` automatically (`"c"` is already in the default set); at `rnx run` time the JIT resolves each symbol from the host loader. A missing library or symbol fails the build with the symbol name attached.

```rnx
import {
    fn compressBound(sourceLen: Int): Int
} from native "z"

// Re-export without wrappers: downstream users import straight from your package.
export {
    fn compress(
        dest: Pointer<Byte>,
        destLen: Pointer<Int>,
        src: Pointer<Byte>,
        srcLen: Int
    ): Int
} from native "z"
```

Only C-ABI types cross the boundary; managed values (`String`, `Array`, objects, closures) are rejected with `E108` until converted to primitives or raw addresses explicitly.

## Raw pointers

A `Pointer<T>` is a raw 64-bit address (`Int`/`Float`/`Byte`/`Bool` pointees; `Byte` accesses are exactly one byte wide). Mint one from real memory with `Pointer.fromAddress<T>(addr)` — for example `buf.address()` on a `ByteBuffer`, which must outlive the pointer. Inside `unsafe`, `ptr.read()` loads `T`, `ptr.write(v)` stores it, `readVolatile`/`writeVolatile` mark the access volatile (LLVM honors it; Cranelift and the interpreter emit a plain access), and `*ptr` / `*ptr = v` sugar works the same. Pointer arithmetic is raw byte addition.

Taking `&local` yields an opaque token, not a readable address: dereferencing it fails cleanly at runtime, and native backends reject `&local` at compile time. For real memory, use `fromAddress`.

## Shipping C-compatible libraries

The reverse direction — calling Rasmalai from C — is `rnx build --lib`. Top-level `export fn` items with C-compatible signatures (`Int`/`Float`/`Bool`/`Void`, `String` params converted from `const char*`) compile to C-ABI wrappers in a static archive bundled with the runtime, plus a `<basename>.h` header with `RNX_<PACKAGE>_H` guards. Any other type in an `export` signature fails with `E108`, keeping the boundary honest by construction. (`pub`/`public` still parse as deprecated aliases with `W204`.)

## Auditing the surface

`unsafe` surface is counted: `rnx audit` reports `unsafe {}` sites, raw `Pointer`/`Address` uses, and `from native` imports per package for CI gating. Manifests carry the badge (`has_hazard: bool`), surfaced by `rnx doc` and `rnx audit` alike.

## Files

Filesystem work lives in `@std/fs`: `import fs, { File, Path } from "@std/fs";`. `File.open(path, mode)` returns a buffered OS handle; `mode` is `OpenMode.Read`, `Write`, `Append`, or `ReadWrite`. A refused open gives `isOpen == false` instead of throwing, and every other fallible handle operation answers `Result`. `File.create(path, mode)` opens for writing with `WriteMode.CreateNew`, `Create`, `Overwrite`, or `Append`. Opened files require `close()`. The text path (`readText`/`writeText`) and the binary path (`readBytes`/`writeBytes`) share the handle cursor; `seek`/`tell` move and read it, `len` measures the file, `flush` pushes to the OS while `sync` forces durable storage, and `lockSync`/`tryLockSync`/`unlockSync` take an advisory lock shared by every handle on the same path. The binary path moves bytes directly between the OS handle and a `ByteBuffer` with no intermediate copy.

```rnx
import { File, OpenMode } from "@std/fs";
import { ByteBuffer } from "@std/bytes";

let f = File.open("/tmp/rnx-notes.txt", OpenMode.Write);
f.writeText("hello bytes");
f.close();
let g = File.open("/tmp/rnx-notes.txt", OpenMode.Read);
let buf = ByteBuffer.allocate(32);
let n = g.readBytes(buf, 0, 11).unwrapOr(-1);
g.close();
print(n, buf.readString(0, n));
```

Reads on a closed handle answer `Err`, and a refused open reads as `isOpen == false`. One-shot work lives on the `fs` namespace, and every fallible call answers `Result` — only `File.open`/`File.create` keep the `isOpen` sentinel for a refused open:

| Call | Purpose |
|---|---|
| `fs.readText` / `fs.writeText` | whole-file text I/O (`WriteMode` picks create, overwrite, or append) |
| `fs.readBytes` / `fs.writeBytes` | whole-file binary I/O through `ByteBuffer` |
| `fs.stat` | metadata snapshot (`FileStat`: size, kind flags, modified time, mode bits) |
| `fs.exists` / `fs.isFile` / `fs.isDir` | boolean probes that never fail |
| `fs.readDir` / `fs.glob` | directory names / pattern matches, both sorted for deterministic builds |
| `fs.mkdir` / `fs.mkdirAll` | one level / recursive directory creation |
| `fs.copy` / `fs.move` / `fs.rename` | copy with `CopyMode`, cross-filesystem move, same-filesystem rename |
| `fs.remove` / `fs.removeAll` | one file or empty directory / full tree |
| `fs.symlink` / `fs.readLink` | create / read symbolic links |
| `fs.chmod` / `fs.truncate` / `fs.fsync` | permission bits / resize / flush to durable storage |

`Path.exists`, `Path.isFile`, `Path.isDir`, `Path.join`, `Path.dir`, `Path.base`, `Path.ext`, and `Path.isAbs` cover path queries without opening handles.

The bare name is sync and blocks the caller; appending `Async` queues the same work on the process-wide fs pool and returns a `Promise`. Await it — writing out the `Result` type, since `await` erases to `Any`:

```rnx
import fs from "@std/fs";

fs.writeText("/tmp/rnx-hello.txt", "hello", .Overwrite);
let back: Result<String, String> = await fs.readTextAsync("/tmp/rnx-hello.txt");
print(back.unwrapOr("missing"));
```

That prints `hello`. Every `Async` twin (`readTextAsync`, `writeBytesAsync`, `statAsync`, `readDirAsync`, `globAsync`, `copyAsync`, `moveAsync`, `renameAsync`) settles with the same `Result` its sync original returns.

`fs.mmap(path, mode)` maps a whole file into memory and `fs.mmapAnon(len)` maps zeroed pages; both build an `Mmap` whose raw start comes from `address()` for `Pointer.fromAddress`. Construction and `address()` both require `unsafe`, and writes through a `.ReadWrite` map reach the file after `flush()`:

```rnx
import fs from "@std/fs";

fs.writeText("/tmp/rnx-map.bin", "xxxxxxxx", .Overwrite);
unsafe {
    let m = fs.mmap("/tmp/rnx-map.bin", .ReadWrite).unwrap();
    let p = Pointer.fromAddress<Byte>(m.address());
    p.write(72);
    m.flush();
    m.close();
}
print(fs.readText("/tmp/rnx-map.bin").unwrapOr("?"));
```

That prints `Hxxxxxxx`. The lifetime rule is load-bearing: `close()` unmaps the region, so every address taken from it must be forgotten first — using one afterwards reads freed memory. And when another process truncates the file below a mapped page, the next touch faults in a way nothing can catch: the OS delivers SIGBUS on POSIX (an access violation on Windows), which no `Result` and no `catch` can cover. Size the file before mapping it and never truncate a mapped file.

## Terminal I/O

Terminal work lives in `@std/io`: `import io, { ColorLevel } from "@std/io";`. The default import binds the module namespace holding the free functions below; file I/O stays in `@std/fs`. Streams are `stdin()` / `stdout()` / `stderr()` calls returning `File` handles, each duplicating its descriptor, so closing the handle never closes descriptors 0, 1, or 2:

```rnx
import io from "@std/io";

let inn = io.stdin();
print(inn.isOpen);
inn.close();
```

`write` prints one value with a trailing newline on stdout; `writeError` matches it on stderr so error text stays separate when either side is piped; `writeRaw` writes bytes as-is with no newline and no pretty-printing:

```rnx
import io from "@std/io";
import { ByteBuffer } from "@std/bytes";

io.write("hello");
io.write([1, 2, 3]);
io.writeRaw(ByteBuffer.fromString("AB"));
io.writeError("boom");
```

One shared renderer formats every value: arrays as `[a, b]`, maps in insertion order as `{k: v}`, class instances in declaration order as `Name{field: value}`, results as `Ok(v)` and `Err(e)`. Nesting deeper than 3 renders as `...`, and a value that contains itself renders as `<cycle>`. Strings print as-is, never quoted and never re-escaped. Colors follow `colorProfile()` when stdout is a terminal and stay out otherwise. `print` forwards each of its arguments through this same renderer and joins them with spaces:

```rnx
import io from "@std/io";
import { Map } from "@std/collections";

let m = new Map<String, Int>();
m.set("ore", 7);
io.write(m);
io.write(Result.Ok<Int, String>(42));
io.write(Result.Err<Int, String>("boom"));
print([1, 2], "x");
```

That prints `{ore: 7}`, `Ok(42)`, `Err(boom)`, and `[1, 2] x` — a `Point(1, 2)` struct would render as `Point{x: 1, y: 2}`.

Input answers `Result`: `read` reads stdin to EOF, while `readLine` writes its prompt with no newline first, reads one line, and strips the trailing newline. Pipe input or type a line when running these; `readLine` reports `Err` on EOF with no bytes read:

```rnx
import io from "@std/io";

let r = io.read();
print(r.isOk());
```

```rnx
import io from "@std/io";

let line = io.readLine("> ");
print(line.isOk());
```

Size and color queries adapt to the terminal: `isTTY` reports whether stdin is a terminal, `width` and `height` fall back to 80 by 24 when the size cannot be read, and `colorProfile` derives the `ColorLevel` tier from `NO_COLOR`, `COLORTERM`, `TERM`, and the TTY state:

```rnx
import io, { ColorLevel } from "@std/io";

print(io.isTTY() == true || io.isTTY() == false);
print(io.width() > 0);
print(io.height() > 0);
print(io.colorProfile() == ColorLevel.Ascii || true);
```

`setRawMode` toggles character-at-a-time, no-echo input and answers `Result` rather than throwing — `Ok(true)` on success, `Err(message)` when stdin is not a TTY. Callers restore with `setRawMode(false)`; the runtime also restores the saved mode automatically at process exit. `clear` homes the cursor with TTY-guarding: a no-op returning normally when stdout is not a TTY, never an error:

```rnx
import io from "@std/io";

let r = io.setRawMode(false);
print(r.isOk() || r.isErr());
io.clear();
```

`print` stays in the prelude as a shim over `io.write`, so existing programs keep working with no import — color, newline, and pretty-printing decisions belong to `@std/io`.

## Process control

Child-process I/O uses `ByteBuffer` on both ends. `Process.spawn` returns a `ChildProcess` with piped `stdin`/`stdout`/`stderr`; `Process.run` captures a command to completion into `ProcessOutput` with `exitCode`, `stdoutText()`, and `stderrText()`.

```rnx
import { Process } from "@std/process";

let out = Process.run("echo", ["hello"]);
print(out.exitCode);
print(out.stdoutText().contains("hello"));
```

For interactive children, `writeStdin`/`readStdout` move `ByteBuffer` contents, `tryWait` polls (`null` while running, the code after exit), `wait` reaps and returns the exit code, and `kill(signal)` defaults to `15` (SIGTERM). Dropping a child detaches it; spawned children require `wait()`.

## Summary

- `unsafe` scopes three operations (`E201`/`E202` enforce the boundary); `from native "lib"` declares foreign functions.
- `rnx build --lib` exports C-ABI wrappers plus headers; `rnx audit` counts the surface.
- `File.open`/`create` with text and zero-copy binary paths plus `close`; `Path` for queries; `fs` one-shots with `*Async` pool twins; `mmap` under `unsafe` with OS-level fault rules.
- `io.write`/`writeError` with pretty-printing plus `writeRaw` for exact bytes; `read`/`readLine` answering `Result`; `isTTY`/`width`/`height`/`colorProfile` queries; `setRawMode` with `Result` and TTY-guarded `clear`; `print` as a prelude shim over `io.write`.
- `Process.spawn`/`run` with `ByteBuffer` pipes; `wait` reaps, `kill` signals, `tryWait` polls.
