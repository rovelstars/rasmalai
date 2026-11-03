---
title: "UI and App Runtime"
description: "The display-list byte format, the headless UI core, and the frame API in @std/app."
section: "Memory, Systems, and Concurrency"
icon: "Layout"
---

# UI and App Runtime

`@std/ui` builds a node tree, lays it out, routes input, and encodes a display
list. `@std/app` owns the window description, calls the renderer, and returns
pixels. Both packages are in development and vendored from the compiler tree:

```sh
export RNX_VENDOR_DIR=$PWD/compiler/stdlib/src
```

## Building a tree

```rnx
import { Build, Key, Node, Role, Tree } from "@std/ui"

fn screen(b: Build): Node {
    let root = b.column()
    root.addChild(b.heading("Appearance"))
    root.addChild(b.text("Choose how runixOS looks and moves."))
    root.addChild(b.checkbox("Reduce motion", false, () => {}))
    return root
}
```

The same tree can be written with blocks when the block form reads better:

```rnx
fn screen(b: Build): Node {
    return b.column() {
        b.heading("Appearance")
        b.checkbox("Reduce motion", false, () => {})
    }
}
```

`Tree.layout(root, width)` runs one measure pass and one place pass. Widths flow
down, heights come back up. `Tree.hitTest(x, y)` returns the deepest node under a
point, and `Tree.focusNext()` skips nodes that cannot be activated.

## Display list v2

`DisplayList` encodes a frame into one little-endian buffer: a 32-byte header,
an array of 64-byte records, and a blob table for UTF-8 text and PNG images.

| Offset | Header field | Type |
|---|---|---|
| 0 | magic `0x44584E52` | `u32` |
| 4 | version (`2`) | `u32` |
| 8 | record count | `u32` |
| 12 | record stride (`64`) | `u32` |
| 16 | blob table offset | `u32` |
| 20 | blob table bytes | `u32` |
| 24 | width | `u32` |
| 28 | height | `u32` |

Each record is `kind`, `color`, `a`, `b` as `u32`, then `x`, `y`, `w`, `h`, `e`,
`f` as little-endian `f64`. `color` is `0xRRGGBBAA`.

| Kind | Meaning | Payload |
|---|---|---|
| 1 | fill | rect in `x`, `y`, `w`, `h` |
| 2 | text | `a` and `b` are a blob range; `x` and `y` are the box top-left; `w` is the font size |
| 3 | clip push | rect in `x`, `y`, `w`, `h` |
| 4 | clip pop | none |
| 5 | rounded rect | rect in `x`, `y`, `w`, `h`; radius in `e` |
| 6 | image | `a` and `b` are a PNG blob range; target rect in `x`, `y`, `w`, `h` |
| 7 | transform push | `[a b c d e f]` across all six lanes |
| 8 | transform pop | none |
| 9 | polygon | `a` is the blob offset and `b` is the point count; each point is two `f64` values |

The builder has a fixed command cap and counts drops, so a frame never
reallocates or aborts halfway. `finish(width, height)` returns the bytes, and
`Display.textOf(bytes)` decodes a finished list for snapshots:

```rnx
import { DisplayList, Display } from "@std/ui"

let d = DisplayList.createDefault()
d.fillRect(0x14161AFF, 0.0, 0.0, 320.0, 240.0)
d.roundRect(0x1E2128FF, 16.0, 16.0, 120.0, 40.0, 8.0)
d.textRun(0xE6E7EBFF, 28.0, 28.0, 14.0, "Hello")
let bytes = d.finish(320, 240)
print(bytes.length())
print(Display.textOf(bytes))
```

`Painter.paint(tree, width, height, theme)` walks the tree and produces the same
primitives for every node. `Theme.dark()` and `Theme.light()` are data, so a
brand package can supply colors without forking the core.

## Damage and idle frames

A frame records which regions changed. `Tree.damageOf` compares each node's
bounds, label, value, checked state, and role with the previous frame; a canvas
node always counts because its paint function can draw anything. The rectangles
are merged and the window renders only those regions into a persistent target.

- The first frame and a resize are full frames.
- `Painter.paintRegions` drops nodes outside the damage, so the display list is
  proportional to what changed.
- An unchanged view produces no damage and no frame. While a view is paused, the
  app blocks on input, so an idle window uses no CPU and presents nothing.
- `RUNIX_DAMAGE=1` outlines changed regions, like the show-surface-updates
  developer flag. `RUNIX_TRACE=1` prints the rectangles and timing.

```rnx
let app = App.launch("Settings", 800, 600, view)
let frame = app.frame()
print(frame.damage().count())
```

## Text layout

Measurement and line breaking come from the shaper, not from a character-count
guess. `Tree` takes a `TextEngine`; `@std/app` installs a parley-backed one and
headless core tests use `TextEngine.fixed()`. The fixed engine is a test double
for logic tests, not the layout the app ships. Tests that assert text geometry
run against the app, where the real shaper and the OS font are available.

- `text` and `heading` fill the available width and wrap.
- Buttons and controls are single-line and clip with `…`.
- `Node.textAlign(Align.center)` or `Align.end` aligns each row.
- `Node.textWrap(false)` turns wrapping off and `Node.textMaxLines(n)` caps it.

Wrapping a paragraph is automatic once the node has a width:

```rnx
fn screen(b: Build): Node {
    return b.column() {
        b.text("A long paragraph wraps at the column width and grows the layout.")
    }
}
```

Every text node is laid out once per measure pass and the painter reuses those
rows, so a label is shaped once per frame.

## Kit, shortcuts, and virtual lists

The headless core carries the behavior, not just the look:

- hover, pressed, and disabled states
- sliders with `sliderValue(label, value, onChange)`
- text fields with caret, selection, and grapheme-cluster editing
- `Shortcuts` as a chord registry the app owns
- a virtual list viewport that builds only visible rows

```rnx
fn Main(): Int {
    let app = App.launch("Kit", 800, 600, view)
    app.bind("ctrl+s", () => save())
    app.bind("f1", () => showHelp())
    return app.show()
}
```

Chords are normalized to lower case, so `Ctrl+S` and `ctrl+s` are the same.
`bind` returns true when it replaced an existing binding.

A fixed-row list is a viewport plus a visible range:

```rnx
let v = b.virtualList("Rows", "rows", 10000, 30, 240)
root.addChild(v.node)
let i = 0
while i < v.count {
    let row = b.listItem("Item " + (v.first + i).toString(), () => {})
    row.fixedHeight(30)
    v.node.addChild(row)
    i += 1
}
```

The list keeps its scroll offset on the keyed node, snaps to row boundaries,
and `scrollAt` handles the wheel. Only the rows in view exist, so a 10,000-row
list holds about ten nodes. Clicking a control focuses it without painting the
focus ring; the ring appears for keyboard navigation. Dragging a slider out of
the window cancels the drag instead of jumping to an edge.

Text editing uses the `TextEngine` for grapheme boundaries. Typing, Backspace,
Delete, arrows, Home, and End work without a native text service; the window
host feeds evdev key codes, and `App.sendKey(code, down)` does the same for tests.

## Canvas and animation

`b.canvas(name, width, height, paint)` creates a fixed-size node that draws
itself. The callback gets a `Canvas` clipped to the node and works in local
coordinates:

```rnx
fn orbit(b: Build, angle: Float): Node {
    return b.canvas("orbit", 240, 240, (c: Canvas) => {
        c.fill(0x1A1D24FF)
        let points = [120.0, 40.0, 200.0, 200.0, 40.0, 200.0]
        c.polygon(0x48C4D6FF, points)
        c.text(0xD8DBE2FF, 12.0, 20.0, 12.0, "orbit")
    })
}
```

`Canvas` has `fill`, `rect`, `roundRect`, `polygon`, `line`, `text`, `clip`,
`unclip`, `pushTransform`, and `popTransform`. It appends to the same display
list as the built-in roles.

Animate from the frame clock instead of a timer:

```rnx
fn screen(b: Build, state: State): Node {
    let dt = b.frameClock().dt()
    if state.playing {
        state.angle += dt * 1.5
    }
    b.animate(state.playing)
    return orbit(b, state.angle)
}
```

- `b.frameClock().dt()` is the seconds since the previous frame;
  `elapsed()` and `frames()` are the running totals.
- `b.animate(false)` stops continuous frames. The app then redraws once per
  input event and sleeps, and the clock stops advancing, so every animation
  freezes in place.
- `App.setAnimating(false)` changes the same flag outside a view.

Advance physics in fixed substeps inside the view, so the result does not depend
on the refresh rate:

```rnx
let left = dt
while left > 0.0 {
    let h = 0.004166666666666667
    if left < h {
        h = left
    }
    left -= h
    velY += 1500.0 * h
    y += velY * h
}
```

## Frames

`@std/app` wraps the list and the renderer:

```rnx
import { App } from "@std/app"
import { Build, Node } from "@std/ui"

fn Main(): Int {
    let app = App.launch("Settings", 320, 240, (b: Build): Node => {
        let root = b.column()
        root.addChild(b.heading("Appearance"))
        root.addChild(b.buttonInert("Reset"))
        return root
    })
    let frame = app.frame()
    let surface = app.draw(frame)
    print(frame.records(), frame.bytes())
    print(surface.rgbaAt(2, 2))
    return 0
}
```

- `App.frame()` reconciles the tree, lays it out, and encodes the list.
- `App.draw(frame)` renders into a new `Surface` and stores the status.
- `App.tick()` is `draw(frame())`.
- `App.pointerDown(x, y)` and `App.keyDown(key)` use the same handlers as the
  headless core.
- `measureText(text, size)` returns shaped width through parley.
- `App.show()` opens a Wayland window and renders until it closes.
- `App.showFor(frames)` presents a fixed number of frames and closes.
- `App.launchWith(Renderer.vulkan, ...)` or `Renderer.gles` selects one GPU
  backend; `Renderer.auto` is the launch default.
- `App.setAnimating(false)` stops continuous frames; the window redraws on
  input only. `App.clock()` is the same `FrameClock` the view reads.
- `Surface.pixels()` is the RGBA8 buffer, for dumps and tests.

The view runs on every frame. `Tree.beginFrame` reuses nodes by position and by
explicit key, so node ids, focus, and handlers survive an unchanged frame. Use
`b.key("name")` before a factory when a node can move.

## Opening a window

On Linux, `App.show` uses `wrappers/host`, an xdg-shell client. It creates a
wgpu surface from the Wayland surface, renders each display list with vello, and
blits the result to the swapchain. Escape closes the window. Pointer clicks and
key presses go through the same handlers as the headless core.

```rnx
fn Main(): Int {
    let app = App.launch("runix window", 520, 360, view)
    return app.show()
}
```

`examples/app-window` is the live demo. `RUNIX_SMOKE=1` presents three frames
and exits, which is enough to check the path without a person at the keyboard.

## Requirements

The renderer is a native wrapper, so `rnx run` needs the wrapper libraries.
`compiler/stdlib/src/app/tools/ci-wrappers.sh` builds all four, installs them
into a prefix, and runs the probes and `examples/app-demo` on Cranelift and LLVM.
The interpreter's foreign-call path accepts `Int`, `Bool`, and `Pointer` but not
`Float`, so use a compiled backend for an app that calls the runtime.
