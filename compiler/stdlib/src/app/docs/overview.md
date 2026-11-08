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

The declarative value API is the shorter path:

```rnx
import { App } from "@std/app"
import { Page, Text, View } from "@std/ui"

fn Main(): Int {
    let app = App.launchView("Settings", 320, 240, () => Page {
        Text("Appearance")
    })
    return app.show()
}
```

## What each call does

- `App.launchView(title, width, height, view)` wraps a `fn(): View` in the same
  frame loop and mounts the value with `View.mount`.
- `App.launch(title, width, height, view)` builds the window and starts with the
  dark theme and an automatic GPU backend. Use `new App(window, theme, view)` to
  pass another theme.
- `App.launchWith(Renderer.vulkan, ...)` or `Renderer.gles` forces one backend.
  `App.setRenderer` changes it before the first GPU call. `Renderer.auto` tries
  Vulkan first and falls back to GL.
- `App.frame()` creates a fresh tree, runs the view, lays it out, and encodes a
  display list. It returns `Frame(index, list, snapshot)`. The tree is laid out
  with the window height, so a root column fills the window and a grow row takes
  the space between fixed header and footer nodes.
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
  `App.shortcuts().pushScope()` and `bindScoped` shadow global chords while a
  modal is open. `App.sendKey(code, down)` feeds an evdev key for tests and
  automation.
- `App.onGamepad(handler)` receives a `Gamepad` with axes normalized to -1..1
  and button state. The host reads `/dev/input/js*` and sends axis and button
  events; no device leaves the pad offline.
- `App.lockPointer`, `App.confinePointer`, and `App.releasePointer` use
  pointer constraints. A slider drag confines the pointer automatically and
  releases it on pointer up or leave.
- `Driver.on(app)` is the test driver over the accessibility projection.
  `find`/`exists` take a role and name; `press`, `click`, `increment`,
  `setValue`, and `focus` run the same handlers as input and rebuild the frame;
  `value` and `dump` read the result.
- `Mcp.on(driver)` projects that driver as MCP tools: `ui.list`, `ui.press`,
  `ui.increment`, `ui.setValue`, and `ui.screenshot`. `tools()` returns the
  schema and `call(tool, role, name, value, delta)` returns a JSON result. It is
  an opt-in projection; the app chooses the transport.
- `App.screenshot(path)` and `App.screenshotBytes()` render the current frame on
  the CPU and encode a PNG. `Driver.screenshot(path)` does the same after an
  action, and MCP `ui.screenshot` passes its `value` as the path. Tests and
  agents do not need a compositor screenshot. `Surface.encodePng()` and
  `Surface.writePng(path)` are the lower-level versions; the encoder uses stored
  deflate so it has no compression dependency.
- `Frame.packet()` is the node layout and semantics buffer for that frame;
  `Frame.nodes()` decodes it. `App.setScale` and `App.setColorSpace` tag every
  frame: scale is thousandths (2000 is 2x) and color space is 0 sRGB, 1 linear,
  or 2 Display P3. The host receives the packet through `host_frame_packet` and
  logs it under `RNX_TRACE`.
- The multi-window probe (`probe/multi_window.c`) is opt-in: run it with
  `RNX_MULTI_PROBE=1` and a Wayland session. It opens two toplevels, presents
  a neutral slate rectangle to each, prints the routing evidence, and closes.
- `App.setSurfaceSocket(path)` (or `RNX_SURFACE_SOCKET`) selects the surface socket
  transport inside `App.runHost`. Each presented frame sends the display list,
  frame packet, damage rectangles, and window id, and waits for the ack. The
  app tests start a fake socket server, call `showFor(2)`, and check two frames,
  window id 1, a non-empty display list, and a non-empty packet.
- The surface socket lives behind `host_cabi`: `host_surface_socket_open` connects
  a Unix socket with a hello (version, size, window id), `host_surface_socket_commit`
  sends a display list, optional frame packet, and damage rectangles, and reads
  the ack that repeats the window id. `host_surface_socket_id` and
  `host_surface_socket_close` complete the seam. A probe server and the Rust tests
  round-trip a committed frame and its id for one and two windows.
