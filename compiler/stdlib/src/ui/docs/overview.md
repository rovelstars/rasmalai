# The UI core

Nodes, layout, hit testing, focus, keyboard activation, and the semantic walk.
Headless: no renderer, no window, no bus.

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

**Classes for the public surface.** A `pub fn` with a non-C signature is rejected
unless the package declares a cabi file, so the core exports classes and enums
and keeps free helpers package-internal.

**A context, then explicit children.** Views receive a `Build`, create nodes
with its factories, and attach them with `addChild`. A node carries its tree, so
attachment needs no ambient state. The trailing-block syntax compiles to exactly
these calls, so the sugar adds no runtime concept.

**Two layout passes.** `measure` walks down with the width a node receives and
back up with the height it needs. `place` walks down once more and writes
rectangles. No allocation per pass, no layout tree.

## Try it

```sh
export RNX_VENDOR_DIR=$PWD/compiler/stdlib/src
cd compiler/stdlib/src/ui && rnx test
cd examples/ui-demo && rnx run --backend interpreter
```

## Input

`pointerDown(x, y)` hit-tests the last layout and runs the node's action.
`keyDown(node, key)` maps Enter and Space to the primary action, Escape to a
dialog's dismiss action, and Left and Right to the next interactive sibling, which
is how a radio group moves and selects. `focusNext` and `focusPrev` walk only
interactive nodes and wrap. `focusScope(dialog)` holds traversal inside a subtree
while it is open, and `clearFocusScope` releases it.

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
`slider`, `toggle`, `checkbox`, `radio`, `textField`, `dialog`, `list`, `item`,
and `canvas`. Factories create nodes; the view attaches them with `addChild`,
or with a trailing block, which compiles to the same calls.

## Not here yet

- Virtualization. A list lays out every child for now.
- The frame packet. Layout, layout bounds, and the semantic tree are separate
  views in memory rather than one encoded buffer.
- A renderer. The CPU rasterizer is the next milestone.

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

`Painter.paintRegions(tree, width, height, theme, damage)` fills only the
damaged regions and paints only the nodes that intersect them. The app uses it
for the window path; `Painter.paint` still builds a full list for tests,
snapshots, and headless dumps. The first frame and a resize force full damage
because the target starts undefined.

`Painter.paintDamage(list, damage, color)` outlines the regions for the
show-updates overlay. `RUNIX_DAMAGE=1` enables it and `RUNIX_TRACE=1` prints the
damage rectangles each frame.

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
