---
title: "Introduction: A Language That Frees On Time"
description: "What Rasmalai is, how to install the toolchain, and your first running program."
---

# Introduction: A Language That Frees On Time

Rasmalai is a systems language with one radical promise: **every value is freed at a point you can see in the source**. No garbage collector pausing your program, no borrow checker negotiating with you, no manual `free` to forget. A scope ends, its values drop — deterministically, on the same thread, with zero pause.

Everything else follows from that promise:

- **One static binary** (`rnx`) covers editing, running, testing, formatting, and shipping.
- **Native 64-bit types everywhere** — `Int` is always 64 bits, `Float` is always IEEE-754 double. No platform surprises.
- **Four backends, one semantics** — interpreter, Cranelift JIT, LLVM, and AOT binaries all run the same code the same way.

## Install the toolchain

Everything ships in one binary:

```sh
curl -fsSL https://rnx.dev/install.sh | sh
```

The script detects your OS and CPU and places `rnx` in `~/.local/bin` (`%LOCALAPPDATA%\rnx\bin` on Windows). Confirm it:

```sh
rnx --version
```

> [!TIP]
> Not ready to install? Every code example in this guide has an **Open in Playground** button that runs it in your browser — no setup at all.

## Your first program

Scaffold a project and run it:

```sh
rnx init hello
cd hello
rnx run
```

```text
Hello from hello!
```

The scaffold created two things: a `Project.config` manifest and `src/main.rnx`. Open the source file:

```rnx
fn Main(): Int {
    print("Hello from hello!");
    return 0;
}
```

The scaffold uses a named `Main` entry point and greets you with the project name. Top-level statements with no wrapper work too — that is the shape used for the rest of this page.

Three ideas are packed in here, and they recur everywhere:

- The entry file runs top-level statements directly — no `main` wrapper needed. A top-level `return <Int>` sets the process exit code (`0` means success); falling through returns `0`. An explicit `fn main(): Int` / `fn Main(): Int` still works when you want it.
- `print()` writes its arguments separated by spaces, then a newline. It works identically on every backend, which is why this guide uses it instead of a debugger.
- Blocks use curly braces and statements need no semicolons. If you have written Go, Rust, or TypeScript, your fingers already know the way.

Try changing it — add a second line and run again:

```rnx
let name = "ore";
print("hello,", name);
print("2 + 2 =", 2 + 2);
```

`rnx run` executes through the interpreter in milliseconds. When you want a real binary instead, `rnx build --release` compiles through LLVM and links a stripped native executable with the same output.

> [!NOTE]
> You will see `fn Main(): Int` (capital M) in some reference pages. Both spellings compile, and both are optional: top-level statements run as-is in files, in `rnx run`, and in the browser playground. Top-level `await` works there too.

## Check without running

`rnx check` parses and typechecks without generating code. It finishes in milliseconds, which makes it the command you will reach for most while editing:

```sh
rnx check
```

```text
Checked hello (ok)
```

Now break something on purpose — delete a brace — and run `rnx check` again. The diagnostic names an error code, points at the exact column, and suggests a fix. Every error in Rasmalai looks like this, and the codes are documented in the [Language Manual](/manual/16-project-and-toolchain).

## Where to go next

This guide walks you from zero to productive in nine short chapters:

1. **Basics and Types** — bindings, 64-bit numbers, strings, operators.
2. **Control Flow** — branches, loops, `switch`, and `defer`.
3. **Functions and Closures** — signatures, lambdas, and defaults.
4. **Data Structures** — structs, classes, records, enums.
5. **Collections** — arrays, maps, sets, and functional transforms.
6. **Error Handling** — `null` and `T?`, `Result`, `throws`, `try`/`catch`.
7. **Modules and Packages** — imports, the standard library, manifests.
8. **AI Assistants & MCP** — connect agents and harnesses through `rnx mcp`.
9. **Troubleshooting & Reinstall** — read diagnostics, fix breakage, reinstall cleanly.

Coming from another language? Jump to your [Rosetta guide](/guide/rosetta/from-rust) for side-by-side translations. Want exact semantics instead of stories? The [Language Manual](/manual/02-numeric-model) is the normative reference.