- Wheel events carry pixel deltas. The host accumulates the continuous Wayland
  axis value and emits whole-pixel steps, so a high-resolution touchpad scrolls
  by its actual distance instead of jumping a fixed notch. Control keys
  (backspace, enter, tab, escape) do not emit text events; only printable
  characters reach a focused field.
- The host can open more than one toplevel in one process: `host_open_window`
  creates a second surface with its own Wayland connection and GPU surface,
  `host_window_count/width/height/generation` describe them, `host_event_window`
  says which window produced the last event, and `host_present_window` routes a
  frame to a chosen id. `App.openWindow`, `windowCount`, `windowWidth`,
  `windowHeight`, `windowGeneration`, `eventWindowId`, and `presentWindow`
  mirror that surface. Each extra window is independent; the app decides when
  to build and present its frame.
- The full-stack surface spans a menu bar, tabs, sidebar/detail, search, grid,
  wrap, scroll, stack overlays, absolute placement, image, icon, slider,
  progress, divider, segmented control, notifications, popover, tooltip, toast,
  camera, portal, detented sheet with stage focus, matched geometry, overview and
  window morph, gesture arena, RTL, dynamic type, DPI, files, workers, and the
  platform module. A headless run drives a tab and the sheet through `Driver`,
  then prints accessibility and MCP counts. It also checks that a closed sheet
  left the tree, that `More` survives its popover, that every accessible node is
  inside the window, and that whitespace search text keeps the placeholder.
- A spatial desktop places five window cards at fixed world coordinates, with a
  camera that pans and zooms the world, a screen-space dock, a detented sheet,
  and a matched-geometry detail panel. Camera input applies the inverse to the
  world and keeps portal overlays in screen coordinates. A headless run drives
  the overview, sheet, and detail through `Driver`, checks the HUD stays inside
  the window and the overview fits every card, and can write a frame PNG.
- `WindowSet` tracks more than one window: `open`, `close`, `focus`, `route`,
  `resize`, `setVisible`, `suspend`, `resume`, and a per-window `frame` counter.
  Focus routing is a single id, so input goes to one window.
- Appearance and presentation changes invalidate the next frame themselves:
  `setTheme`, `setScale`, `setColorSpace`, `setDpi`, `setTextScale`, safe areas,
  keyboard insets, and camera changes mark a full repaint. The app calls the
  setter; it does not ask for a manual redraw.
- `App.suspend` pauses the frame clock and drawing, and `App.resume` restores
  the previous animation request and forces a full frame. The host maps the
  xdg-toplevel `suspended` state to events 23/24, and input is ignored while
  suspended.
- `@std/app/os/platform` exposes `Platform` (name, arch, OS checks, path
  separator, home, temp directory, CPU count) over `@std/os`.
- `@std/app/os/notifications` exposes `NotificationCenter` and `Notice` for an
  app-local queue.
- `@std/app/os/files` exposes `Files` for caller-path reads, writes, existence,
  and path joins. `@std/app/os/workers` wraps a `Thread` as `Worker` with
  `wrap` and `join`.
- `Router` is a path table with a back stack: `route`, `navigate`, `replace`,
  `back`, `currentPath`, and `view`. `AppConfig` holds the title, size,
  appearance, and entry; `theme(systemDark)` resolves `Appearance.system`.
- `Notifications` is an app-local queue: `post`, `dismiss`, `clear`, `count`,
  and `view(b)` for a toast stack.
- `AppFiles` forwards `readText`, `writeText`, `exists`, and `join` to
  `@std/fs` with caller-provided paths. `Worker.of(Thread.spawn(fn))` wraps a
  thread and `join()` returns its value.
- `AppBundle(name, version, entry, capabilities)` builds the `app.json`
  manifest and `writeTo(path)` writes it.
- `App.accessibility()` returns the frame's accessibility projection. The host
  encodes it into an AccessKit tree and publishes it over AT-SPI when the
  desktop enables accessibility. Screen-reader actions return as events and run
  the same pointer handlers. Each
  `A11yNode` carries its role, name, value, bounds, action mask, checked and
  disabled state, and focus. `find`, `at`, and `dump` inspect it; `press`,
  `increment`, and `setValue` route through the same handlers as pointer and
  keyboard input, so a screen reader action and a click cannot diverge.
