---
title: The app runtime
description: Build windows, frames, input, routing, and tests with @std/app.
section: Overview
order: 0
---

# The app runtime

`@std/app` turns a `@std/ui` view into pixels and routes input back into it. It
owns the window, the frame loop, the renderer call, the accessibility
projection, and the test driver. App code stays on values: you return a `View`
from a function, and the package mounts it into the node tree.

```rnx
import { App } from "@std/app"
import { Button, Page, Text } from "@std/ui"

class Counter {
    let n: Int = 0
}

fn Main(): Int {
    let state = new Counter()
    let app = App.launchView("Counter", 320, 220, () => Page(title: "Counter") {
        Text("Clicks " + state.n.toString())
        Button("Add", () => { state.n += 1 })
    })
    return app.show()
}
```

## The main types

`App` owns one window, a theme, a view function, and a frame clock. Use
`App.launchView` for a `fn(): View`, or `App.launch` for a `fn(Build): Node`.
`App.show()` opens the window and runs until it closes. `App.frame()` builds and
encodes a frame without opening a window.

`Frame` is one encoded frame. It carries the display list, the semantic
snapshot, the damage regions, and the node packet. `App.draw(frame)` renders it
into a `Surface`, which holds RGBA8 pixels and can report a pixel, encode a PNG,
or write one to disk.

`Driver` drives the accessibility projection of the same tree that input uses.
It finds nodes by role and name, presses them, sets values, and takes
screenshots. `Mcp.on(driver)` projects the driver as MCP tools for agents.

`ViewRouter` maps paths to `View` values. `Route` and `RouteMatch` add nested
layouts and outlets. The older `Router` class does the same for `fn(): Node`
builders.

`AppBundle` writes the `app.json` manifest for a name, version, entry, and
capability list. `AppConfig` holds the title, size, appearance, and entry before
a window exists.

`WindowSet` tracks several windows and their focus, visibility, pause state,
and frame counts. `Renderer`, `Appearance`, `Gamepad`, `Accessibility`,
`Notifications`, `AppFiles`, and `Worker` cover the rest of the package.

## Guides

Read [Getting started](getting-started) for the import line and a first frame.
Then pick a topic:

- [Windows](guides/windows): open a host window, choose a backend, and manage
  suspension or extra windows.
- [Frames and damage](guides/frames-and-damage): build, draw, measure, and
  screenshot a frame, and repaint only what changed.
- [Input](guides/input): pointer, keyboard, touch, wheel, shortcuts, and
  gamepads.
- [Reactive state](guides/reactive-state): signals, computed values, effects,
  and keyed components.
- [Routing](guides/routing): flat and nested routes, params, history, and
  transitions.
- [Testing](guides/testing): drive the UI headless with `Driver`, scripts, and
  MCP.
- [Native wrappers](guides/native-wrappers): build the C ABI libraries behind
  the runtime.
- [Packaging](guides/packaging): declare an app and write its manifest.

Related packages include `@std/app/fonts` for a content-addressed font cache,
`@std/app/media` for audio, video, camera, and a file picker, and the
`@std/app/os/platform`, `@std/app/os/notifications`, `@std/app/os/files`, and
`@std/app/os/workers` modules.
