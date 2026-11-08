---
title: "UI and App Runtime"
description: "The display-list byte format, the headless UI core, and the frame API in @std/app."
section: "Memory, Systems, and Concurrency"
icon: "Layout"
---

# UI and App Runtime

`@std/ui` builds a node tree, lays it out, routes input, and encodes a display
list. `@std/app` owns the window description, calls the renderer, and returns
pixels. Both packages are in development and vendored from the compiler tree.

App code should use the declarative value API in
[Declarative Views](/manual/22-declarative-views). The builder below remains
the retained-tree and runtime API.

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

The same tree can be written with blocks when the block form reads better.
Every node modifier returns the node, so a configured child is one expression:

```rnx
fn screen(b: Build): Node {
    return b.column().padding(20).gap(8) {
        b.heading("Appearance")
        b.checkbox("Reduce motion", false, () => {})
        b.button("Reset", () => {}).fixedWidth(96)
    }
}
```

Inside a block, a statement whose value is a node becomes a child. A statement
whose value is `Void` is a compile error (`E108`): a block statement must
produce a child, so hoist setup calls above the block.

`Tree.layout(root, width)` runs one measure pass and one place pass. Widths flow
down, heights come back up. `Tree.layoutIn(root, width, height)` adds a height
budget, so a root can fill the window and its grow children take the rest.
`Tree.hitTest(x, y)` returns the deepest node under a
point, and `Tree.focusNext()` skips nodes that cannot be activated.

### Layout styles

Every node takes optional styles. An unset value keeps the role default, so
existing screens keep their geometry.

- `padding(n)` is space inside the box; `margin(n)` is space outside it.
- `gap(n)` is the space between children.
- `grow(f)` takes a share of the free main-axis space.
- `alignMain(Align.center)` or `Align.end` places the child stack.
- `alignCross(Align.center)`, `Align.end`, or `Align.stretch` places children
  across the axis. A row stretches height; a column stretches width.
- `minWidth`, `maxWidth`, `minHeight`, and `maxHeight` clamp the measured box.
- `fixedWidth` and `fixedHeight` pin a size.

A row uses equal slots until a child sets a fixed width or `grow`. Then the
fixed children keep their size and the grow children share what is left:

```rnx
let root = b.column()
let shell = b.row()
shell.grow(1.0)
shell.alignCross(Align.stretch)
let sidebar = b.column()
sidebar.fixedWidth(220)
let detail = b.column()
detail.grow(1.0)
shell.addChild(sidebar)
shell.addChild(detail)
root.addChild(shell)
```

`App` lays out with the window height, so a root column fills the window and a
grow row fills the space between a fixed header and footer.

### Containers

Three containers cover layouts that a row or a column cannot express:

- `b.stack()` overlays children at one origin. Later children paint and hit on
  top, which is enough for a badge, a scrim, or a floating card.
- `b.scroll()` is a clipped viewport. Give it a height with `fixedHeight` or
  `grow`, add children as a column, and move it with the wheel, `scrollAt`, or
  `Node.scrollTo`.
- `b.wrap()` flows children into lines at the available width, so chips and tags
  wrap instead of overflowing.
- `b.grid(columns)` places children in fixed columns, with each row as tall as
  its tallest cell.
- `b.divider()` and `b.progress(v)` cover rules and value bars.
- `b.panel()` is a padded surface, and `b.segmented(labels, selected, onSelect)`
  is a row of radios.
- `b.image(png, w, h)` is a bitmap control; an empty buffer draws a placeholder.
- `b.icon(name, size)` draws `plus`, `close`, `check`, or `chevron`.
- `b.menu(labels, onSelect)` is a vertical list of actions.
- `b.popover(width)` is an absolute panel for a menu or a card.
- `b.toast(message)` pins a message to the bottom right of its parent.
- `node.tooltip(text)` shows a label while the node is hovered.
- `b.spacer()` is an empty stack with `grow(1.0)`, for pushing siblings apart.

A child can leave the flow with `absolute()` and pin itself with `left`, `top`,
`right`, and `bottom`. Two opposite edges size the child between them. A child
can override its parent's cross-axis alignment with `alignSelf`, and
`aspectRatio` sets height from width when no height is given.

`WritingMode.vertical` stacks text down the block axis, one glyph per baseline.
The measure pass caches measured boxes by node, available size, exact flag, and a
layout version bumped by every sizing setter, so repeated layouts skip unchanged
work and any size or text change invalidates the cache.

