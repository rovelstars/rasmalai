---
title: "Macros and Comptime"
description: "Macro expansion: syntax rewriting, then comptime derives and build-time evaluation."
section: "Language"
icon: "Zap"
---

# Macros and Comptime

Rasmalai macro expansion has two stages. Syntax expansion rewrites syntax before typechecking. Comptime expansion runs after typechecking and before code generation: derives, static assertions, and build-time evaluation. There is no separate macro language and no second virtual machine. Build-time functions are ordinary Rasmalai functions marked `#[CompTime]`, lowered like everything else, and executed in the existing interpreter under strict limits.

## Syntax expansion

Two forms expand before typechecking. They cannot see types.

```rnx
let v = vec![1, 2, 3];
let bytes = EmbedBytes!("assets/icon.bin");
```

`vec![...]` (or `vec!(...)`) becomes an array literal. `EmbedBytes!("path")` reads a file at build time and splices its bytes as an array of integers. The path must be crate-relative: absolute paths and `..` escapes are rejected, and a missing file fails the build. The file's content hash joins the expansion cache key, so editing the file rebuilds dependents and identical inputs reuse the cached expansion.

Lowercase `vec!` and the print family (`print!`, `println!`, `eprint!`, `eprintln!`) are the lowercase bang forms; other macros are `PascalCase!`. There is no ordinary `print` function; use the macros.

## Call macros

`@std/io` exports four macro functions: `print`, `println`, `eprint`, and `eprintln`. The stock prelude re-exports them, so the usual form needs no import:

```rnx
fn Main(): Int {
  println!("x = {}", 10)
  print!("no newline")
  eprintln!("error on stderr")
  return 0
}
```

A call macro is a function marked `#[Macro]` with exactly one parameter of type `Array<Any>` and a `Void` or `Any` return. The `!` call rewrites to an ordinary call with one array argument, so `println!("x = {}", x)` becomes `println(["x = {}", x])`. The array is typed `Array<Any>`, which lets `println!("x", 1, true)` mix argument types without a common element type.

Formatting runs at runtime:

- `println!()` writes one newline.
- If the first argument is a string with a `{}` placeholder, it is the format string. Each `{}` consumes the next argument and renders it with the shared pretty-printer.
- `{{` and `}}` produce literal braces.
- Without a placeholder, every argument is rendered and joined with one space.
- A missing argument leaves `{}` in the output and writes one warning to stderr for the process.
- Extra arguments are appended with a space.
- `print!` and `eprint!` add no newline. `println!` and `eprintln!` add one.
- `eprint!` and `eprintln!` write file descriptor 2.

Macros live in their own namespace. A macro name called without `!` is E108 and asks for the bang, and `helper!(...)` on a function that is not a macro is E108 and asks you to remove it. `print(...)` is E108 and points at `print!(...)`. There is no ordinary `print` function. You can import a macro explicitly instead of relying on the prelude:

```rnx
import { println } from "@std/io";

fn Main(): Int {
  println!("explicit {}", 7)
  return 0
}
```

A macro function is declared in a package like any other export:

```rnx
#[Macro]
export fn println(args: Array<Any>) {
  writeArgs(args, true, 1)
}
```

`export { println } from "@std/io"` keeps the macro flag, which is how the prelude makes the print family ambient.

## Logging macros

`@std/log` exports `log!`, `debug!`, `info!`, `warn!`, and `error!`. Each writes one line to stderr as `<LEVEL> <message>`. The stock prelude re-exports them, so no import is needed:

```rnx
fn Main(): Int {
  info!("starting")
  warn!("low disk: {}", 3)
  error!("failed: {}", "timeout")
  return 0
}
```

Formatting follows the print family: `{}` placeholders, `{{` and `}}` for literal braces, and space-joined arguments when no placeholder is present. `log!` uses the info level.

`RNX_LOG` sets the threshold:

- `RNX_LOG=error` shows only `error!`.
- `RNX_LOG=warn` adds `warn!`.
- `RNX_LOG=info` is the default and adds `info!` and `log!`.
- `RNX_LOG=debug` adds `debug!`.
- `RNX_LOG=off` hides every level.

A hidden level returns before formatting. A missing `{}` argument in a hidden message therefore produces no warning. Logging writes to file descriptor 2; `stdout` stays available for program data. Import a macro by name when a local declaration shadows the prelude:

```rnx
import { warn } from "@std/log";

fn Main(): Int {
  warn!("explicit {}", 1)
  return 0
}
```

`@std/log` declares the `log` capability. A target that does not provide `log` rejects the import with `E108` unless a prelude omits the package.

## Comptime expansion: derives, assertions, build-time values

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

A derive name other than `Clone` or `Debug` resolves to a crate-local `#[CompTime]` function. It receives the type info as an array of strings (the type name first, then one `field:Type` entry per field, sorted) and returns an array of method source strings, which are parsed, hygiene-renamed, and attached to the type:

```rnx
#[CompTime]
fn Describe(info: Array<String>): Array<String> {
  let name = info[0];
  return ["fn label(): String \{ return \"" + name + "\"; }"];
}

#[Derive(Describe)]
struct Point { let x: Int; let y: Int }
```

Derives never cross package boundaries in v1, and `Serde` is out of scope. Supported derives are `Clone` and `Debug`; other behavior uses a `#[CompTime]` function.

## Limits and sandbox

Build-time code runs with 1,000,000 steps of fuel, a call depth cap of 64, a 10,000-node output cap, and at most 8 fixed-point expansion rounds. Each cap has its own diagnostic (E117 through E120). Generated locals are gensym-scoped; a generated method that would collide with an existing member is a hard error (E121) pointing at both spans.

The sandbox is narrow: arguments, type info, and the pure prelude subset. It provides no clock, randomness, environment, network, ambient file reads, or foreign calls, and calls only into other `#[CompTime]` functions. File bytes enter exclusively through syntax expansion's `EmbedBytes!`. Violations fail closed (E122). `#[CompTime]` functions must be self-contained: they see their arguments, not module constants or derived methods.

Unknown `#[...]` attributes fail closed; the namespace is fixed and user extensions go through `#[Derive(Name)]`.

## Observability

Pass `--expand-macros` to dump the module before and after each stage into `.rnx-cache/expansions/`, and `--time-passes` to see the `syntax_expand` and `comptime_expand` spans alongside every other pass.