- `App.copy`, `App.cut`, and `App.paste` work on the focused text box's
  selection. `Ctrl+C`, `Ctrl+X`, `Ctrl+V`, and `Ctrl+A` call the same methods.
  With a window open, copy writes the system selection through
  `wl_data_device` and paste reads it lazily; the app-local buffer remains the
  fallback for tests and headless runs. A selection change arrives as event 13
  and invalidates the cached text.
- `App.clock()` is the `FrameClock` the view reads for `dt()` and `elapsed()`.
- `App.setAnimating(false)` stops continuous frames. The window then redraws
  once per input event and sleeps in between.
- `App.show()` opens a Wayland window and loops until it closes. Escape closes.
  On runixOS, a surface socket host will replace this host behind the same call.
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
generation, so a cleared target can never be left partially painted. After each
frame the app commits the tree, so damage from input that arrived between
frames is measured against the pixels on screen.

`App.runHost` presents while the view asks for frames and blocks on input while
it does not. `b.animate(false)` in the view, or `App.setAnimating(false)`
outside it, freezes the frame clock and stops presenting identical frames. On
Wayland the swapchain paces acquisition; a surface socket host blocks on
`frame_done` there.

`App.setDamageOverlay(true)` outlines damaged regions. `RNX_TRACE=1` prints
each present's regions and
timing from the host, plus one line per scroll step.

## The window host

`App.show` calls `wrappers/host`, which speaks xdg-shell through `wayland-client`
and presents through wgpu and vello. The app never sees a surface, a frame
handle, or a present call. Pointer down, resize, and key down arrive through the
host pump and go into the last tree, so the same handlers run as in a headless
test.

The window maps as `app_id=dev.rasmalai.app`. Use `App.showFor(3)` to present
three frames and exit.

The host loads the Wayland XKB keymap and sends a text event for each key that
produces characters, so typing follows the keyboard layout instead of the US
table the app keeps as a fallback. Held keys repeat from the compositor's
repeat info: the host polls its socket with the next repeat deadline and emits
the same key and text events. `App.sendKey` stays the synthetic path for tests
and automation.

The host also speaks `text-input-v3`. When a text box has focus, the app enables
the text input with the caret rectangle, so the compositor can place its
candidate window. Preedit arrives as event 14 and is drawn inline without
changing the field value; commit arrives as event 15 and is inserted at the
caret; delete-surrounding arrives as event 16; leaving the surface clears the
preedit with event 17.

## Text and backends

`measureText(text, size)` returns the shaped width through parley, and the
`TextEngine` installed by `App` lays out wrapped rows with the same crate. The
renderer shapes display-list text runs with it too, so layout and paint agree.

The app tests also carry semantic and frame goldens. A deterministic screen is
snapshotted by the semantic tree and by a CPU-rendered RGBA frame; a missing
golden is written once and fails the run so it can be committed. Budget checks
print the 1k-node frame, 10k-row scroll, and idle damage timings and assert a
generous ceiling, which catches accidental quadratic work without turning a
busy runner into a flaky test.

The app tests are the conformance layer for text. They build frames with no
window, inspect display-list geometry, and assert real results: a paragraph
wraps, a clipped control ends in `…`, a centered row moves, and a button is
exactly `measureText(label, size)` plus its padding. Run them on each target OS
to cover that OS's font defaults; exact pixel goldens are a separate, smaller
set per OS.

The runtime calls native wrappers, so `rnx run` needs their `.pc` files and
shared libraries. `tools/ci-wrappers.sh` builds all four, installs them into a
prefix, and runs the probes and a smoke app on Cranelift and LLVM. The
interpreter's foreign-call path accepts `Int`, `Bool`, and `Pointer` arguments
but not `Float`; use a compiled backend for an app that calls the runtime.

## Native ABI widths

A wrapper function that the app reads as `Int` must return `int64_t` in its
header and `i64` in Rust. The call site reads the full 64-bit return register;
a 32-bit `int` return is zero-extended, so `-1` arrives as `4294967295`. That
breaks every negative status code: a wheel-up delta clamps a list to its
maximum, a GPU failure never reaches the CPU fallback, and a `-4` present
retry is read as success. Keep the header, the Rust signature, and the rnx
declaration on the same 64-bit width.
