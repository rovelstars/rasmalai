---
title: Layout
description: Choose a container, size children, and place overlays with the value API.
section: Layout
order: 30
---

# Layout

Layout answers two questions: which axis does a container stack on, and how much
space does each child get. `@std/ui` uses the container constructor to choose
the axis. There is no public `Row` or `Column`.

Vertical containers are `Page`, `Section`, `Card`, `List`, `Panel`, and
`Stack` (the default). Horizontal containers are `Header`, `Toolbar`,
`ListRow`, and `Stack(horizontal: true)`.

```rnx
import { Button, ButtonTone, Card, Header, Page, Spacer, Stack, Text, View } from "@std/ui"

fn Settings(): View {
    return Page(title: "Settings", padding: 24, gap: 16) {
        Header {
            Text("Rasmalai")
            Spacer()
            Button("Done", () => {})
        }
        Card(title: "Profile") {
            Text("Ada Lovelace")
            Text("ada@example.test")
        }
        Stack(horizontal: true, gap: 12) {
            Button("Cancel", () => {})
            Button("Save", () => {}, tone: ButtonTone.accent)
        }
    }
}
```

`Spacer` has `grow(1.0)`, so it takes the free space in a row and pushes its
siblings apart. The same trick holds a column open between a header and a
footer.

## Sizing a child

Every `View` accepts `width`, `height`, `grow`, `maxWidth`, and `aspectRatio`.
Set them as fields after construction, because several of them depend on the
actual content:

```rnx
import { Button, Card, Grid, Page, Scroll, Text, View } from "@std/ui"

fn Workspace(keys: Array<String>, press: fn(String): Void): View {
    let body = Scroll(gap: 12, padding: 16) {
        Grid(columns: 2, gap: 12) {
            Card(title: "Left") { Text("A") }
            Card(title: "Right") { Text("B") }
        }
    }
    body.height = 320
    body.grow = 1.0

    let keypad = Grid(columns: 4, gap: 8) {
        for label in keys {
            Button(label, () => press(label))
        }
    }
    keypad.maxWidth = 320
    keypad.center = true

    return Page(padding: 16, gap: 12) {
        body
        keypad
    }
}
```

`grow` takes a share of the free main-axis space. `maxWidth` caps the measured
width, and `center` sets the child's cross-axis alignment to center, which is
how the keypad stays centered on a wide window. `aspectRatio` sets height from
width when no height is given, so square keys stay square as the window
resizes.

`Scroll` is a clipped viewport. Give it a height through the `height` field or
`grow`, add children, and the wheel scrolls it. The offset lives on the keyed
node, so a rebuild does not jump the content back to the top.

## Grids and panels

`Grid(columns: 2)` gives each column an equal share after the gaps, and each row
is as tall as its tallest cell. `Panel` is a padded surface with a name and no
heading. `Section(title:)` and `Card(title:)` add a heading, and `Card` also
takes an elevation token.

```rnx
import { List, ListRow, Page, Panel, Section, Text, View } from "@std/ui"

fn Dashboard(): View {
    return Page(title: "Dashboard", padding: 20) {
        Panel(name: "summary", padding: 12) {
            Text("12 tasks")
            Text("4 due today")
        }
        Section(title: "Recent") {
            List {
                ListRow { Text("Design review") }
                ListRow { Text("Release notes") }
            }
        }
    }
}
```

## Absolute placement

`Place` and `Inset` take a child out of the flow. `Place` pins a rectangle;
`Inset` pins the four edges, and two opposite edges set the size between them.

```rnx
import { Inset, Overlay, Page, View } from "@std/ui"

fn Stage(content: View, hud: View): View {
    let overlay = Overlay {
        hud
    }
    return Page(padding: 0, gap: 0) {
        content
        Inset(overlay, 0, 0, 0, 0)
    }
}
```

`Overlay` marks its subtree as a portal. It paints above the rest of the tree
and wins hit testing there, which keeps a HUD or a sheet fixed while the content
under it moves. `Place(content, x, y, w, h, z, clip)` and
`Inset(content, l, t, r, b, z, clip)` also set a z index and a clip flag.

The equivalent methods are `View.at(x, y, w, h)`, `View.inset(l, t, r, b)`,
`View.zIndex(n)`, and `View.clipped()`. A clip cuts the subtree; `clipRadius`
rounds the cut.

## Direction and scale

The tree carries a few presentation settings that apply to everything below it.
`Node.rtl()` flips row order and mirrors start and end text alignment, while
columns keep their top-to-bottom order. `Node.vertical()` stacks text glyphs down
the block axis. Set either from `View.onMount` on the subtree root:

```rnx
import { Header, Node, Page, Spacer, Text, Tree, View } from "@std/ui"

fn Localized(): View {
    let page = Page(title: "Home") {
        Header {
            Text("Title")
            Spacer()
        }
        Text("Body")
    }
    page.onMount((t: Tree, n: Node) => {
        n.rtl()
        t.setTextScale(1.2)
        t.setSafeArea(24, 0, 0, 0)
    })
    return page
}
```

`Tree.setTextScale(f)` multiplies every text role for dynamic type, in
measurement and painting alike. `Tree.setDensity(d)` records the device pixel
ratio, and `Tree.dp(n)` converts a logical length to device pixels.
`Tree.setSafeArea(top, right, bottom, left)` and `Tree.setKeyboardInset(bottom)`
shrink and offset the root so content stays clear of system bars and the soft
keyboard.

## Builder layout

A builder `Node` adds the rules the value API keeps out of the way: `margin`,
`minWidth` and `minHeight`, `alignMain`, `alignCross`, `alignSelf`, `order`,
`gapPercent`, `autoMargin`, `minContent`, `maxContent`, and `fitContent`.
[Components](components) shows the builder boundary.
