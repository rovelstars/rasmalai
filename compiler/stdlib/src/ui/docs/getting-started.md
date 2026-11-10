---
title: Getting started
description: Import @std/ui, build a first screen, and mount it headless or in a window.
section: Getting started
order: 10
---

# Getting started

`@std/ui` ships with the compiler. There is no install step. Import what you
need:

```rnx
import { Page, Text, TextEngine, Tree, View } from "@std/ui"
```

## Copy this

This program builds a screen, mounts it into a headless tree, and prints the
semantic snapshot. It needs no window and no GPU.

```rnx
import { Page, Text, TextEngine, Tree, View } from "@std/ui"

fn Main(): Int {
    let tree = new Tree(TextEngine.fixed())
    tree.beginFrame()
    let root = View.mount(tree, Page(title: "Today", padding: 20, gap: 12) {
        Text("3 open")
        Text("Write the docs")
    })
    tree.endFrame()
    tree.layout(root, 320)
    println!(tree.snapshot())
    return 0
}
```

Run it with `rnx run`. The snapshot prints one line per node, with the role
first and the accessible name after it:

```text
column
  heading Today
  text 3 open
  text Write the docs
```

`TextEngine.fixed()` measures every character at the same width. It is a test
double, not the shaper the app ships, so use it for structure and not for
visual text layout.

## Put it in a window

For real pixels, hand the same screen to `@std/app`. `App.launchView` takes a
`fn(): View` and mounts it on every frame:

```rnx
import { App } from "@std/app"
import { Button, Page, Text, View } from "@std/ui"

fn Screen(): View {
    return Page(title: "Today", padding: 20, gap: 12) {
        Text("3 open")
        Button("Clear", () => {})
    }
}

fn Main(): Int {
    let app = App.launchView("Today", 480, 360, () => Screen())
    return app.show()
}
```

`App.show()` opens the window and runs until it closes. `App.showFor(3)`
presents three frames and exits, which is enough to smoke-test the path.

Building a windowed app needs the native wrapper libraries. The ui package
itself has no native dependency, so the headless program above runs anywhere
the compiler runs.

## Keep going

Read [Views](guides/views) next to learn how a `View` is composed, keyed, and
mounted. [Layout](guides/layout) explains how the containers size their
children.
