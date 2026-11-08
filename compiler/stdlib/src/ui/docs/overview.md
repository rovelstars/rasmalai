# The UI core

Nodes, layout, hit testing, focus, keyboard activation, and the semantic walk.
Headless: no renderer, no window, no bus.

The author-facing API is a `View` value. `Page`, `Header`, `List`, `Text`,
`Button`, and the other constructors build it; `View.mount` or
`App.launchView` mounts it into the retained tree. The builder and node arena
below remain the runtime surface and the escape hatch for custom primitives.

This is M0 from `requirements_UI/49-m0-contract.md`. The parts of the contract
that exist here are the arena, the builder, layout, input dispatch, focus, and
the semantic tree, and positional plus keyed reconciliation. The packet encoding
comes next.

## How it is built

**Parallel arrays.** `Tree` keeps one array per node property and addresses nodes
by integer id. A subtree walk is index arithmetic, a snapshot is a copy, and
there is no per-node allocation. Child counts are stored when a node is attached,
so `childCount` is O(1) instead of a sibling walk.

**Wire ids for roles and actions.** The arena stores `Int`, not the enum. An
enum-typed array in a class field currently trips the prelude interface check, and
an integer lane is smaller anyway. The enums stay in method signatures, which is
all an author sees.

**Plain `fn` for the package surface.** `export fn` is the C-ABI surface and is
validated as such. Ordinary package-visible helpers use plain `fn`, which is
importable across files and packages. The value constructors are plain `fn`.

**A value, then mount.** App code returns a `View`; `View.mount` creates or
reuses the retained nodes and applies layout. The builder (`Build`, `Node`) is
still the runtime and escape-hatch API. Collection blocks lower a trailing block
to the callee's final array parameter, so `Page { Text("a") }` is
`Page(children: [Text("a")])`. The legacy child-sink block form remains for
builder calls.

**Two layout passes.** `measure` walks down with the width a node receives and
back up with the height it needs. `place` walks down once more and writes
rectangles. No allocation per pass, no layout tree.

## Try it

```sh
export RNX_VENDOR_DIR=$PWD/compiler/stdlib/src
cd compiler/stdlib/src/ui && rnx test
```

## Layout styles

Styles are per node and default to the role values, so a screen that sets none
keeps the geometry it had.

- `padding(n)` replaces the role inset; `margin(n)` adds space outside the box.
- `gap(n)` replaces the role gap between children.
- `grow(f)` takes a share of the free main-axis space in a container with a
  fixed size or a height budget.
- `alignMain(a)` places the child stack at the start, center, or end.
- `alignCross(a)` places children across the axis. `stretch` fills the cross
  size: a row stretches height, a column stretches width. `baseline` lines up
  the first text baselines in a row.
- `alignSelf(a)` overrides the parent's cross-axis alignment for one child.
- `order(n)` sorts siblings; lower comes first, and equal values keep tree
  order.
- `minWidth`, `maxWidth`, `minHeight`, and `maxHeight` clamp the measured box.
- `Constraints` is an explicit parent-supplied box: min, max, and ideal for
  both axes. `node.constrain(c)` applies it to one child, and
  `Tree.layoutConstrained(root, width, height, c)` applies it to the root
  viewport. `Constraints.tight`, `loose`, and `unbounded` build the common
  shapes.
- `idealWidth` and `idealHeight` give a preferred size: width wins over the
  intrinsic size, and height pads short content while taller content still wins.
  Min and max still clamp both. In a flex row, a child that hits `maxWidth` hands
  the leftover to the last grow child; `minWidth` survives overflow shrink.
- `minContent()`, `maxContent()`, and `fitContent()` choose the width from the
  content instead of the parent slot.
- `aspectRatio(r)` sets height from width when no height is given.
- `gapPercent(p)` sets the gap from the container's content width; `autoMargin()`
  turns free main-axis space into equal margin on that child, so it can center
  or push to an edge.
- `zIndex(n)` orders siblings for paint and hit testing; higher paints later and
  wins hits. Ties fall back to `order` and then tree order.
- `b.image(png, w, h)` is a bitmap control. An empty buffer paints a placeholder.

## RTL, safe areas, type scale, and density

`rtl()` flips row order and mirrors start/end text alignment; columns keep their
top-to-bottom order. `setSafeArea(top, right, bottom, left)` and
`setKeyboardInset(bottom)` shrink and offset the root, so content never sits
under the system bars or the soft keyboard.

