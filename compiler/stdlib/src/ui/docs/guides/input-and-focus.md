---
title: Input and focus
description: Wire controls, understand keyboard focus, and carry typed payloads between views.
section: Input
order: 90
---

# Input and focus

Controls are leaves with callbacks. The retained tree owns focus, hover, and
pressed state, and it sends every pointer click, key press, and accessibility
action to the same handler. A test click and a test `Enter` therefore exercise
one code path, not two.

```rnx
import { Button, Checkbox, Page, Text, TextField, View } from "@std/ui"

class Profile {
    let name: String = ""
    let visible: Bool = false
    fn save() {}
}

fn Form(state: Profile): View {
    return Page(title: "Profile", padding: 20, gap: 12) {
        Text("Name")
        TextField(state.name, "Your name", (v: String) => { state.name = v })
        Checkbox("Public", state.visible, () => { state.visible = !state.visible })
        Button("Save", () => state.save())
    }
}
```

`TextField` keeps a caret and a selection. Typing goes through the text engine,
so grapheme clusters behave correctly, and `Backspace`, `Delete`, the arrow
keys, `Home`, and `End` edit the field. A click focuses a field or slider
without painting the focus ring; the ring appears for keyboard navigation and
programmatic focus.

Tab moves focus to the next interactive node, and `Shift+Tab` moves back.
`Enter` and `Space` run the primary action. `Escape` dismisses a dialog, and the
left and right arrows move between interactive siblings, which is how a radio
group changes its selection.

## Disabling a control

The value API has no disabled flag. `View.onMount` receives the node, so set the
flag there, or use the builder for a control that stays disabled:

```rnx
import { Button, Node, Tree, View } from "@std/ui"

fn SaveButton(ready: Bool): View {
    let save = Button("Save", () => {})
    save.onMount((t: Tree, n: Node) => {
        n.disabled(!ready)
        n.tooltip("Fill every field first")
    })
    return save
}
```

A disabled node leaves focus traversal, hover activation, and dispatch. The flag
is applied on every mount, so a rebuild can change it with the state.

## Drag and drop

A drag source carries a typed payload, and a drop target accepts one or more
kinds. The payload starts after the pointer moves past 4 pixels, so a click on
the same card is still a click:

```rnx
import { Card, DragData, Page, Text, View } from "@std/ui"

class BoardState {
    let last: String = ""
}

fn Board(state: BoardState): View {
    let card = Card(title: "Drag me")
    card.draggable("text", "hello drop")

    let zone = Card(title: "Drop text here")
    zone.dropTarget("text", (d: DragData) => {
        state.last = d.kind + " " + d.text
    })

    return Page(title: "Board", padding: 20, gap: 12) {
        card
        zone
        Text(state.last)
    }
}
```

A target accepts a comma-separated kind list such as `"text,file"` or `"*"` for
any kind. The nearest accepting node on the hit path wins. A rejected drop
leaves the payload unmoved.

The tree also tracks drag phases, long presses, double taps, and pinch. Those
are exposed on `Tree` as `dragPhase()`, `longPressed()`, `tapCount()`,
`pinchScale()`, and `pinchRotation()`, and the app driver can produce them in a
test. [Testing](testing) covers the driver.

## Scrolling

The wheel moves the nearest scrollable ancestor under the pointer. When the
inner viewport reaches its edge, the next step goes to the scrollable ancestor
outside it. `Scroll` is clipped and keeps its offset on the keyed node, so a
rebuild does not return the view to the top. Give it a height through the
`height` field or `grow`.
