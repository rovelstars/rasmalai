---
title: "Introduction"
description: "What Rasmalai is, how to install the toolchain, and your first running program."
icon: "Rocket"
---

# Introduction

This guide takes you from an empty folder to working programs: variables and numbers, functions, collections, error handling, modules, and the toolchain itself, with runnable examples at every step.

## Before you begin

To follow this guide, you should be comfortable with basic programming ideas already: variables, `if` statements, loops, and functions. You do not need to know Rust, C++, or TypeScript — this guide explains everything from scratch. If this is your first language, you can still follow along, but expect a steeper climb: work every example by hand, and when something breaks, the [Troubleshooting](/guide/11-troubleshooting-and-reinstall) page covers the usual beginner traps.

One more thing worth knowing up front: rnx has one central promise. **Every value is freed at a point you can see in the source.** No garbage collector pausing your program, no borrow checker arguing with you, no manual `free` to forget. A scope ends, its values drop — deterministically, on the same thread. Everything in this guide follows from that.

## Installing rnx

Everything ships in one binary. Grab it with the installer for your OS:

```sh
curl -fsSL https://rasmalai.rovelstars.com/install.sh | sh
```

The script detects your OS and CPU and places `rnx` in `~/.local/bin` (`%LOCALAPPDATA%\rnx\bin` on Windows, where you would run `irm https://rasmalai.rovelstars.com/install.ps1 | iex` in PowerShell instead). Now make sure it actually works:

```sh
rnx --version
```

You should see something like `rnx 0.1.0`. If the terminal says the command was not found, the installer directory is not on your `PATH` yet — open a fresh terminal (installers update `PATH` for new shells only) and try again. Still stuck? Jump to [Troubleshooting](/guide/11-troubleshooting-and-reinstall), which walks through reinstalling cleanly per OS.

> [!TIP]
> Not ready to install? Every example in this guide runs in the browser Playground — no setup at all. Installing is still worth it: the real toolchain adds files, packages, and tests.

## Your first program

With `rnx` on your `PATH`, scaffold a project and run it:

```sh
rnx init hello
cd hello
rnx run
```

You will see:

```text
Initialized project `hello` in .../hello
  manifest: Project.config
  entries.main: src/main.rnx
Next: cd hello && rnx run
```

and then, from `rnx run`:

```text
Hello from hello!
```

Congratulations — that is a real rnx program, compiled and executed. Open `src/main.rnx` and look at what you got:

```rnx
fn Main(): Int {
    print("Hello from hello!");
    return 0;
}
```

Here is what just happened. `rnx init hello` created a folder with a `Project.config` manifest and an entry file at `src/main.rnx`. `fn Main(): Int` is the entry function: the program starts there, `print` writes to the terminal, and the returned `Int` becomes the process exit code (`0` means success). Try changing the message, run `rnx run` again, and watch your edit take effect. That loop — edit, run, see — is the whole development cycle for the next few chapters.

## How this guide works

Each chapter follows the same rhythm: a short explanation, a complete program you can run, and then a breakdown of what the program did. callouts mark the traps:

- `> [!NOTE]` — something that surprises most newcomers.
- `> [!TIP]` — the easier way, once you know it exists.
- `> [!WARNING]` — something that will bite you if you ignore it.

Chapters link forward and back, so if a snippet uses something unfamiliar, follow the link instead of guessing. And if an error message confuses you at any point, [Error Handling](/guide/07-error-handling) teaches you to read diagnostics, and [Troubleshooting](/guide/11-troubleshooting-and-reinstall) covers everything else.

Ready? [Basics and Types](/guide/02-basics-and-types) is next: naming values and numbers that mean exactly what they say.
