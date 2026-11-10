---
title: Frames and damage
description: Build, draw, measure, and screenshot a frame, and repaint only changed regions.
section: Frames
order: 30
---

# Frames and damage

A frame is the unit the app builds and the host presents. `App.frame()` runs the
view, reconciles the tree, lays it out, and encodes a display list. It returns a
`Frame` with the list, a semantic snapshot, the damage regions, and the node
packet. The tree lays out at the window size, so a root column fills the window
and a grow row takes the space between fixed header and footer nodes.

`App.draw(frame)` renders the list into a new `Surface` and stores the status
code. `App.lastStatus()` reads it. `App.tick()` is the same thing in one call:
`draw(frame())`.

```rnx
import { App } from "@std/app"
import { Page, Text } from "@std/ui"

fn Main(): Int {
    let app = App.launchView("Frame", 320, 220, () => Page(title: "Frame") {
        Text("One frame")
    })
    let frame = app.frame()
    println!(frame.stats())
    let surface = app.draw(frame)
    println!(surface.rgbaAt(2, 2))
    println!(surface.backend())
    return 0
}
```

`Frame.stats()` reports node count, record count, byte count, dropped records,
and phase timings. `Frame.timeline()` returns just the timings. `Frame.records()`
and `Frame.bytes()` read the list size. `Frame.describe()` turns the list into
text for tests and the inspector.

`Frame.packet()` is the node layout and semantics buffer for that frame, and
`Frame.nodes()` decodes it. `Frame.scale()` and `Frame.colorSpace()` report the
tags the host receives. Scale is in thousandths, so 2000 is 2x. Color space is
0 for sRGB, 1 for linear, and 2 for Display P3.

`App.measureText(text, size)` returns the shaped width of one line through the
app's text engine. `App.measureStyledText` adds weight, italic, and tracking to
the measurement.

A `Surface` exposes the pixels. `pixel(x, y)` returns `0xRRGGBBAA` and
`rgbaAt(x, y)` returns `"r,g,b,a"`. `pixels()` is the raw RGBA8 buffer.
`draw()` tries the GPU and falls back to the CPU when no device answers;
`drawCpu()` forces the CPU path. `backend()` is `1` after a GPU draw, `2` after a
CPU draw, and `0` before either.

## Damage

Each frame carries the regions that changed. In the window path, the app builds
only the damaged regions and hands them to the host. The host keeps one target
texture, renders each region through a translated scene into a scratch texture,
copies that rectangle into the target, and blits the target to the swapchain.

The first frame, a resize, and any host target recreation are full frames. The
host increments a generation counter when it recreates the target, and the app
forces a full frame when it sees a new generation, so a cleared target is never
left partially painted. After each frame the app commits the tree, so damage
from input that arrived between frames is measured against the pixels on screen.

`App.frame()` returns the full list for tests and headless dumps.
`App.setDamageOverlay(true)` outlines the damaged regions so you can see them.
`RNX_TRACE=1` prints each present's regions and timing from the host.

Appearance and presentation changes invalidate the next frame themselves.
`App.setTheme`, `App.setScale`, `App.setColorSpace`, `App.setDpi`,
`App.setTextScale`, `App.setSafeArea`, `App.setKeyboardInset`, and
`App.setCamera` each mark a full repaint. The app calls the setter; it does not
ask for a manual redraw.

## Pacing

The view runs on every frame. `Tree.beginFrame()` reconciles it by position and
by explicit key, so node ids, focus, and handlers survive an unchanged frame.
Use `keyed` when a list can reorder.

`App.setAnimating(false)` stops continuous frames. The window then redraws once
per input event and sleeps between events. A view can make the same request with
`b.animate(false)`.

`App.clock()` is the `FrameClock` the view reads. `dt()` is the seconds since the
previous tick and `elapsed()` is animation time since the clock started. The app
ticks it once per frame, so animation state depends on the frame count instead of
wall time.

For deterministic tests, `App.setFixedFrameNs(ns)` replaces the real clock with a
fixed step, and `App.advance(ns)` builds one frame after advancing by `ns`:

```rnx
app.setFixedFrameNs(16000000)
app.advance(16000000)
let frame = app.advance(16000000)
println!(frame.stats())
```

## Screenshots

`App.screenshot(path)` renders the current frame on the CPU and writes a PNG.
`App.screenshotBytes()` returns the same PNG as a `ByteBuffer`.
`App.capture(path)` uses the host surface when a window is open and falls back to
the screenshot path when it is not. `Surface.encodePng()` and
`Surface.writePng(path)` are the lower-level versions.

## Keep going

Read [Input](input) to feed pointer and keyboard events into the same tree.
