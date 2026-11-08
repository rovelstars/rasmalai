---
title: "Declarative Views"
description: "The View value API: Page, List, Text, controls, keys, and App.launchView."
section: "Memory, Systems, and Concurrency"
icon: "Boxes"
---

# Declarative Views

`@std/ui` now has a value API. App code returns a `View`, not a retained
`Node`, and `@std/app` mounts it into the node arena.

```rnx
import { Button, Header, List, ListRow, Page, Spacer, Text, TextField, Toolbar, View } from "@std/ui"

fn TaskScreen(state: Tasks): View {
    return Page(title: "Today", padding: 20, gap: 12) {
        Header {
            Text(state.openCount().toString() + " open")
            Spacer()
            Button("Clear", () => state.clear())
        }
        List {
            for task in state.items {
                TaskRow(task, state)
            }
        }
        Toolbar {
            TextField(state.draft, "New task", (v: String) => { state.draft = v })
            Button("Add", () => state.add())
        }
    }
}
```

Start the app with `launchView`:

```rnx
fn Main(): Int {
    let state = new Tasks()
    let app = App.launchView("Today", 480, 360, () => TaskScreen(state))
    return app.show()
}
```

## What is available

Containers: `Page`, `Header`, `Section`, `Card`, `Toolbar`, `List`,
`ListRow`, `Panel`, `Stack`, `Grid`, `Scroll`, `Overlay`.

Text: `Text`, `Heading`, `DisplayText`.

Controls: `Button`, `Checkbox`, `Slider`, `TextField`, `Divider`, `Progress`.

Layout: `Spacer`, `Place`, `Inset`, `Named`, `View.onMount`.

Drawing: `Scene`.

There is no public `Column` or `Row`. Use `Header`, `Toolbar`, `ListRow`, or
`Stack(horizontal: true)` for horizontal composition; use `Page`, `List`,
`Section`, `Card`, or `Stack` for vertical composition.

## Blocks

The examples use collection blocks from
[Collection Blocks](/manual/21-collection-blocks). A block after `Page` fills
`children`; slots fill named array parameters.

## Keys

Use `keyed` when rows can reorder:

```rnx
fn TaskRow(task: Task, state: Tasks): View {
    return ListRow(padding: 10) {
        Checkbox(task.title, task.done, () => state.toggle(task.id))
        Spacer()
        Text(task.status())
    }.keyed(task.id.toString())
}
```

The retained tree reuses the keyed node, so focus, scroll, and hover survive
reordering.

## Typography

`DisplayText` uses the `display` text role, 56px by default:

```rnx
DisplayText(state.label(), align: Align.end, width: 312)
```

Text views carry a single style. Set `fontWeight` (100 to 900), `italic`,
`lineHeight` (a multiplier, `0` for normal), and `letterSpacing` (logical px)
before mounting:

```rnx
let title = Text("Quarterly report", align: Align.start)
title.fontWeight = 700
let body = Text("Wrapped paragraph", align: Align.start)
body.lineHeight = 1.6
body.letterSpacing = 0.5
```

The shaper measures and breaks styled text, not only plain text. `textRun`
carries the style lanes so the CPU and GPU renderers shape the same run. A
display-list record with weight 400, no italic, and zero tracking is the
default and stays compatible with older frames. A visual check covers styled
runs and the plain default on both renderers.

Inline spans shape one paragraph with mixed styles. Offsets count characters,
the same way `String.slice` does, and a later span wins on overlap:

```rnx
let spans = [
    new TextSpan(0, 4, 700),
    new TextSpan(5, 11, 400, true),
    new TextSpan(12, 17, 400, false, 0.0, theme.accent),
    new TextSpan(18, 26, 400, false, 2.0),
    new TextSpan(26, 31, 400, false, 0.0, -1, FontFamily.serif),
    new TextSpan(32, 41, 400, false, 0.0, -1, FontFamily.sans, TextDecoration.underline),
]
RichText("Bold italic color tracked serif underline", spans, align: Align.start)
```

Parley shapes the full paragraph with ranged properties, so wrapping and kerning
work across span boundaries. Each shaped run becomes one text record with its
own color, weight, italic, and tracking. Uncovered text keeps the role color
and base style. A visual check covers mixed spans across wrapping and kerning
boundaries.

## Named fonts

`FontRegistry` assigns ids from 100 and caps at 16 faces and 8 MB. A span
carries the face, and the frame packs its id into the text record. Font bytes
register with the renderer once, not once per frame:

```rnx
import { App, registerFont } from "@std/app"
import { Fonts, FontHash, FontSource } from "@std/app/fonts"
import { FontRegistry, TextSpan } from "@std/ui"

let registry = new FontRegistry()
let face = registry.register("Inter", fs.readBytes("assets/Inter.ttf").unwrap())
let app = App.launchView("Fonts", 800, 500, () => FontDemo(face))
registerFont(face)
```

`@std/app/fonts` adds a content-addressed cache and provider-neutral sources.
A `FontSource` names a URL, an optional SHA-256, a license, and a size cap:

```rnx
let fonts = new Fonts(registry, ".rnx-cache/fonts")
let source = new FontSource("local", "Inter", 400, false, path, digest, "OFL-1.1")
let face = fonts.loadSource(source, (url) => fs.readBytes(url).unwrap())
```

Pinned sources are stored as `<hash>.ttf` and verified on every load. Unpinned
sources record the first observed digest in a `.sha256` sidecar and verify
against it later. Cache misses without a fetcher return null and a warning, so
offline builds use a pre-seeded cache instead of reaching the network.
`@std/app/fonts/google` turns the CSS reply from the Google Fonts API into
sources; the app supplies the HTTP fetcher, so the font package needs no
network permission of its own. The acceptance case covers a registered face, a
cached source, and a miss without a fetcher.

## Links and rich editing

A span with a `link` URL is a link. Links paint in the theme accent with an
underline unless the span sets its own color or decoration:

```rnx
let spans = [
    new TextSpan(9, 13, 500, false, 0.0, -1, FontFamily.sans, TextDecoration.none, null, "https://example.test/docs"),
]
RichText("Read the docs.", spans)
```

`Tree.linkAt(x, y)` returns the URL under a point, and `Tree.linkRects(id)`
returns one bounds rect per link. Hovering a link sets cursor shape 2, hovering
a text box sets 1, and everything else is 0. A click calls `App.onLink(url)`
once; it does not also activate the node behind the text. The accessibility
tree exposes each link as a child node with role `link`, the link text as its
name, and the URL as value and description. `Driver.openLink(name)` and MCP
`ui.openLink` drive the same handler.

`RichTextField` keeps span ranges while text changes. The handler receives the
new value and the adjusted spans, so the app can store both:

```rnx
let editor = RichTextField(state.value, state.spans, (value, spans) => {
    state.value = value
    state.spans = spans
}, "Type here")
```

Typing inside or at the end of a span extends it; typing at its start inserts
before it. Replacing a selection gives the new text the caret style, and
deleting a prefix shifts or removes spans. The acceptance case covers links
under a pointer, `onLink`, and the accessibility children.

## Screen-space overlays

`Overlay` marks a subtree as a portal. `Place` and `Inset` set absolute world
or screen coordinates.

```rnx
let overlay = Overlay {
    Hud(state)
    if state.sheetOpen {
        SheetView(state)
    }
}
return Page(padding: 0, gap: 0) {
    World(state)
    Inset(overlay, 0, 0, 0, 0)
}
```

Routes can nest. A parent build receives a `RouteMatch` and calls `outlet()` to
mount the child that matches the remaining path:

```rnx
let root = new Route("/", (m: RouteMatch): View => {
    return Page(title: "Home", children: [m.outlet()])
}, [
    new Route("/tasks", (m: RouteMatch): View => TasksLayout(m), [
        new Route("/", (m: RouteMatch): View => TaskList(m), []),
        new Route("/:id", (m: RouteMatch): View => TaskDetail(m), []),
    ]),
    new Route("/*", (m: RouteMatch): View => NotFound(m), []),
])
let router = ViewRouter.fromRoute(root)
router.navigate("/tasks/7")
```

Parent and child params merge; a child key shadows the parent. Exact beats
`:param` beats `*` at each level. `withTransition` keeps the outgoing view
mounted under its old generation key while it fades out, and fades the incoming
view in over it through `Crossfade`. The two screens share one origin and the
taller child sets the box. The acceptance case nests a root frame around a
tasks frame around a detail and captures one deterministic mid-fade frame.

The spatial acceptance case uses a camera-transformed 6000x4000 world,
absolute floating cards, a HUD, a sheet, and a detail overlay. `View.onMount`
creates the `Stage`, wires `GestureArena`, presents or dismisses the `Sheet`
spring, and keeps the content inert with `Stage.sync()` after each rebuild.
`Overview` and `WindowMorph` cover the overview fit and matched geometry.

