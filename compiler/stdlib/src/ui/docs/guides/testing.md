---
title: Testing
description: Assert on the semantic tree, drive real controls, and keep frame timing fixed.
section: Testing
order: 110
---

# Testing

A `@std/ui` screen is a value, so a test can mount it without a window. The
snapshot of the retained tree gives you roles, names, and order. For behavior,
`@std/app` adds a driver that presses the same handlers a person would.

## Snapshot test

A `test fn` runs under `rnx test`. Mount the view into a `Tree` with the fixed
text engine, lay it out, and assert on `snapshot()`:

```rnx
import { Page, Text, TextEngine, Tree, View } from "@std/ui"

fn screen(): View {
    return Page(title: "Today") {
        Text("3 open")
    }
}

test fn pageSnapshotHasTitleAndRows() {
    let tree = new Tree(TextEngine.fixed())
    tree.beginFrame()
    let root = View.mount(tree, screen())
    tree.endFrame()
    tree.layout(root, 320)
    let text = tree.snapshot()
    assert(text.contains("heading Today"), text)
    assert(text.contains("text 3 open"), text)
}
```

The fixed engine gives every character the same width, which makes the
structure deterministic. Do not assert pixel positions from it. Text geometry
belongs in a test that installs the real shaper, which `@std/app` does.

Run the file with `rnx test`. A filter runs one test by name, as in
`rnx test pageSnapshotHasTitleAndRows`.

`tree.find(Role.text, "3 open")` returns a node id when you need to inspect one
node. `tree.unnamedInteractive()` catches a control that would reach a screen
reader without a name.

## Driver test

`Driver.on(app)` finds nodes by role and name. Each action rebuilds a frame, so
the next lookup sees the new state:

```rnx
import { App, Driver } from "@std/app"
import { Button, Page, Text, View } from "@std/ui"

class Tasks {
    let count: Int = 0
    fn add() { this.count += 1 }
}

fn Screen(state: Tasks): View {
    return Page(title: "Tasks") {
        Text(state.count.toString() + " open")
        Button("Add", () => state.add())
    }
}

test fn addIncrementsTheCount() {
    let state = new Tasks()
    let app = App.launchView("Tasks", 480, 360, () => Screen(state))
    let driver = Driver.on(app)
    assert(driver.press("button", "Add"), "Add is reachable")
    assert(driver.exists("text", "1 open"), driver.dump())
}
```

`press` runs the primary action. `click` sends a pointer down and up at the
node center. `increment` and `setValue` change a slider or field. `focus` moves
focus, and `value` reads the current value back. `dump()` prints the whole
projection when an assertion needs a diagnostic.

Names matter here. The driver matches the label you pass to a control, and the
name of a text or container view. `View.named` sets that name, so use it when a
label is ambiguous or when a test needs a stable lookup key.

## Deterministic frames

`App` normally reads a monotonic clock. `App.setFixedFrameNs` replaces it with a
fixed step, which is what a frame-by-frame animation test needs:

```rnx
import { App } from "@std/app"
import { Page, Text, View } from "@std/ui"

let app = App.launchView("Motion", 320, 200, () => Page {
    Text("frame")
})
app.setFixedFrameNs(16000000)
app.frame()
app.frame()
```

`16000000` is 16 milliseconds. With a fixed step, `dt()` is the same on every
frame and a spring or keyframe track lands on the same values on every run.

For a reactive test, keep the app frame and the graph in step. Bind the graph
before writing, then assert on the rebuild counters:

```rnx
import { App } from "@std/app"
import { Page, Reactive, Signal, Text, View } from "@std/ui"

fn Counter(rx: Reactive, count: Signal<Int>): View {
    return Page(title: "Counter") {
        rx.component("label", () => {
            let n = count.get() as Int
            return Text(n.toString())
        })
    }
}

test fn counterRebuildsOnce() {
    let rx = new Reactive()
    let count = rx.signal(0)
    let app = App.launchView("Counter", 320, 200, () => Counter(rx, count))
    app.bindReactive(rx)
    count.set(2)
    app.frame()
    assert(rx.componentRebuilds("label") == 1, "the label rebuilt once")
}
```

`Reactive.componentRebuilds(key)`, `componentDirty(key)`, and `frameNeeded()`
expose the graph's state without reading private fields.

## Screenshots

`App.screenshot(path)` renders the current frame on the CPU and writes a PNG.
`App.screenshotBytes()` returns the bytes. `Driver.screenshot(path)` does the
same after an action, and MCP `ui.screenshot` takes a path as its value. No
compositor is involved, so a test captures exactly the frame it just built.

For a run without a window, set `RNX_UI_SCRIPT` to a semicolon-separated script
and call `App.show()`. Steps such as `press:button:Add`, `set:textbox:Name:Ada`,
and `shot:/tmp/result.png` drive the app headlessly and print `ui.result ok` or
`ui.result fail <step>`.
