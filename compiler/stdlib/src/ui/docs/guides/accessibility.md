---
title: Accessibility
description: Name controls, expose state, and check the semantic tree your app publishes.
section: Accessibility
order: 100
---

# Accessibility

Every mounted view becomes a semantic node. The node carries a role, a name, a
value, its bounds, an action mask, and its state flags. `@std/app` publishes that
projection through AccessKit, and `Driver` uses the same projection in tests. A
screen reader action and a pointer click run the same handler, so they cannot
disagree.

The constructor chooses the role. `Text` is `text`, `Heading` is `heading`,
`Button` is `button`, `Checkbox` is `checkbox`, `Slider` is `slider`,
`TextField` and `RichTextField` are `textbox`, `List` is `list`, `Grid` is
`grid`, `Scroll` is `scroll`, `Image` is `image`, `Scene` is `canvas`, and
`DisplayText` is `display`. `Page` is a column, `Header` and `Toolbar` are rows,
and `Card`, `Section`, and `Panel` are sections.

## Names and state

A text node uses its text as the name, and an interactive node uses the label
passed to its constructor. `View.named` overrides the name on text and container
views. A control keeps the label it was created with, so pass the wording you
want a screen reader to read:

```rnx
import { Checkbox, Page, Text, View } from "@std/ui"

fn Task(done: Bool): View {
    let check = Checkbox("Write the documentation", done, () => {})
    check.description = "Marks the task complete"

    let status = Text("Saved").named("Save status")
    status.live = 1

    return Page(title: "Tasks") {
        check
        status
    }
}
```

`description` is read after the name. `live` marks a live region: `0` is off,
`1` is polite, and `2` is assertive. `View.level` sets a heading level, and
`View.selected`, `expanded`, `busy`, `required`, and `invalid` map to the
matching semantic states. `invalid` takes `0` for valid, `1` for invalid, `2`
for spelling, and `3` for grammar.

```rnx
import { Page, TextField, View } from "@std/ui"

fn EmailField(value: String, onChange: fn(String): Void): View {
    let email = TextField(value, "Email", onChange)
    email.required = true
    email.invalid = 1
    email.description = "We only use this for receipts."
    return Page(title: "Receipt", padding: 20) {
        email
    }
}
```

## Links

A link span becomes a child node with role `link`, the link text as its name,
and the URL as its value and description. The text around it stays in the
parent text node:

```rnx
import { FontFamily, RichText, TextDecoration, TextSpan, View } from "@std/ui"

fn ReleaseNote(): View {
    let spans = [
        new TextSpan(9, 17, 500, false, 0.0, -1, FontFamily.sans, TextDecoration.none, null, "https://example.test/releases"),
    ]
    return RichText("Read the releases.", spans)
}
```

`Tree.linkAt(x, y)` returns the URL under a point, and `Tree.linkRects(id)`
returns one rectangle per link. The app forwards a click through `App.onLink`
once, without also activating the node behind the text.

## Checking the projection

`tree.snapshot()` prints one line per node in tree order. It is the quickest way
to see the roles and names a screen exposes:

```rnx
import { Checkbox, Page, TextEngine, Tree, View } from "@std/ui"

fn task(): View {
    let check = Checkbox("Write the documentation", false, () => {})
    return Page(title: "Tasks") {
        check
    }
}

let tree = new Tree(TextEngine.fixed())
tree.beginFrame()
let root = View.mount(tree, task())
tree.endFrame()
tree.layout(root, 320)
println!(tree.snapshot())
```

`tree.unnamedInteractive()` returns the first interactive node without a name,
so a test can fail on a control that would reach a screen reader as unnamed.
`@std/app` also exposes the frame projection through `App.accessibility()`, with
`find`, `at`, and `dump`.

Background content behind a modal can be removed from the projection with
`Node.inert`, set from `View.onMount`. An inert subtree leaves focus traversal,
hit testing, and interactive semantics, which is the same rule a native modal
applies to its background.
