---
title: Views
description: Compose screens from View values, fill children with blocks, and key rows that move.
section: Views
order: 20
---

# Views

A `View` is a description of one node. It holds a kind, optional text, children,
and style attributes. The component functions create it, and `View.mount` turns
it into retained nodes.

The smallest screen is a container with two children:

```rnx
import { Button, Page, Text, View } from "@std/ui"

fn Greeting(): View {
    return Page(title: "Hello", padding: 24, gap: 12) {
        Text("Welcome back.")
        Button("Continue", () => {})
    }
}
```

The trailing block fills the callee's `children` parameter. `Page { Text("a") }`
is the same call as `Page(children: [Text("a")])`. Named arguments set the other
parameters, so `Page(title: "Hello", gap: 12)` needs no builder calls.

## Rows that move

Lists reorder, and a rebuilt view matches nodes by position unless you supply a
key. `View.keyed` gives a view a stable identity. When a keyed row moves, the
retained node moves with it, and focus, scroll, and hover stay on the row.

The next example takes an `Inbox` with a `messages` array. Each message has an
`id`, a `subject`, and a `time`:

```rnx
import { List, ListRow, Page, Spacer, Text, View } from "@std/ui"

fn Inbox(state: Inbox): View {
    return Page(title: "Inbox", padding: 16, gap: 8) {
        List {
            for message in state.messages {
                ListRow(padding: 12) {
                    Text(message.subject)
                    Spacer()
                    Text(message.time)
                }.keyed(message.id.toString())
            }
        }
        if state.messages.length() == 0 {
            Text("Nothing here")
        }
    }
}
```

`if` and `for` work inside a collection block. A statement whose value is a
`View` becomes a child, so a loop adds one child per iteration and a false
branch adds none.

Use a key that survives the data, such as a row id. An index is stable only
while the list never inserts or removes from the middle.

## Arrays of children

When the children already exist, pass them as an array. This is useful when a
helper returns a list of rows:

```rnx
import { ListRow, Page, Text, View } from "@std/ui"

fn Inbox(state: Inbox): View {
    let rows: Array<View> = []
    for message in state.messages {
        rows.push(ListRow(padding: 12) {
            Text(message.subject)
        }.keyed(message.id.toString()))
    }
    return Page(title: "Inbox", children: rows)
}
```

The block form and the `children` parameter produce the same tree. Use whichever
keeps the logical structure readable.

## Attributes

Constructors cover the common case. The remaining attributes are fields on the
`View` and are read when the view mounts, so set them after construction and
before you return the view:

```rnx
import { Card, Node, Text, Tree, View } from "@std/ui"

fn Stats(): View {
    let card = Card(title: "Stats")
    card.padding = 20
    card.gap = 6
    card.maxWidth = 360
    card.center = true
    card.onMount((t: Tree, n: Node) => {
        n.tooltip("Updated hourly")
    })
    return card
}
```

The attributes used most often are `padding`, `gap`, `width`, `height`,
`grow`, `maxWidth`, `center`, `aspectRatio`, `opacity`, `blend`, `clipRadius`,
`shadowColor`, and `z`. [Layout](layout) covers sizing, and
[Colors and themes](colors-and-themes) covers paint attributes.

`View.named` sets the accessible name on a text or container view, which also
makes the node easy to find from a test. A control keeps the label from its
constructor. `View.keyed` sets the reconciliation key. Both return the same
view, so they can be chained.

## Mounting

Headless code mounts a view into a `Tree`:

```rnx
import { TextEngine, Tree, View } from "@std/ui"

let tree = new Tree(TextEngine.fixed())
tree.beginFrame()
let root = View.mount(tree, Greeting())
tree.endFrame()
tree.layout(root, 480)
```

`beginFrame` and `endFrame` bracket a rebuild so the tree can reconcile. The
layout call needs a width; the app follows it with the window height so a root
container can fill the window. `tree.snapshot()` returns the semantic tree as
text, one node per line.

`View.onMount` runs after the node is created and styled. Use it for the few
node-level operations that have no value API equivalent, such as a tooltip or a
disabled flag. [Input and focus](input-and-focus) shows that pattern.
