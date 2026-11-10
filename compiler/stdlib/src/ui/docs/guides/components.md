---
title: Components
description: Buttons, fields, progress, images, scenes, and when to reach for the builder.
section: Components
order: 40
---

# Components

The value API has a small set of controls. Each one takes its label or value as
the first argument and its callback last, so a form reads in the same order as
the data it edits.

```rnx
import { Button, ButtonTone, Checkbox, Divider, Page, Progress, Slider, Stack, TextField, View } from "@std/ui"

class Draft {
    let title: String = ""
    let published: Bool = false
    let cover: Float = 0.5
    fn save() {}
    fn remove() {}
}

fn Editor(state: Draft): View {
    return Page(title: "Editor", padding: 20, gap: 12) {
        TextField(state.title, "Title", (v: String) => { state.title = v })
        Checkbox("Published", state.published, () => { state.published = !state.published })
        Slider("Cover size", state.cover, (v: Float) => { state.cover = v })
        Progress(state.cover)
        Divider()
        Stack(horizontal: true, gap: 8) {
            Button("Save", () => state.save())
            Button("Delete", () => state.remove(), tone: ButtonTone.danger)
        }
    }
}
```

Callbacks receive the new value: `TextField` passes a `String`, `Slider` passes a
`Float`, and `Checkbox` passes nothing because the new state is yours to store.
`Progress` takes a fraction from 0 to 1. `Divider` is a one-pixel rule.

## Buttons

`Button(label, onTap)` is the default. The named options change the visual role,
not the callback:

```rnx
import { Button, ButtonSize, ButtonTone, Stack, View } from "@std/ui"

fn Actions(): View {
    return Stack(horizontal: true, gap: 8) {
        Button("Save", () => {}, tone: ButtonTone.accent)
        Button("Delete", () => {}, tone: ButtonTone.danger, size: ButtonSize.sm)
        Button("More", () => {}, tone: ButtonTone.ghost)
        Button("7", () => {}, round: true, accent: true, textSize: 22)
    }
}
```

`tone` is `neutral`, `accent`, `danger`, or `ghost`. `size` is `sm`, `md`, or
`lg`; the size resolves padding and height through the space scale. `round`
makes the control a pill or a circle when the width and height match. `accent`
selects the accent fill without changing the tone. `gradient` draws the fill as
one linear gradient record, which keeps the edges antialiased. `textSize` sets
the label size in logical pixels.

## Images and scenes

`Image` takes a `ByteBuffer` of PNG, JPEG, WebP, or SVG bytes. It reads the
intrinsic size from the header, so width and height default to the image size.
`fit` decides how the image fills the box:

```rnx
import { ByteBuffer } from "@std/bytes"
import { Canvas, Card, Image, ImageFit, Scene, View, rgba } from "@std/ui"

fn Artwork(png: ByteBuffer): View {
    return Card(title: "Artwork", padding: 12) {
        Image(png, fit: ImageFit.contain, width: 320, height: 180)
        Scene("swatch", 320, 80, (c: Canvas) => {
            c.fill(rgba(17, 19, 23, 255))
            c.roundRect(rgba(96, 147, 246, 255), 16.0, 16.0, 96.0, 48.0, 10.0)
        })
    }
}
```

The fit modes are `fill`, `contain`, `cover`, `scaleDown`, and `none`. `cover`
clips to the box; `none` draws at the intrinsic size.

`Scene` is a fixed-size node that draws itself. Its paint function runs once per
frame in the node's local coordinates. `Canvas` also has `rect`, `line`,
`polygon`, `text`, `pathFill`, `pathStroke`, `linearGradient`, `radialGradient`,
`clip`, `clipRound`, `pushLayer`, `pushLayerBlend`, and `pushTransform`. Annotate
the parameter as `(c: Canvas)` so the method calls resolve in a native build.

## The builder escape hatch

The value API does not cover every control. Tabs, menu bars, dialogs, popovers,
and virtual lists live on the builder, and a tooltip is a node method.
`App.launch` takes a `fn(Build): Node`, and builder nodes use the same layout
and paint code:

```rnx
import { App } from "@std/app"
import { Build, Node } from "@std/ui"

fn Bar(b: Build): Node {
    return b.row().padding(8).gap(8) {
        b.button("Back", () => {})
        b.spacer()
        b.tabs(["Day", "Week"], 0, (i: Int) => { println!(i) })
    }
}

fn Main(): Int {
    let app = App.launch("Bar", 640, 120, (b: Build): Node => Bar(b))
    return app.show()
}
```

A builder node exposes the full node surface: `margin`, `minWidth`, `minHeight`,
`alignMain`, `alignCross`, `alignSelf`, `order`, `gapPercent`, `offsetY`, and
the `disabled`, `inert`, `tooltip`, and relation setters. Reach for it when the
value API has no constructor, and keep the boundary at one function so the rest
of the screen stays declarative.
