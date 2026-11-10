---
title: Getting started
description: Import @std/app, run a first frame, and open a window.
section: Getting started
order: 10
---

# Getting started with @std/app

The package ships with the compiler, so there is no install step. Import it
next to the `@std/ui` values your view uses:

```rnx
import { App } from "@std/app"
import { Page, Text } from "@std/ui"
```

`@std/app` supplies the window and frame loop. `@std/ui` supplies `Page`,
`Text`, `Button`, and the rest of the view values.

## Copy this

```rnx
import { App } from "@std/app"
import { Page, Text } from "@std/ui"

fn Main(): Int {
    let app = App.launchView("Hello", 320, 240, () => Page(title: "Hello") {
        Text("Hello from @std/app")
    })
    return app.show()
}
```

Run it with the Cranelift backend, which is the `rnx run` default:

```sh
rnx run
```

The program opens a Wayland window titled `Hello` and draws one label. `Escape`
closes the window unless a dialog is open. `App.show()` returns the host status:
zero means the loop ended normally.

Native UI apps need the wrapper libraries. From the repository root, build the
wrappers and point the loader at the prefix:

```sh
compiler/stdlib/src/app/tools/build-wrappers.sh
export PKG_CONFIG_PATH="$PWD/.rnx-cache/wrappers:$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="$PWD/.rnx-cache/wrappers:$LD_LIBRARY_PATH"
rnx run
```

The interpreter's foreign-call path does not accept `Float`, so use a compiled
backend for any app that calls the runtime. Cranelift and LLVM both work.

## Check a frame without a window

A frame needs no compositor. This version builds the same view, encodes it, and
prints the node and record counts:

```rnx
import { App } from "@std/app"
import { Page, Text } from "@std/ui"

fn Main(): Int {
    let app = App.launchView("Hello", 320, 240, () => Page(title: "Hello") {
        Text("Hello from @std/app")
    })
    let frame = app.frame()
    println!(frame.stats())
    return 0
}
```

The output starts with `nodes`, `records`, `bytes`, `dropped`, and then the
build, layout, paint, and total timings. No window is opened, so this shape fits
a test or a machine without a display.

## Keep going

Read [Frames and damage](guides/frames-and-damage) to draw the frame, inspect
pixels, and understand the damage list. If you want the window loop first, open
[Windows](guides/windows).