`rtl()` mirrors row order and start/end text alignment. `App.setSafeArea` and
`App.setKeyboardInset` shrink the root around system bars and the soft keyboard.
`App.setTextScale` applies dynamic type to measurement and painting together,
and `App.setDpi` records the device pixel ratio, tags the frame at that scale,
and exposes `tree.dp(n)` for logical-to-device conversion.

`Constraints` carries min, max, and ideal for both axes as one value.
`node.constrain(c)` gives it to a child and
`Tree.layoutConstrained(root, width, height, c)` applies it to the viewport;
`Constraints.tight`, `loose`, and `unbounded` cover the common shapes.

`idealWidth` and `idealHeight` give a preferred size; min and max still clamp
it, taller content still wins, and a flex child that reaches `maxWidth` hands
the leftover to its grow sibling.

`b.tabs(labels, selected, onSelect)` and `b.menuBar(labels, onSelect)` cover
tab strips and a menu bar, with tab and menubar/menuitem roles.

`zIndex(n)` orders siblings for paint and hit testing: a higher value paints
later and wins the hit. `gapPercent(p)` sets a gap from the container width,
and `autoMargin()` turns free main-axis space into margin, which centers a child
or pushes it to an edge.

`order(n)` sorts siblings, and `alignCross(Align.baseline)` lines up the first
text baselines in a row. `minContent`, `maxContent`, and `fitContent` choose a
width from the content: the widest word, the full line, or the full line capped
by the available width.

```rnx
let page = b.scroll()
page.fixedHeight(320)
let row = b.row()
row.addChild(b.button("Back", () => {}))
row.addChild(b.spacer())
row.addChild(b.button("Next", () => {}))
page.addChild(row)

let overlay = b.stack()
overlay.addChild(b.text("content"))
let badge = b.text("3")
badge.fixedWidth(24)
badge.fixedHeight(24)
badge.absolute()
badge.right(8)
badge.top(8)
overlay.addChild(badge)
```

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

Images are views. `pngSize` reads the IHDR header, and `Image` fits the decoded
PNG inside its box:

```rnx
let png = fs.readBytes("assets/icon.png").unwrap()
Image(png, fit: ImageFit.cover, width: 320, height: 200)
```

The fit modes are `fill`, `contain`, `cover`, `scaleDown`, and `none`. `cover`
clips to the node box; `none` draws at the intrinsic size. `imageSize` reads PNG,
JPEG, WebP, or SVG headers without decoding pixels. SVG rasterizes through
`resvg` at the requested display size. The CPU renderer decodes those formats on
a bounded worker queue, so the frame thread never blocks; the next
frame paints from the ready cache, and `Surface.waitImages` /
`cpu_wait_images` drains the queue for screenshots and tests. The view projects
as an image role, so accessibility bounds and screenshots match the pixels.
A single screen can mix all of these: one icon under every fit mode, a WebP
tile, an SVG card, a decoded video frame, and a camera frame.

## Media playback

`@std/app/media` adds audio, video, camera, and a native file picker:

```rnx
import { AudioPlayer, CameraStream, FilePicker, VideoPlayer } from "@std/app/media"

let audio = AudioPlayer.open("assets/theme.wav")
audio?.play()

let video = VideoPlayer.open("assets/clip.mp4")
let frame = video?.next()
let picture = Image(frame, fit: ImageFit.contain, width: 320, height: 240)

let camera = CameraStream.openDefault(640, 480)
let shot = camera?.next()

let path = FilePicker.pickFile("Open media", "png,jpg,wav,mp4")
```

`AudioPlayer` decodes with FFmpeg, resamples to stereo, and feeds a bounded
two-second ring to ALSA or a null sink. `VideoPlayer` decodes on a worker and
keeps at most four RGBA frames queued; `next` never blocks. Video and camera
frames use the same raw RGBA image path as decoded files, so no PNG round trip
is involved. `CameraStream` uses V4L2 and converts YUYV or RGB24 to RGBA.

Headless runs use the stubs: `RNX_CAMERA=stub` generates moving frames,
`RNX_AUDIO=fast` drains the audio queue without pacing, and
`RNX_PICK_FILE` supplies the picker result. A real picker uses the XDG portal
and is only opened in a windowed session. Video tests read `RNX_TEST_VIDEO`
and use the camera and picker stubs.