For immediate-mode drawing, `Canvas` exposes paths as well as rects and text:

```rnx
let p = new Path()
p.moveTo(20.0, 80.0)
p.cubicTo(60.0, 0.0, 120.0, 160.0, 180.0, 80.0)
c.pathStroke(p, theme.text, 4.0, StrokeCap.round, StrokeJoin.round)

c.pushLayer(0.55)
c.roundRect(theme.accent, 200.0, 60.0, 80.0, 80.0, 40.0)
c.popLayer()
```

Paths use the v3 display-list records, and group opacity is one layer record.
Multi-stop linear and radial fills use kind 25:

```rnx
let stops = [
    new GradientStop(0.0, rgba(80, 140, 255, 255)),
    new GradientStop(0.55, rgba(230, 90, 220, 255)),
    new GradientStop(1.0, rgba(255, 150, 60, 255)),
]
c.linearGradient(LinearGradient.vertical(stops), 20.0, 20.0, 300.0, 120.0)
c.radialGradient(new RadialGradient(stops, 0.0), 340.0, 20.0, 300.0, 120.0)
```

Stops are sorted and clamped to 0..1. A radial radius of 0 means
`min(width, height) / 2`. The paint acceptance case covers paths, layered
opacity, and both gradient kinds.

A view can blur or mask its whole subtree. `blurSigma` emits a Gaussian blur
layer; `maskRadius` emits a rounded-rect alpha mask, so pixels outside the
rounded shape are cut rather than clipped:

```rnx
let card = Card(padding: 0) {
    Scene("sharp", 720, 140, painter)
}
card.blurSigma = 6.0

let cut = Card(padding: 0) {
    Scene("masked", 720, 140, painter)
}
cut.maskRadius = 24.0
```

Both use full paint for the frame. The CPU renderer applies a Gaussian blur and
the rounded alpha mask. The GPU path renders each blur group to an offscreen
texture, runs separable Gaussian passes on wgpu, and draws the result back at
the group's z-order; nested groups recurse. The compositing acceptance case
shows both effects next to the opacity and blend demos.

Motion uses the same value API. `Spring`, `Curve`, `Decay`, and `Motion` share a
clock and never read global time inside `step`:

```rnx
let spring = Motion.ofSpring(Spring.fromResponse(0.45, 0.55, 80.0))
spring.to(600.0)
spring.step(clock.dt())
let x = spring.value()
```

The motion acceptance case shows the spring, a curve, a friction decay, and a
keyframe track moving under one frame clock. Retargeting preserves velocity; a
fling restarts decay from the present position.

Keyframes are a pure track:

```rnx
let track = new KeyframeTrack([
    new Keyframe(0.0, 40.0),
    new Keyframe(0.5, 340.0),
    new Keyframe(1.0, 640.0, Curve.easeOut()),
], true)
let x = track.valueAt(clock.elapsed())
```

Frames sort by time, the outgoing frame's curve shapes each segment, and a
looping track wraps at `duration()`. Sampling the same time always returns the
same value; the track owns no clock.

Reactive state uses an explicit graph. There is no process-global observer:

```rnx
let rx = new Reactive()
let count = rx.signal(0)
let doubled = rx.computed<Int>(() => count.get() * 2)

rx.effect(() => {
    print("count", count.get())
})

count.set(2)
rx.drain()
let v = doubled.get()
```

`Signal.set` schedules dependent effects instead of running them synchronously.
`drain` runs them once in stable id order. `Computed.get` is lazy and recomputes
only when a dependency version changed. `batch` coalesces a group of writes.
Each graph is independent; tests and multiple app surfaces do not share
dependencies.

A component is a keyed subtree boundary. The builder runs inside the reactive
scope, so signals read there subscribe that component and nothing else:

```rnx
let rx = new Reactive()
let left = rx.signal(0)
let right = rx.signal(0)

let panels = Grid(columns: 2, gap: 12) {
    rx.component("left", () => Text("left " + left.get().toString()))
    rx.component("right", () => Text("right " + right.get().toString()))
}
```

Writing `left` dirties only the `left` component. A clean component returns its
cached `View` and does not run the builder. Call `app.bindReactive(rx)` so the
app drains queued effects at the start of a frame and keeps requesting frames
while a component is dirty or an effect is queued. An empty key mounts the
builder with no caching. The state acceptance case shows two sibling panels
whose rebuild counters move independently.

Views composite as groups:

```rnx
Card()
    .opacity(0.6)
    .blend(BlendMode.screen)
```

Group opacity applies once to the composited subtree, not to each child, so
overlapping children do not darken twice. Blend modes apply the group to the
backdrop. Frames containing composited views use full paint instead of partial
damage, which keeps the backdrop correct. The compositing acceptance case
covers group opacity, blend modes, and the full-paint fallback.

Shadows and rounded clips are view attributes:

```rnx
Card()
    .shadow(rgba(0, 0, 0, 170), 0.0, 10.0, 12.0, 12.0)

Scene("art", 560, 180, paint).clipRadius = 22.0
```

A shadow is a Gaussian blurred silhouette drawn before the node. A rounded clip
clips the node and its children. Both use full paint until damage inflation is
implemented. The paint acceptance case covers a shadow, a rounded clip, and the
full-paint fallback.

Dragging is tracked per pointer id. The Tree exposes `dragNode()`, `dragPhase()`,
`dragDx()`, `dragDy()`, `dragVelocityX()`, and `dragVelocityY()`. The phase moves
from none to began after 4 px of slop, then changed, ended, or cancelled.
`Driver.drag(role, name, dx, dy, steps)` and the `drag:role:name:dx:dy` script
step drive the same path for tests. The gesture acceptance case covers the drag
phases, the slop threshold, and velocity.

A pointer that stays within 4 px for 500 ms sets `longPressed()` once.
Two taps within 300 ms and 30 px set `tapCount() == 2`. With two pointers down,
`pinchScale()` and `pinchRotation()` report the distance and angle relative to
the initial pair; lifting either pointer resets them. `Driver.longPress`,
`Driver.doubleTap`, and `Driver.pinch` drive the same paths, and the script
steps are `longPress:role:name`, `doubleTap:role:name`, and
`pinch:role:name:scale`. The gesture acceptance case shows the press, tap-count,
and pinch state after each driver call.

Drag and drop carries a typed payload:

```rnx
let card = Card(title: "drag me")
card.draggable("text", "hello drop")

let zone = Card(title: "drop text here")
zone.dropTarget("text", (d: DragData) => {
    state.last = d.kind + " " + d.text
})
```

The payload starts only after the 4 px drag slop. A drop target accepts a
comma-separated kind list or `"*"`; the nearest accepting node on the hit path
wins. `Driver.dragTo` and the `drop:sourceRole:sourceName:targetRole:targetName`
script step drive the same path. The drag-and-drop acceptance case shows an
accepted text drop and a rejected file zone.

Scroll nodes use the same physics core. `scrollFling` drives a `Decay` toward
the projected offset, overscroll uses `rubberBand`, and the viewport springs
back to the bound. `Driver.fling` and the `fling:role:name:velocity` script step
start it. The scroll acceptance case covers a fling, overscroll, and the spring
back to the bound.

Declarative presence uses a keyed fade:

```rnx
Presence(state.show, "panel", Transition.normal()) {
    Card(...)
}
```

A keyed presence stays mounted while it fades out, then removes its children
once the leave transition finishes. It fades back in on the next show. The
presence key survives view rebuilds, and the app keeps requesting frames while
the transition is active. The transition acceptance case covers a keyed leave, a
removal after the leave finishes, and the fade back in.

The calculator acceptance case follows the phone-calculator layout: a utility
row with square root, pi, Euler's number, and history, a right-aligned
expression with the running history above it, and a five-row keypad of circular
buttons with an accent equals key. `Button(round:, accent:, gradient:, textSize:)` exposes those
styles without breaking the role-based accessibility projection. `gradient`
emits one linear-gradient record, so the fill is antialiased instead of a stack
of clipped bands. `aspectRatio`
keeps the keys square so they stay circular at any window width, and `maxWidth`
plus `center` cap and centre the keypad on a desktop-sized window. The
accessibility driver can press `12 + 7 × 3 =` and read back the expression,
history, result, and projection flags.

## Current limits

- `View` is a class per node in this first slice. The retained arena is still
  the runtime representation.
- `Stage`, `Sheet`, `Overview`, `GestureArena`, and `WindowMorph` are runtime
  classes rather than first-class `View` components. The spatial acceptance
  case wires them through state and `View.onMount`.
- Collection blocks cover free functions, static factories, instance methods on
  typed locals, and `this.method`. Generic array parameters are not supported
  yet.
- Arbitrary per-node font sizes are not exposed; use `DisplayText` for a
  display-sized readout.
