---
title: "Macros and Comptime"
description: "Two-phase macros: syntax-only expansion, then typed derives and build-time evaluation."
section: "Language"
icon: "Zap"
---

# Macros and Comptime

Rasmalai macros run in two phases. Phase A rewrites syntax before typechecking. Phase B runs after typechecking and before code generation: derives, static assertions, and build-time evaluation. There is no separate macro language and no second virtual machine. Build-time functions are ordinary Rasmalai functions marked `#[CompTime]`, lowered like everything else, and executed in the existing interpreter under strict limits.

## Phase A: syntax only

Two forms expand before typechecking. They cannot see types.

```rnx
let v = vec![1, 2, 3];
let bytes = EmbedBytes!("assets/icon.bin");
```

`vec![...]` (or `vec!(...)`) becomes an array literal. `EmbedBytes!("path")` reads a file at build time and splices its bytes as an array of integers. The path must be crate-relative: absolute paths and `..` escapes are rejected, and a missing file fails the build. The file's content hash joins the expansion cache key, so editing the file rebuilds dependents and identical inputs reuse the cached expansion.

Lowercase `vec!` is the only lowercase bang form; every other macro is `PascalCase!`. `print()` is an ordinary prelude function, not a macro.

## Phase B: derives, assertions, build-time values

`#[Derive(Clone, Debug)]` on a struct or class generates `clone()` and `debug()` methods from the declared fields. For a class, the initializer must take one parameter per field in order.

```rnx
#[Derive(Clone, Debug)]
struct Pair { let a: Int; let b: Int }

let p = Pair(1, 2);
let q = p.clone();
```

`StaticAssert!(condition, message)` evaluates a constant boolean at build time and fails the build with the message and span when it is false. The condition must be constant: literals and arithmetic over them.

```rnx
StaticAssert!(2 + 2 == 4, "arithmetic is broken");
```

`CompTime!(expression)` in a static initializer evaluates a constant expression or a call to a `#[CompTime]` function, then splices the resulting literal. Lookup tables land in the binary with no runtime cost:

```rnx
#[CompTime]
fn Table(): Array<Int> {
  return vec![10, 20, 30];
}

fn Main(): Int {
  static LUT: Array<Int> = CompTime!(Table());
  return LUT[0] + LUT[1] + LUT[2];
}
```

## User-defined derives

A derive name other than `Clone` or `Debug` resolves to a crate-local `#[CompTime]` function. It receives the type info as an array of strings — the type name first, then one `field:Type` entry per field, sorted — and returns an array of method source strings, which are parsed, hygiene-renamed, and attached to the type:

```rnx
#[CompTime]
fn Describe(info: Array<String>): Array<String> {
  let name = info[0];
  return ["fn label(): String \{ return \"" + name + "\"; }"];
}

#[Derive(Describe)]
struct Point { let x: Int; let y: Int }
```

Derives never cross package boundaries in v1, and `Serde` is out of scope: derive `Clone` or `Debug`, or write a `#[CompTime]` function.

## Limits and sandbox

Build-time code runs with 1,000,000 steps of fuel, a call depth cap of 64, a 10,000-node output cap, and at most 8 fixed-point expansion rounds. Each cap has its own diagnostic (E117 through E120). Generated locals are gensym-scoped; a generated method that would collide with an existing member is a hard error (E121) pointing at both spans.

The sandbox is narrow: arguments, type info, and the pure prelude subset. No clock, no randomness, no environment, no network, no ambient file reads, no foreign calls, and calls only into other `#[CompTime]` functions. File bytes enter exclusively through Phase A `EmbedBytes!`. Violations fail closed (E122). `#[CompTime]` functions must be self-contained: they see their arguments, not module constants or derived methods.

Unknown `#[...]` attributes fail closed; the namespace is fixed and user extensions go through `#[Derive(Name)]`.

## Observability

Pass `--expand-macros` to dump the module before and after each phase into `.rnx-cache/expansions/`, and `--time-passes` to see the `expandA` and `expandB` spans alongside every other pass.