`Painter.paint(tree, width, height, theme)` walks the tree and produces the same
primitives for every node. `Theme.dark()`, `Theme.light()`, and
`Theme.highContrast()` are data, so a brand package can supply colors without
forking the core.

The theme carries semantic colors beyond the original six: raised and sunken
surfaces, muted text, accent hover, accent text, focus, danger, success, warning,
and disabled. `Button(tone:)` selects neutral, accent, danger, or ghost without
hardcoding a color. Ghost buttons use a real rounded stroke, not a transparent
fill over an opaque border. `mix` interpolates alpha, so compositing stays
correct.

The same theme carries the type, space, radius, elevation, and motion scales:

```rnx
let body = theme.type(TypeToken.body)
let gap = theme.space(SpaceToken.md)
let radius = theme.radius(RadiusToken.md)
let lift = theme.elevation(ElevationToken.level2)
let normal = theme.motion(MotionToken.normal)
```

These are typed values, not string lookups. The default scale matches the
geometry the roles used before tokens existed, so adopting tokens does not move
a screen. `Card(elevation: ElevationToken.level2)` emits the existing rounded
shadow records, and `Button(size: ButtonSize.lg)` resolves padding and height
through the space scale. `Theme.withSpaceScale(SpaceScale.compact())` tightens
spacing without changing type sizes. `Theme.dark()`, `light()`, and
`highContrast()` share the same type scale and differ in color and borders.

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
- `App.setDamageOverlay(true)` outlines changed regions, like the
  show-surface-updates developer flag. `RNX_TRACE=1` prints the rectangles and timing.

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
    let row = b.listItem("row " + (v.first + i).toString(), () => {})
    row.fixedHeight(30)
    row.offsetY(0.0 - v.offset)
    v.node.addChild(row)
    i += 1
}
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
Typing in a live window goes through the host's XKB keymap, so non-US layouts
and shifted characters arrive as text. Held keys repeat from the compositor's
repeat info. `sendKey` is still the synthetic path for tests and automation.
`Ctrl+C`, `Ctrl+X`, `Ctrl+V`, and `Ctrl+A` copy, cut, paste, and select all in
the focused text box. A live window uses the system selection through
`wl_data_device`; headless runs use an app-local buffer. A dialog restores focus
to the control that opened it when it closes, and `Shortcuts.pushScope` lets a
modal shadow global chords.

A live text field also enables `text-input-v3` while it has focus. The
compositor sends preedit and commit events; the preedit is drawn at the caret
without touching the value, and the commit is inserted when it arrives. The
caret rectangle is sent with the enable request so the candidate window lands
next to the field.

`App.onGamepad(handler)` reports a normalized `Gamepad`: axes are -1..1 and
buttons are pressed or released. The host discovers `/dev/input/js*` devices.
`App.lockPointer`, `App.confinePointer`, and `App.releasePointer` use Wayland
pointer constraints; a slider drag confines the pointer while it is held.

`App.accessibility()` returns the frame's accessibility projection: every
semantic node with role, name, value, bounds, action mask, checked, disabled, and
focus state. `press`, `increment`, and `setValue` on that projection run the
same handlers as a pointer or keyboard action, so a screen reader and a click
cannot diverge.

The host publishes the projection through AccessKit over AT-SPI. When the
desktop enables accessibility, the application appears on the assistive bus with
its role, name, bounds, state, and actions; pressing a button there returns an
action event and runs the same handler as a pointer click.

Every frame also carries a node packet: one buffer with each live node's id,
parent, role, flags, bounds, actions, name, and value, plus a header holding the
surface size, output scale, and color space. `frame.nodes()` decodes it, so a
host or a test reads layout and semantics without walking the tree. Scale is in
thousandths (`2000` is 2x), and the color space is sRGB, linear, or Display P3.

### Spatial stage

The stage is an optional layer over a normal tree. Layout never changes; the
stage changes presentation.

- `Camera(scale, x, y)` transforms the scene. `zoomAt` keeps the point under the
  cursor fixed, and hit testing reverses the transform, so clicks land where the
  pixels are.
- `node.portal(layer)` keeps a node in its logical parent but paints it above
  the tree inside the layer. It wins hit testing there, which is what makes
  menus that escape a clipped list and drag previews work.
- `Spring` carries velocity across retargets, so an interrupted animation
  continues instead of restarting.
- `Sheet.of(detents, index)` is a detented surface: `dragBy` clamps,
  `release(velocity)` settles or flings one detent, and `step(dt)` advances it.