`WritingMode.vertical` stacks text glyphs down the block axis: the node becomes
one glyph-line wide, each grapheme paints on its own baseline, and its height is
the sum of glyph advances. It currently applies to display text, not textbox
editing.

The measure pass caches each node's measured box by node, available size, exact
flag, and a layout version that every sizing setter bumps. A repeated layout
with no changes restores the cached boxes and reports a hit; any size, padding,
gap, text, or writing-mode change invalidates the cache.

`setTextScale(f)` multiplies every text role for dynamic type: measurement,
wrapping, height, and painting all read the scaled size from one place.
`setDensity(d)` records the device pixel ratio; layout stays in logical units
and `dp(n)` converts to device pixels. The app forwards density to the frame
packet scale, so a 2x display is tagged at 2000.
- `absolute()` takes a child out of the flow; `left`, `top`, `right`, `bottom`,
  and `inset` pin it. Two opposite edges set the size between them.

A row uses equal slots until a child has a fixed width or `grow`. Then fixed
children keep their width and grow children share the rest. A container reports
the width its children need, so a row can size a column to its content.
`Tree.layoutIn(root, width, height)` adds a height budget; `Tree.layout(root,
width)` keeps the content height. The app uses `layoutIn` with the window size,
so the root fills the window.

## Stack, scroll, and wrap

`b.stack()` overlays its children at the same origin. Later children paint and
hit on top. `b.spacer()` is an empty stack with `grow(1.0)`, for pushing a row
apart or holding a column open. A child can also leave the flow with
`absolute()` and pin itself with `left`, `top`, `right`, and `bottom`. Two
opposite edges set the size between them.

`b.grid(columns)` places children in fixed columns. Cells share the width after
gaps; a row is as tall as its tallest cell.

`b.divider()` is a one-pixel rule, `b.progress(v)` is a four-pixel bar with a
value in 0..1, and `b.panel()` is a padded surface without a heading. A
`b.segmented(labels, selected, onSelect)` is a row of radios that reports the
chosen index. `b.searchField` is a text field with a search placeholder.

`b.icon(name, size)` draws one of `plus`, `close`, `check`, or `chevron`.
`b.tabs(labels, selected, onSelect)` is a tab strip; the selected tab is
checked and underlined. `b.menuBar(labels, onSelect)` is a horizontal menu bar
with one action per item and a `menubar`/`menuitem` role pair.
`b.menu(labels, onSelect)` is a vertical list of actions. `b.popover(width)` is
an absolute panel; set its edges with `left`, `top`, `right`, or `bottom`.
`b.toast(message)` pins a panel to the bottom right of its parent. Call
`node.tooltip(text)` to paint a label while the node is hovered. `b.wrap()` is the flowing variant
when the column count should follow the width.

`b.scroll()` is a clipped viewport with its own offset. Give it a height through
`fixedHeight` or `grow`, then add children as a column. The wheel, `scrollAt`,
and `Node.scrollTo` move it; the content height sets the maximum. The offset
stays on the keyed node, so a rebuild does not jump to the top.

`b.wrap()` flows children into lines at the available width, with `gap` between
children and lines. It measures each child at its intrinsic width, so a row of
chips or tags wraps instead of overflowing. Nested scrolls chain: when the
inner viewport reaches its edge, the next wheel step goes to its scrollable
ancestor.

## Input

`pointerDown(x, y)` hit-tests the last layout and runs the node's action.
`pointerMove` tracks hover and drags a slider. Hover styling includes the
hovered node's ancestors, so a card keeps its hover surface while the pointer is
over its text. A move only reports a change when the visual hover key changes:
the nearest hover-painted ancestor, or the exact node when it has a tooltip.
Moving between two text children of one card therefore does not request a frame.
`pointerUp` clears the pressed
state. `keyDown(node, key, shift)` maps Enter and Space to the primary action,
Escape to a dialog's dismiss action, and Left and Right to the next interactive
sibling, which is how a radio group moves and selects. `disabled(true)` removes
a node from focus, hover activation, and dispatch. `pointerLeave` cancels hover
and an active drag, so dragging a slider out of the window stops at the last
inside value instead of jumping to an edge. The focus ring is painted only for
keyboard or programmatic focus; clicking a field or slider moves focus without
showing the ring. `focusNext` and `focusPrev`
walk only interactive nodes and wrap. `focusScope(dialog)` holds traversal inside a subtree
while it is open and remembers the node that had focus; `clearFocusScope`
restores that node when the scope is released.

