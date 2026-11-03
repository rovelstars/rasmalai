# The app runtime

`@std/app` turns a `@std/ui` tree into pixels. It owns the window description,
frame encoding, the renderer call, and input dispatch. App code never imports a
wrapper, and no frame handle or present call is visible.

```rnx
import { App } from "@std/app"
import { Build, Node } from "@std/ui"

fn Main(): Int {
    let app = App.launch("Settings", 320, 240, (b: Build): Node => {
        let root = b.column()
        root.addChild(b.heading("Appearance"))
        root.addChild(b.checkbox("Reduce motion", false, () => {}))
        return root
    })
    let frame = app.frame()
    let surface = app.draw(frame)
    print(surface.rgbaAt(2, 2))
    return 0
}
```

## What each call does

- `App.launch(title, width, height, view)` builds the window and starts with the
  dark theme and an automatic GPU backend. Use `new App(window, theme, view)` to
  pass another theme.
- `App.launchWith(Renderer.vulkan, ...)` or `Renderer.gles` forces one backend.
  `App.setRenderer` changes it before the first GPU call. `Renderer.auto` tries
  Vulkan first and falls back to GL.
- `App.frame()` creates a fresh tree, runs the view, lays it out, and encodes a
  display list. It returns `Frame(index, list, snapshot)`.
- `App.draw(frame)` renders the list into a `Surface` and stores the status code.
  `Surface.draw()` is the same call on an existing surface.
- `App.tick()` is `draw(frame())`.
- `Surface.draw()` renders on the app's GPU device and falls back to the CPU
  when the device fails. `Surface.drawCpu()` forces the CPU renderer.
  `Surface.pixel(x, y)` returns `0xRRGGBBAA`; `rgbaAt(x, y)` returns
  `"r,g,b,a"`; `records()` reads the list record count. `Surface.pixels()` is
  the raw RGBA8 buffer, for dumps and tests.
- `App.pointerDown(x, y)`, `pointerMove`, `pointerUp`, and `keyDown(key)` route
  through the same handlers the headless core uses. `App.focusNext()` advances
  focus. `App.scrollAt` feeds the wheel.
- `App.bind(chord, run)` registers a shortcut such as `ctrl+s` or `f1`.
  `App.sendKey(code, down)` feeds an evdev key for tests and automation.
- `App.clock()` is the `FrameClock` the view reads for `dt()` and `elapsed()`.
- `App.setAnimating(false)` stops continuous frames. The window then redraws
  once per input event and sleeps in between.
- `App.show()` opens a Wayland window and loops until it closes. Escape closes.
  On runixOS Epiclese will replace this host behind the same call.
- `App.showFor(frames)` opens the same window, presents that many frames, and
  closes. Use it for a smoke test.

The view runs on every frame, and `Tree.beginFrame()` reconciles it by position
and by explicit key. Node ids, focus, and handlers survive an unchanged frame,
so state attached to a node does not move. `App.frame()` is the only call that
matters; the app does not see the reuse.

## Damage and pacing

Each frame carries the regions that changed. `App.frame()` returns a full list
for tests and headless dumps; the window path builds only the damaged regions
and hands them to the host. The host keeps one target texture, renders each
region through a translated scene into a scratch texture, copies that rectangle
into the target, and blits the target to the swapchain. The first frame, a resize, and any
host target recreation are full. The host increments a generation counter when
it recreates the target, and the app forces a full frame when it sees a new
generation, so a cleared target can never be left partially painted.

`App.runHost` presents while the view asks for frames and blocks on input while
it does not. `b.animate(false)` in the view, or `App.setAnimating(false)`
outside it, freezes the frame clock and stops presenting identical frames. On
Wayland the swapchain paces acquisition; an Epiclese host blocks on
`frame_done` there.

`RUNIX_DAMAGE=1` outlines damaged regions when an app enables
`App.setDamageOverlay`. `RUNIX_TRACE=1` prints each present's regions and
timing from the host.

## The window host

`App.show` calls `wrappers/host`, which speaks xdg-shell through `wayland-client`
and presents through wgpu and vello. The app never sees a surface, a frame
handle, or a present call. Pointer down, resize, and key down arrive through the
host pump and go into the last tree, so the same handlers run as in a headless
test.

The window maps as `app_id=dev.runix.app`. `examples/app-window` is the live
demo; set `RUNIX_SMOKE=1` to present three frames and exit.

## Text and backends

`measureText(text, size)` returns the shaped width through parley, and the
`TextEngine` installed by `App` lays out wrapped rows with the same crate. The
renderer shapes display-list text runs with it too, so layout and paint agree.

The app tests are the conformance layer for text. They build frames with no
window, inspect display-list geometry, and assert real results: a paragraph
wraps, a clipped control ends in `…`, a centered row moves, and a button is
exactly `measureText(label, size)` plus its padding. Run them on each target OS
to cover that OS's font defaults; exact pixel goldens are a separate, smaller
set per OS.

The runtime calls native wrappers, so `rnx run` needs their `.pc` files and
shared libraries. `tools/ci-wrappers.sh` builds all four, installs them into a
prefix, and runs the probes and the examples on Cranelift and LLVM. The
interpreter's foreign-call path accepts `Int`, `Bool`, and `Pointer` arguments
but not `Float`; use a compiled backend for an app that calls the runtime.
