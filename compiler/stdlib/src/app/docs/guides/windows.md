---
title: Windows
description: Open a host window, choose a renderer, and manage lifecycle and extra windows.
section: Windows
order: 20
---

# Windows

`App.show()` opens the platform window and runs the frame loop until the window
closes. On Linux the window comes from `wrappers/host`, which speaks xdg-shell
through Wayland and presents through wgpu and vello. App code never sees a
surface, a frame handle, or a present call.

`Escape` closes the window unless a dialog is open. `App.quit()` asks the live
host loop to close after the current frame. `show()` returns zero on a normal
exit and a negative status when the host fails.

## A fixed-frame check

Use `showFor(frames)` when you want the real host path without a person at the
keyboard. It opens the same window, presents that many frames, and closes:

```rnx
import { App } from "@std/app"
import { Page, Text } from "@std/ui"

fn Main(): Int {
    let app = App.launchView("Smoke", 320, 220, () => Page(title: "Smoke") {
        Text("Three frames")
    })
    return app.showFor(3)
}
```

This is the shortest check that the window, the GPU surface, and the display
list all load.

## Choose a renderer

`App.launchView` and `App.launch` use `Renderer.auto`, which tries Vulkan first
and falls back to GL. Call `setRenderer` before the first GPU call to force one
backend:

```rnx
import { App, Renderer } from "@std/app"
import { Page, Text } from "@std/ui"

fn Main(): Int {
    let app = App.launchView("Settings", 480, 360, () => Page(title: "Settings") {
        Text("Rendered with GL")
    })
    app.setRenderer(Renderer.gles)
    return app.show()
}
```

`App.launchWith(Renderer.vulkan, title, width, height, view)` makes the same
choice at launch time for a `fn(Build): Node` view. Use `App.renderer()` to read
the current choice. A process uses one backend.

`App.launch` and `App.launchView` start with the dark theme. Use
`new App(new Window(title, width, height), theme, view)` when you already have a
theme, or resolve `Appearance.system` through `AppConfig.theme(systemDark)`.

## Suspend and resume

`App.suspend()` stops drawing and pauses the frame clock, which is what a hidden
surface does. `App.resume()` restores the previous animation request and forces
a full frame. `App.isSuspended()` reports the current state. Pointer and touch
input are ignored while suspended.

The host maps the xdg-toplevel `suspended` state to these calls, so a minimized
window stops consuming frames without app code watching the compositor.

## More than one window

A live host can open another toplevel. `App.openWindow(title, width, height)`
returns the new window id, or a negative status before a host exists.
`windowCount`, `windowWidth`, `windowHeight`, and `windowGeneration` describe the
open windows. `eventWindowId()` says which window produced the last event, and
`presentWindow(id, frame)` sends a frame to a chosen window.

Each extra window is independent. The app decides when to build and present its
frame. `WindowSet` keeps the bookkeeping when you want to model that state
yourself:

```rnx
import { WindowSet } from "@std/app"

fn Main(): Int {
    let windows = new WindowSet()
    let main = windows.open("Main", 640, 400)
    let inspector = windows.open("Inspector", 320, 240)
    windows.focus(main)
    windows.frame(main)
    println!(windows.count())
    println!(windows.framesOf(main))
    println!(windows.route(inspector))
    return 0
}
```

`WindowSet` also has `close`, `resize`, `setVisible`, `suspend`, `resume`, and
per-window title, size, and visibility readers. Focus routing is a single id, so
input goes to one window.

## Surface socket

`App.setSurfaceSocket(path)`, or the `RNX_SURFACE_SOCKET` environment variable,
sends frames to a Unix socket instead of the Wayland window. Each presented frame
carries the display list, the frame packet, the damage rectangles, and the window
id, then waits for the ack. This is the surface path used by a runixOS host and by
the socket tests. `App.showFor(2)` still works with a socket set.

## Keep going

Read [Frames and damage](frames-and-damage) next to see what a frame contains
and how the host repaints it.
