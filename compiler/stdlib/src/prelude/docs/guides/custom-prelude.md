---
title: "Custom preludes"
description: "Select a different prelude package for a target and provide the names the compiler requires."
section: Preludes
order: 30
---

# Custom preludes

A target with no host console, no file system, or no operating system cannot use the stock prelude's full surface. It still needs the core catalog so the compiler can type `Int`, `String`, `Array`, `Result`, and the rest. A custom prelude supplies that catalog and only the extras the target can support.

## How a target selects one

The target descriptor names its prelude package. The built-in descriptors already do this: `x86_64-unknown-uefi` selects `@std/uefi`, and `wasm32-unknown-wasi` selects `@std/wasi`. Every other built-in target uses `@std/prelude`.

A project can select a package explicitly with an alias on the prelude spec:

```rnx
export default {
    project: { name: "probe", version: "0.1.0" },
    dependencies: {
        "@std/prelude": { alias: "@local/prelude" },
        "@local/prelude": { path: "../local_prelude" },
    },
}
```

The alias makes every lookup of `@std/prelude` resolve to `@local/prelude`, and the path entry points at the package that implements it. The package's entry file is loaded as the prelude source.

## What the package must declare

The prelude validator checks for the core names: `Int`, `Float`, `FastFloat`, `Bool`, `String`, `Char`, `Array`, `Map`, `Set`, `Date`, `Error`, `Any`, `Void`, `GenRef`, `Result`, and `Iterator`. `Null` is a language builtin and needs no declaration.

A missing name fails with `E108`. The message names the prelude package and the missing name, and the hint says to declare the name or choose another package. The fix is either to declare it in the prelude or to pick a package that already provides it.

`Promise` is not in the required list, because a target can omit async support. Omitting it disables async lowering: `async fn` and `await` fail with `E108` naming the prelude and `Promise`. Declare `Promise` when the target supports async.

## Start from an existing prelude

The safest starting point is the stock `@std/prelude` source. Copy the re-export list, remove the packages the target cannot build, and add the target's own console or logging surface. The UEFI package is a worked example: it re-exports the foundation names and provides `println!` without depending on `@std/io`, `@std/fs`, or `@std/log`.
