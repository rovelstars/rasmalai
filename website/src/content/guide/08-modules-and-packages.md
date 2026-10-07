---
title: "Modules and Packages"
description: "ESM imports, the seeded standard library, and the project manifest."
icon: "Library"
---

# Modules and Packages

Real programs outgrow one file. rnx keeps growth boring on purpose: one namespace per file, imports that say exactly what they take, and a manifest that fits on an index card.

## One file, one namespace

Each file is its own namespace. Import names, aliases, whole namespaces, or a file for its side effects:

```rnx
import { Map } from "@std/collections";
import { AtomicInt } from "@std/sync";

let scores = new Map<String, Int>();
scores.set("ore", 7);
AtomicInt.byId(1).set(scores.len());
print(scores.get("ore"));
```

That prints `7`. Relative imports (`./math`, `./shapes`) wire up multi-file projects, and two files can each define a helper with the same name without colliding — names live in their file's namespace. Imports are per file: if `lib.rnx` names something it never imported, it fails, even when the entry file imports it. Dependencies travel with the file that declares them, never with the program around them.

An imported namespace is also a value. Passing it to `print` lists its exports, read-only:

```rnx
import io from "@std/io";

print(io);
```

That prints the module listing (`[Module io] { clear: ..., write: ..., ... }` — the exact set grows with the stdlib). Assigning to a namespace field is an error — namespaces are snapshots, not objects.

Every file also sees `@std/prelude` without importing it: `Int`, `Float`, `String`, `Bool`, `Array`, `print`, `assert` resolve from there, and only the ones you use reach the binary.

## Entry files run; imported files declare

Only the entry file — the `entries.main` file in `Project.config`, or the file handed to `rnx run` — executes loose top-level statements. That is where scripts live: no `main` wrapper needed for a quick task, top-level `await` for async work, and `return <Int>` for the exit code:

```rnx
import { Map } from "@std/collections";

let scores = new Map<String, Int>();
scores.set("ore", 7);
let total = await Promise.resolve(scores.len());
print("entries:", total);
if total == 0 {
    return 1;
}
```

That prints `entries: 1`. Every file pulled in through `import` is declarative-only: classes, structs, enums, functions, and constants. A loose `print` or `let` there fails with `E112` — move it into a function and call it from the entry file. An explicit `fn main()`/`fn Main()` in the entry still works exactly as before.

**Common mistake:** putting script logic in an imported helper file and wondering why nothing runs — or worse, getting `E112` and deleting the code. Imported files declare; only the entry executes. Move the statements into a function, import it, call it.

## The standard library is seeded, not embedded

`@std/` imports resolve from a global cache the installer seeds — nothing to install by hand, versions pinned automatically. Seventeen modules ship with the compiler:

| Module | What it holds |
|---|---|
| `@std/collections` | Generic `Map<K, V>` and `Set<T>` |
| `@std/sync` | `AtomicInt`, `Mutex`, `Channel`, `Barrier` |
| `@std/fs` | `File` and `Path` with text and binary I/O, one-shot operations with `*Async` pool twins, and `mmap` |
| `@std/bytes` | `ByteBuffer` fixed-size raw byte buffers |
| `@std/process` | `Process` host control and child lifecycles |
| `@std/os` | Platform queries (`OS.platform()`, `OS.cpuCount()`, ...) |
| `@std/simd` | `Vec4f` lanes and reductions |
| `@std/math` | Trig, `Vec2`, and numeric helpers |
| `@std/time` | Clocks and durations |
| `@std/random` | Seeded random generation |
| `@std/env` | Environment variables |
| `@std/testing` | Test helpers |
| `@std/web` | `URL`, `Headers`, status codes, query strings |
| `@std/json` | `JSON` parsing and stringification |
| `@std/net` | `TcpStream`, `TcpListener`, TLS, DNS |
| `@std/io` | Terminal streams with pretty-printing, line input, size queries, and raw mode |
| `@std/prelude` | Foundation types (auto-imported) |

Each module has a generated reference page under [its module docs](/docs/@std/simd/overview), rendered from the same `/** */` doc comments you write with `rnx doc`.

The cache is the offline story: with a seeded machine, builds never touch the network — missing entries download once from the registry, integrity-checked like any other package, and an explicit `dependencies` entry for an `@std/*` package overrides the pinned version. A cold cache with no network fails loudly (`E108` naming the registry, with an `rnx fetch-std` hint); `rnx fetch-std` seeds the cache and `rnx doctor --repair-std` repairs it. Offline labs are served by distribution — seeded installers and pre-warmed images carry the standard library.

The web playground works the same way, minus the filesystem. Its compiler engine downloads first, then preloads `@std/prelude` and fetches each further `@std/*` import from the same site's registry at the engine-pinned version, caching every module in the browser beside the engine bytes. Module paths stay identical, so code that checks in the playground checks the same on the command line.

A taste of the platform side — child processes and OS queries compose like everything else:

```rnx
import { Process } from "@std/process";
import { OS } from "@std/os";

print(OS.platform(), OS.cpuCount());
let out = Process.run("echo", ["hello"]);
print(out.exitCode, out.stdoutText().contains("hello"));
```

On a Linux box that prints something like `linux 16` followed by `0 true` — exit code zero, and yes, the output contained "hello".

## The project manifest

`Project.config` is an `.rnx` module exporting a default object. It declares the package name, version, and entry file. Dependencies use SemVer ranges, local paths, or git pins:

```rnx
export default {
    project: {
        name: "colony",
        version: "0.4.0"
    },
    entries: {
        main: "src/main.rnx"
    },
    dependencies: {
        sqlite3: "^3.45.0",
        physics_2d: { path: "../physics_2d" }
    }
}
```

`rnx lock` writes a deterministic `Project.deplock` checksum file; `rnx run --locked` and `rnx build --locked` re-verify every checksum before executing. Workspaces (`workspace` with `members`) hold monorepos whose members resolve to each other by name.

## Testing from day one

Tests live beside your code in `test fn` blocks — no parameters, no return, stripped from normal builds:

```rnx
fn add(a: Int, b: Int): Int {
    return a + b;
}

test fn adds_up() {
    assert(add(20, 22) == 42, "addition");
}

print(add(20, 22));
```

```sh
rnx test
```

```text
  ✓ adds_up 0.02 ms

tests: 1 passed, 0 failed in 0.14 ms
```

`assert` takes a `Bool` and a message. A failing assertion marks that test failed, but the rest of the suite keeps running. Write the test next to the function while the behavior is fresh in your head — coming back to add tests later rarely happens.

## Where to go from here

You now have the whole working language. Two directions remain:

- **Build something real** — the [standard library docs](/docs/@std/simd/overview) cover every module, and the [Playground](/playground) runs everything in your browser.
- **Understand the machinery** — the [Language Manual](/manual) covers the numeric model, deterministic ARC, protocols, concurrency, and the toolchain spec, normatively.