`Shortcuts.pushScope` and `bindScoped` add a modal layer: a scoped chord
shadows the global binding with the same keys until `popScope`. `selectedText`
and `replaceSelection` expose a text box's selection for the app clipboard.

All of them end in the same handler. A test that clicks, one that presses Enter,
and one that calls the action directly exercise one code path.

## Semantics

Each node carries a role, a name, an action mask, bounds, a value string, and a
checked flag. `snapshot()` renders one line per node in tree order, which is what
the golden tests compare and what the inspector shows. `unnamedInteractive()`
returns the first interactive node without a name, so the audit fails in a test
instead of in review.

## Components

`Build` creates `column`, `row`, `section`, `text`, `heading`, `button`,
`slider`, `sliderValue`, `toggle`, `checkbox`, `radio`, `textField`, `dialog`,
## Spatial stage

The stage is an optional layer over the same tree. Layout stays logical; the
stage changes presentation.

- `Camera(scale, x, y)` is a scene transform. `screenX`/`worldX` convert both
  ways, `zoomAt` keeps the screen anchor fixed, and `panBy` moves the origin.
  `Painter.paintCamera` wraps the tree in one transform record and
  `Tree.hitTestCamera` applies the inverse, so hit testing agrees with pixels.
  `pointerDownCamera`, `pointerMoveCamera`, and `scrollAtCamera` route input
  through that inverse. Portal overlays hit in screen coordinates before the
  inverse, so a dock or sheet stays fixed while the world moves.
- `Node.portal(layer)` keeps a node in its logical parent but paints it above
  the tree, clipped to the layer. It also wins hit testing, because the visual
  parent is the layer. A passive overlay whose portalled subtree has no
  actionable or scrollable node under the pointer passes the hit through to the
  world. A portal that is not attached to the laid-out tree is neither painted
  nor hit tested.
- `Spring(stiffness, damping, value)` integrates a damped spring in substeps.
  `retarget` and `interrupt` keep the current velocity, so an interrupted
  animation continues from where it was.
- `Sheet.of(detents, index)` is a detented surface. `dragBy` clamps to the
  detent range, `release(velocity)` settles to the nearest detent or moves one
  step on a fast fling, and `step(dt)` advances the spring.
- `Match(from, to)` interpolates a rectangle with a spring; `open`, `close`,
  `progress`, `x/y/w/h`, and `done` drive matched geometry.
- `Overview(windowW, windowH, padding)` fits a set of window rectangles into
  the viewport: `add`, `enter`, `exit`, and `fit` drive the camera; `screenX/Y/W/H`
  map a card through it, and `fits` checks the padded viewport. `exit` restores
  the camera in place, so an app can hold one camera object for the whole life
  of the stage.
- `WindowMorph(from)` interpolates a window rectangle into an overview
  thumbnail and back with matched geometry; `swapped` flips at the halfway
  frame so the real content changes exactly once.
- `Ambient` is the ambient-layer lifecycle. `advance(dt)` returns the full
  delta when live, a quarter when covered, and zero when frozen or hidden.

### Gestures and stage focus

`GestureArena` resolves a gesture start. Each claim has an `axis`, a `priority`,
and a `depth`. `begin` picks the highest priority, breaks ties by the deeper
node, then by later registration. A winning claim is sticky: `begin` on another
axis returns the active node until `release`, which is the rule that stops a
sheet from stealing a scroll mid-flick. `lastDecision` exposes the choice in dev
builds.

`Node.inert()` leaves a subtree out of focus traversal, hit testing, and
interactive semantics. `Stage.of(tree, content, overlay, ambient, sheet)` ties
the stage rules together: `present(first)` marks the content inert, raises the
sheet, throttles the ambient layer, scopes focus to the overlay, and focuses the
first control. `dismiss` reverses all four and restores the previous focus.
Node flags reset on every rebuild, so the `onMount` callback calls `sync()`
while the stage is open to re-apply the inert content root.
`occludes` reports the sheet state for backdrop policy. The accessibility
projection carries an `inert` flag so the platform adapter hides the background.

