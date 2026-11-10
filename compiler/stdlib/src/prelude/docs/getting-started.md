---
title: "Getting started with @std/prelude"
description: "Use the ambient names with no import and run a first program."
section: Getting started
order: 10
---

# Getting started with @std/prelude

There is no install step and no import line. The prelude is already in scope in every file. `println!`, `Array`, `String`, `Result`, `Promise`, and the rest of the catalog are available as soon as a program starts.

## Copy this

```rnx
println!("no import needed");
let found = [1, 2, 3].find((n: Int): Bool => n > 1);
println!(found ?? -1);
assert("abc".contains("b"), "prelude");
```

Save it as the entry file of a project and run `rnx run`, or start with `rnx init` and replace `src/main.rnx`.

## What to expect

The program prints two lines. The first is `no import needed`. The second is `2`, because `find` returns the first value above 1. The `assert` call succeeds and produces no output. The process exits with status 0.

## When to import it

Almost never. The compiler loads the prelude before your file, so adding `import ... from "@std/prelude"` is redundant. Reference the package directly only when you need to disambiguate a name that your own declaration or another import shadows.

`@std/prelude` is also the package a target can replace. The built-in UEFI target selects `@std/uefi`, and the WASI target selects `@std/wasi`. See [Custom preludes](guides/custom-prelude) for the selection rules.