- `Match` interpolates a rectangle between a source and a target for matched
  geometry.
- `Ambient` throttles a covered layer and freezes a hidden one.

`GestureArena` resolves competing drags: each claim names an axis, a priority,
and a depth. The highest priority wins, ties go to the deeper node, and a winner
holds until release so a sheet cannot steal a scroll mid-flick. `node.inert()`
removes a subtree from focus, hit testing, and semantics; `Stage.present` applies
it to the content while a sheet is up, moves focus into the overlay, throttles
the ambient layer, and `Stage.dismiss` restores all four. Node flags reset on
each rebuild, so call `Stage.sync()` from `onMount` while the stage is open.
The accessibility projection marks inert nodes so a screen reader skips the
background.

Accessibility relations use stable tree ids. Name a label node and point a
control at it; the host maps the relation to AccessKit:

```rnx
let emailLabel = b.text("Email")
let email = b.textField("", "you@example.com", onText)
email.labelRel(emailLabel.id)
email.describedBy(hint.id)
```

The window host forwards Wayland touch down, move, up, and cancel events with a
stable touch id. `App.touchDown`, `touchMove`, `touchUp`, and `touchCancel`
feed the same pointer table as the mouse, so two fingers drive pinch and
rotation without a separate gesture path. `Node.controls(id)`,
`Node.describedBy(id)`, and `Node.labelRel(id)` write the v6 relation lanes. `App.announce("Saved", 1)` posts a polite live announcement
(priority `2` is assertive); it appears once in the packet as a synthetic live
node and does not enter `ui.tree` or Driver node lists. The script step
`announce:Saved:1` drives the same path. The host diffs the packet, so an
unchanged relation or announcement produces no AccessKit update.

`Overview(viewW, viewH, padding)` fits a set of window rectangles into the
viewport and maps each card through the resulting camera; `enter` saves the
camera and `exit` restores it in place. `WindowMorph` pairs a window rectangle
with an overview thumbnail using matched geometry, and `swapped()` flips at the
halfway frame so the content changes once.

A real spatial screen places five window cards at fixed world coordinates on a
larger canvas, the camera pans and zooms them, and the dock, sheet, and detail
panel stay in screen space through portals. Camera input applies the inverse
transform to the world and keeps portal overlays in screen coordinates.
Accessibility bounds follow the same rule: world nodes project through the
camera and portals keep screen-space bounds, so a driver click and a screen
reader agree with the pixels. The world stays a normal tree; the stage adds
layers, not a second layout system.

A full app combines every control, the spatial stage, camera, portal, sheet,
matched geometry, overview, notifications, files, workers, platform queries,
RTL, dynamic type, and DPI, and can be driven headlessly through `Driver`.

The host opens additional toplevels with `App.openWindow`. Each window has its
own surface and frame generation, `App.eventWindowId()` says which window
produced an event, and `App.presentWindow(id, frame)` routes a frame to that
window. `WindowSet` keeps the logical side of the same picture:
 open, close, focus, resize, hide, and
suspend windows, with a per-window frame counter and one routed focus. `App`
itself can `suspend` and `resume`: the frame clock pauses, drawing stops, input
is ignored, and resume restores the previous animation request. A compositor
that reports the xdg-toplevel suspended state drives the same path through
events 23 and 24.

The app layer adds the pieces a whole application needs: `Router` for paths
with a back stack, `AppConfig` plus `Appearance` for size and light/dark/system
theming, `Notifications` for an in-app queue, `AppFiles` for caller-path file
reads and writes, `Worker.of(Thread.spawn(fn))` for background work, and
`AppBundle` for an `app.json` packaging manifest.

`ViewRouter` is the declarative route surface:

```rnx
let r = new ViewRouter()
r.route("/", (p: Map<String, String>): View => Home(p))
r.route("/task/:id", (p: Map<String, String>): View => Task(p))
r.route("/docs/*", (p: Map<String, String>): View => Docs(p))
r.navigate("/task/7")
let page = r.view()
```

Exact routes beat `:param` routes, which beat a final `*` wildcard. Query
parameters merge into the same map. `navigate` pushes history, `replace` does
not, and `back` restores the previous view. `resolve(path)` matches without
navigating. A route table like this is the visual acceptance case for
navigation.

`Driver.on(app)` is the test driver over that projection. Find a node by role
and name, then `press`, `click`, `increment`, `setValue`, or `focus` it; each
action rebuilds the frame so the next lookup is current. `Mcp.on(driver)` maps
the same operations to `ui.list`, `ui.press`, `ui.increment`, `ui.setValue`, and
`ui.screenshot` with JSON results. The MCP projection is opt-in; the app owns the
transport.