`FramePacket` encodes every live node's id, parent, role, flags, bounds, action
mask, name, and value into one buffer: a 48-byte header (`424242`, version,
count, stride, width, height, scale, color, flags, string base, reserved) and a
64-byte record per node with a UTF-8 string table. The header carries the
output scale in thousandths (1000 is 1x, 2000 is 2x) and the color-space wire id
(0 sRGB, 1 linear, 2 Display P3), so a host can tag the buffer without guessing.
`FramePacket.decode` is the read side; `FrameNode` is a struct, and the packet
keeps parallel arrays so decoding does not allocate a class array of nodes.

`list`, `listItem`, `virtualList`, `canvas`, `stack`, `scroll`, `wrap`,
`grid`, `spacer`, `divider`, `progress`, `panel`, `searchField`, `segmented`,
`icon`, `image`, `tabs`, `menuBar`, `menu`, `popover`, and `toast`. Factories create nodes; the view attaches them with `addChild`,
or with a trailing block, which compiles to the same calls. An explicit
`parent.addChild(child)` inside a block attaches the child and does not also
attach the parent to the enclosing block.

## Not here yet

- The frame packet. Layout, layout bounds, and the semantic tree are separate
  views in memory rather than one encoded buffer.
- A renderer. `@std/ui` stays headless; `@std/app` owns the window and the
  renderer.

## Regression checks

The `test fn` blocks cover creation order, deepest-node hit testing, tap dispatch,
Enter reaching the same handler, focus wraparound, column stacking, root
protection, damage regions, and the unnamed-interactive audit. They use fixed
text metrics, so they check structure, not the shipped font layout.

## Display list v2

`DisplayList` encodes primitives into one little-endian buffer. A frame is a
32-byte header, an array of 64-byte records, and a blob table for UTF-8 text and
PNG images:

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
`f` as `f64`. `color` is `0xRRGGBBAA`. The kinds are:

| Kind | Meaning | Payload |
|---|---|---|
| 1 | fill | rect in `x`, `y`, `w`, `h` |
| 2 | text | `a`/`b` is a blob range; `x`/`y` is the text box top-left; `w` is the font size |
| 3 | clip push | rect in `x`, `y`, `w`, `h` |
| 4 | clip pop | none |
| 5 | rounded rect | rect in `x`, `y`, `w`, `h`; radius in `e` |
| 6 | image | `a`/`b` is a PNG blob range; target rect in `x`, `y`, `w`, `h` |
| 7 | transform push | `[a b c d e f]` across all six lanes |
| 8 | transform pop | none |
| 9 | polygon | `a` is the blob offset, `b` is the point count; each point is two `f64` values |

`finish(width, height)` returns the buffer. `Display.textOf(bytes)` decodes it
back to one line per record, which is what snapshots and the inspector use.
Unknown kinds are skipped by a reader, so the format can grow.

## Painting

`Painter.paint(tree, width, height, theme)` walks the arena in child order and
emits a background fill, one surface and outline per control, one text run per
label, and an accent outline for the focused node. `Theme.dark()` and
`Theme.light()` are data; a brand kit passes its own colors to the same call.

## Text layout

`@std/ui` never shapes text itself. A `TextEngine` supplies two operations:
`measure(text, size)` for one unwrapped line, and `rows(text, size, maxWidth,
wrap, maxLines, ellipsis)` for line breaking. `@std/app` installs a parley-backed
engine before the first frame.

`Tree` takes an engine in its constructor. There is no implicit default. The
app passes the parley engine; core tests pass `TextEngine.fixed()`, which
advances a fixed amount per character. That engine is a test double, not the
shaper the app ships.

This package's tests stay at the level React core tests operate on: the tree,
reconciliation, focus, dispatch, semantics, damage bookkeeping, and layout
structure with explicit or fixed metrics. They do not assert font-derived
sizes. Text layout with real metrics lives in `@std/app` tests, where parley,
the OS font, and the software renderer are available: wrapping, ellipsis,
alignment, button width, and eventually text editing.

Every text-bearing node is laid out once during the measure pass. The rows,
their heights, and their baselines are stored on the node and reused by the
painter, so a label is shaped once per frame, not once per pass.

- `text` and `heading` fill the width they are given and wrap by default.
- Buttons and the other controls are single-line and clip with `…`.
- `Node.textAlign(Align.center)` and `Align.end` move each row inside the
  content box.
- `Node.textWrap(false)` disables wrapping; `Node.textMaxLines(n)` caps rows.
- `Node.setTextEngine` on the tree, or `Build.setTextEngine`, swaps the engine.

