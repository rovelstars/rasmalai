---
title: Input
description: Route pointer, keyboard, touch, wheel, shortcut, and gamepad events into the view.
section: Input
order: 40
---

# Input

The window host and the headless core feed the same handlers. A pointer press in
a live window and `App.pointerDown` in a test reach the same node callback, so a
test does not need a second input path.

`App.pointerDown(x, y)`, `pointerMove`, `pointerUp`, and `pointerLeave` route the
mouse. `App.scrollAt(x, y, dy)` feeds the wheel; the host accumulates the
continuous Wayland axis value and emits whole-pixel steps. `App.keyDown(key)`
routes a `Key` to the focused node and focuses the first control when nothing is
focused. `App.focusNext()` advances focus.

## Buttons and shortcuts

```rnx
import { App } from "@std/app"
import { Button, Page, Text } from "@std/ui"

class Counter {
    let n: Int = 0
}

fn Main(): Int {
    let state = new Counter()
    let app = App.launchView("Input", 320, 220, () => Page(title: "Input") {
        Text("Count " + state.n.toString())
        Button("Add", () => { state.n += 1 })
    })
    app.bind("ctrl+s", () => { println!("saved") })
    app.bind("f1", () => { state.n = 0 })
    return app.show()
}
```

`App.bind(chord, run)` takes a chord such as `ctrl+s` or `f1`.
`Tab` and `Shift+Tab` move focus without a binding. `App.shortcuts()` returns the
`Shortcuts` table when you need a modal scope. With `app` in scope:

```rnx
let shortcuts = app.shortcuts()
shortcuts.bind("f2", () => { println!("global") })
let scope = shortcuts.pushScope()
shortcuts.bindScoped(scope, "f2", () => { println!("modal") })
shortcuts.popScope()
```

A pushed scope shadows global chords until it is popped.

## Send input without a window

`App.sendKey(code, down)` feeds an evdev key code. Key code `30` is `a`, `14` is
backspace, and `29` is the left control modifier. Modifiers are tracked across
calls, so a shortcut can be driven as a press and release pair.

```rnx
import { App } from "@std/app"
import { Button, Page, TextField } from "@std/ui"

class Form {
    let name: String = ""
}

fn Main(): Int {
    let state = new Form()
    let app = App.launchView("Type", 320, 220, () => Page(title: "Type") {
        TextField(state.name, "Name", (v: String) => { state.name = v })
        Button("Save", () => { println!("saved " + state.name) })
    })
    app.frame()
    app.focusNext()
    app.sendKey(30, true)
    app.sendKey(30, false)
    let frame = app.frame()
    println!(state.name)
    println!(frame.stats())
    return 0
}
```

Build one frame first, then `focusNext` lands on the text box. The key arrives
as text. A focused text box also receives `Ctrl+C`, `Ctrl+X`, `Ctrl+V`, and
`Ctrl+A`. `App.copy`, `App.cut`, `App.paste`, and
`App.clipboardText` are the same operations from code.

The window host loads the Wayland XKB keymap, so typing follows the keyboard
layout instead of a fixed US table. Held keys repeat from the compositor's repeat
info. The host also speaks `text-input-v3`: preedit text is drawn inline without
changing the value, commit inserts at the caret, and leaving the surface clears
the preedit.

## Touch and pointer capture

Host touch ids use `App.touchDown(id, x, y)`, `touchMove`, `touchUp`, and
`touchCancel`. `App.lockPointer()` locks the pointer to its position,
`App.confinePointer()` keeps motion inside the window, and
`App.releasePointer()` clears either mode. A slider drag confines the pointer
automatically and releases it on pointer up or leave.

## Gamepads

`App.onGamepad(handler)` runs for axis, button, and connection changes. Axes are
normalized to the range -1 to 1 and buttons report pressed state. With `app` in
scope:

```rnx
import { Gamepad } from "@std/app"

app.onGamepad((pad: Gamepad) => {
    if pad.pressed(0) {
        println!(pad.axis(0))
    }
})
```

`App.gamepad()` returns the current pad. The host reads `/dev/input/js*` and
sends axis and button events, so a test can set the same fields with `setAxis`,
`setButton`, and `setConnected`.

## Keep going

Read [Reactive state](reactive-state) to rebuild only the components that read a
changed value.
