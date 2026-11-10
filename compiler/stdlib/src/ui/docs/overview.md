---
title: The @std/ui package
description: Build a screen as a View value, then mount it with @std/app.
section: Overview
order: 0
---

# The @std/ui package

`@std/ui` turns a screen into a value. A screen function returns a `View`, and
`@std/app` mounts that value into a retained node tree, lays it out, routes
input, and paints it.

The package is headless. It owns layout, hit testing, focus, and the semantic
tree, but it has no window and no renderer. `@std/app` owns the Wayland window,
the GPU surface, and the accessibility bridge. Keeping that line sharp means a
screen can be built and inspected in a test with no display attached.

The problem the package solves is state that changes over time. A view is
rebuilt from current state on every frame, while the retained tree matches the
new view against the old nodes by position and by key. A keyed row keeps focus,
scroll, and hover when the list reorders, instead of leaving them on the slot it
moved out of.

```rnx
import { App } from "@std/app"
import { Button, Checkbox, Header, Page, Spacer, Text, View } from "@std/ui"

class Tasks {
    let done: Bool = false
    fn clear() {}
    fn add() {}
}

fn TaskScreen(state: Tasks): View {
    return Page(title: "Today", padding: 20, gap: 12) {
        Header {
            Text("1 open")
            Spacer()
            Button("Clear", () => state.clear())
        }
        Checkbox("Write the docs", state.done, () => { state.done = true })
        Button("Add", () => state.add())
    }
}

fn Main(): Int {
    let state = new Tasks()
    let app = App.launchView("Today", 480, 360, () => TaskScreen(state))
    return app.show()
}
```

`Page` is a column with a heading, `Header` is a row, and `Text`, `Checkbox`,
and `Button` are leaves. A trailing block fills the `children` parameter, so the
nesting in the source matches the nesting on screen.

## The main types

`View` describes one node: its kind, its text, its children, and its style
attributes. The component functions `Page`, `Card`, `List`, `Text`, `Button`,
and the rest return `View` values. You rarely construct `View` directly.

`Build` and `Node` are the retained tree. `View.mount` creates or reuses nodes
through them, and `App.launch` takes a `fn(Build): Node` and hands that callback
the same tree for builder-style code. Treat `Build` as the escape hatch for
widgets the value API does not cover yet.

`Theme` carries colors and the type, space, radius, elevation, and motion
scales. The scales are typed lookups such as `theme.space(SpaceToken.md)`, not
string names.

`Reactive`, `Signal`, `Computed`, and `Effect` form an explicit state graph.
There is no process-global observer. A signal write schedules dependent effects.
`drain` runs the queue once, and the app calls it at the start of each frame.
`Reactive.component` makes a keyed subtree that rebuilds only when the signals
it read change.

`Transition` and `Presence` drive enter and leave fades. `Spring`, `Curve`,
`Decay`, `Motion`, and `KeyframeTrack` are pure motion values that you step with
a frame delta.

`TextSpan`, `RichText`, and `FontRegistry` cover styled runs, inline spans,
links, and named font faces.

## Where to go next

Start with [Getting started](getting-started) for a program you can run, then
read [Views](guides/views) for the value model and keys.

[Layout](guides/layout) covers containers and sizing.
[Components](guides/components) covers buttons, fields, images, and scenes.
[Text and fonts](guides/text-and-fonts) covers typography and rich text.
[State and reactive](guides/state-and-reactive) covers the signal graph.
[Animation](guides/animation) covers presence, springs, curves, and keyframes.
[Colors and themes](guides/colors-and-themes) covers `rgba`, `Theme`, and the
scales. [Input and focus](guides/input-and-focus) covers controls and keyboard
behavior. [Accessibility](guides/accessibility) covers roles, names, and states.
[Testing](guides/testing) covers headless snapshots and the app driver.