Row text comes from parley's line ranges, so Unicode shaping, cluster
boundaries, and explicit newlines are handled by the shaper. Character offsets
cross the ABI, which matches `String.slice`.

## Damage

`Tree.beginFrame` snapshots each node's bounds, text, value, checked state, and
role. After layout, `Tree.damageOf(width, height, full)` returns the rectangles
that changed: moved or resized nodes, changed labels and states, canvas nodes
(their paint runs every frame), and the old and new focused node. Removed nodes
contribute their previous bounds.

`Damage.compact()` merges overlapping and touching rectangles, and collapses a
large set into one bounding box. The list is small by construction, so the
merge stays cheap.

Damage compares against the last presented frame, not against the start of the
current build. `Tree.commit()` snapshots the frame after it is painted; input
that arrives between frames mutates the tree after that snapshot, so the next
`damageOf` sees it. Without this, a slider drag or a typed character would
already be in the tree when `beginFrame` ran and would leave the old pixels on
screen. `App.frame` calls `commit` itself. A program that drives `Tree` directly
must call it after presenting. A removed node reports its last committed bounds
once, then stops.

`Painter.paintRegions(tree, width, height, theme, damage)` fills only the
damaged regions and paints only the nodes that intersect them. The app uses it
for the window path; `Painter.paint` still builds a full list for tests,
snapshots, and headless dumps. The first frame and a resize force full damage
because the target starts undefined.

`Painter.paintDamage(list, damage, color)` outlines the regions for the
show-updates overlay. Enable it with `App.setDamageOverlay(true)`; `RNX_TRACE=1`
prints the damage rectangles each frame.

## State and editing

Every node carries hover, pressed, and disabled state. `disabled(true)` dims the
control and removes it from focus and dispatch. `range(v)` sets a slider's
fraction; `sliderValue(label, v, onChange)` reports changes. Text fields keep a
caret and a selection and edit by grapheme cluster through the `TextEngine`:
`typeText`, `deleteBackward`, `deleteForward`, `selectAllText`, and the arrow,
Home, End, Backspace, and Delete keys.

`Shortcuts` is a small chord registry, normalized to lower case. `bind` returns
whether it replaced an existing chord, and `run` tells the caller whether the
chord was consumed. The app owns the registry; the UI core does not impose a
default set beyond what the app binds.

## Virtual lists

`b.virtualList(label, key, total, rowHeight, viewport)` creates a clipped list
node and returns a `VirtualRange` with `first`, `count`, and `offset`. The
caller builds only `count` rows and applies `row.offsetY(0.0 - v.offset)`, so a
scroll moves the rows by the exact pixel delta instead of jumping one row at a
time. The scroll position lives on the keyed list node, so it survives
rebuilds.
`scrollAt(x, y, dy)` scrolls the nearest scrollable ancestor under the point.
If the point is just outside the list, it falls back to the last list that
scrolled, then to any scrollable node containing the point, so a stale pointer
coordinate does not stop the wheel. Painting and hit testing respect clipping,
so offscreen rows cost nothing.
The reconciler falls back to the next unclaimed node of the same
role after the creation cursor; a list whose row count changes can be followed
by more nodes without appending a fresh tail every frame.

## Canvas nodes

`b.canvas(name, width, height, paint)` creates a fixed-size node that draws
itself. `paint` receives a `Canvas` clipped to the node rectangle and works in
local coordinates; the painter adds the node origin. The drawing calls are
`fill`, `rect`, `roundRect`, `polygon`, `line`, `text`, `clip`, `unclip`,
`pushTransform`, and `popTransform`. Each call appends to the frame's display
list, so a canvas is another producer of the same primitives, not a separate
renderer.

A canvas callback runs once per frame. Use it for charts, sprites, and
animation; use the built-in roles for text and controls.

## Animation

`FrameClock` is the time source. `App.frame` ticks it with `Clock.mono()` and
passes it to the view through `b.frameClock()`. `dt()` is the seconds since the
previous frame, `elapsed()` is the running total, and `frames()` counts ticks.

`b.animate(false)` tells the app the view is paused. `App` then draws once per
input event and sleeps instead of requesting continuous frames. The clock keeps
its baseline current while paused, so `dt()` is zero and animations freeze
without jumping on resume. `App.setAnimating` exposes the same flag outside a
view.

Physics belongs in the view, driven by `dt()`: integrate in fixed substeps so
the motion is the same at any refresh rate.