`App.screenshot(path)` renders the current frame on the CPU and writes a PNG, and
`App.screenshotBytes()` returns the same bytes. `Driver.screenshot(path)` does it
after an action, and the MCP `ui.screenshot` tool takes the path as its value.
No compositor screenshot is involved: tests and agents can drive the app and
capture the exact frame they just built. `Surface.encodePng()` and
`Surface.writePng(path)` expose the encoder directly.

For automation without a window, set `RNX_UI_SCRIPT` to a semicolon-separated
script. `App.show()` runs it headless and returns its status:

```text
press:button:Add;wait:2;assert:button:Add
set:textbox:name:hello;assertValue:textbox:name:hello
key:textbox:name:enter;assertValue:textbox:name:hello
scroll:list:Results:120;shot:/tmp/result.png
```

Supported steps are `press`, `click`, `focus`, `set`, `inc`, `key`, `scroll`,
`wait`, `shot`, `stats`, `assert`, `assertValue`, `assertChecked`,
`assertSelected`, `assertExpanded`, `assertBusy`, `assertDescription`,
`assertRange`, and `assertFocus`. Each step prints `ui.step <n> <step> ok|fail`;
the run ends with `ui.result ok` or `ui.result fail <step>`. The `stats` step
prints the last frame's node, record, byte, and dropped counts.

The `rnx mcp` server exposes the same transport as `ui_drive`, with `path`,
`steps`, and an optional `timeout_secs`. An MCP client can launch any app,
drive its real controls through the accessibility projection, assert state, and
capture a screenshot without a compositor. When the script contains a `shot:`
step, `ui_drive` returns the PNG as an inline image content block, so the model
sees the frame without a separate file read. Pass `golden` with a baseline PNG
to compare pixels; `golden_tolerance` sets the allowed per-channel delta and
`golden_max_diff_ratio` sets the fraction of pixels that may differ. The result
reports dimensions, differing pixels, the maximum delta, and the ratio.

`RNX_INSPECT=1` draws a node inspector over the frame: one accent outline per
live node, a `role name` label, and a stronger highlight for the selected node.
The overlay is painted after the normal display list, so screenshots and AI
runs can see structure and bounds. `inspect:on`/`inspect:off` are script steps,
and `Driver.inspect` toggles it from tests.

Every frame records build, layout, and paint time:
`Frame.timeline()` returns `build=<us> layout=<us> paint=<us> total=<us>`, and
`Frame.stats()` appends it after the node and record counts. `Driver.timeline()`
and the `timeline` script step expose the same line. The timings use the same
monotonic clock as the frame clock, so a fixed-frame test stays deterministic
in shape without asserting exact microseconds.

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
- `App.measureText(text, size)` returns shaped width through parley;
  `measureStyledText(text, size, weight, italic, tracking)` includes the style.
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
    let app = App.launch("rnx window", 520, 360, view)
    return app.show()
}
```

`App.showFor(3)` presents three frames and exits, which is enough to check the
path without a person at the keyboard.

## Platform support

The compiler and CLI install on Linux, macOS, and Windows. Native UI apps are
Linux/Wayland only: the window host, accessibility bridge, audio output, camera
capture, and file picker are Linux backends. Other platforms keep the language,
compiler, and headless image and scene paths, but there is no window host or
accessibility bridge yet. `rnx build` links native Linux executables.

The accessibility packet is platform neutral. AccessKit consumes it on Linux;
Windows UIA and macOS NSAccessibility need native window hosts before they can
expose the same roles, names, values, bounds, actions, relations, and live
announcements. Headless media uses the stubs described above on any host, but
the native media wrapper itself is Linux-only.

The approved order for other platforms is: complete the sysroot
implementation, then the in-house linker for the other operating systems and
CPU architectures, then the Windows/macOS window hosts and their accessibility
and media backends.

## Requirements

The renderer is a native wrapper, so `rnx run` needs the wrapper libraries.
`compiler/stdlib/src/app/tools/ci-wrappers.sh` builds all four, installs them
into a prefix, and runs the probes and a smoke app on Cranelift and LLVM.
The interpreter's foreign-call path accepts `Int`, `Bool`, and `Pointer` but not
`Float`, so use a compiled backend for an app that calls the runtime.
