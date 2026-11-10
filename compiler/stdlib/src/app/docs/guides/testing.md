---
title: Testing
description: Drive a view headless with Driver, UI scripts, screenshots, and MCP tools.
section: Testing
order: 70
---

# Testing

A UI test does not need a compositor. `Driver` reads the accessibility
projection of the same tree that pointer and keyboard input use, then runs the
same handlers. A press from the driver and a click from a person cannot diverge.

Create the app, build a frame, then wrap it:

```rnx
import { App, Driver, Mcp } from "@std/app"
import { Button, Page, TextField } from "@std/ui"

class Form {
    let name: String = ""
}

fn Main(): Int {
    let state = new Form()
    let app = App.launchView("Form", 320, 220, () => Page(title: "Form") {
        TextField(state.name, "Name", (v: String) => { state.name = v })
        Button("Save", () => { println!("saved " + state.name) })
    })

    let driver = Driver.on(app)
    driver.frame()
    if !driver.setValue("textbox", "Name", "Ada") {
        return 1
    }
    println!(driver.value("textbox", "Name"))

    let status = app.runScript("assert:textbox:Name;assertValue:textbox:Name:Ada")
    println!(status)

    let mcp = Mcp.on(driver)
    println!(mcp.tools().contains("ui.press"))
    println!(mcp.call("ui.press", "button", "Save", "", 0.0))
    return 0
}
```

`Driver` finds nodes by role and name. `exists` reports whether a match is
present; `press`, `click`, `increment`, `setValue`, and `focus` run the matching
handler and build a new frame. `value`, `checked`, `selectedOf`, `expandedOf`,
`busyOf`, `descriptionOf`, and `rangeOf` read state. `dump()` writes every node
with its role, name, value, bounds, actions, and flags.

`App.accessibility()` returns the projection as `Accessibility`, and
`App.accessibilityTree()` returns the same tree without announcements. Each
`A11yNode` carries its role, name, value, bounds, action mask, checked and
disabled state, and focus. The host encodes the projection into an AccessKit
tree and publishes it over AT-SPI when the desktop enables accessibility. A
screen reader action returns as an event and runs the same handlers as a click.
`App.announce(text, priority)` adds a transient announcement; priority 1 is
polite and priority 2 is assertive.

`Driver.screenshot(path)` writes the current frame after an action.
`Driver.encode()` returns the accessibility packet, and `Driver.announce` sends a
transient message to assistive technology.

## Scripts

`app.runScript(script)` runs semicolon-separated steps headless. It builds a
frame before the first step and after each state change, and returns zero when
every step passes. A failing step prints `ui.result fail` and returns one.

The verbs are `press`, `click`, `focus`, `set`, `inc`, `key`, `scroll`, `fling`,
`drag`, `longPress`, `doubleTap`, `pinch`, `drop`, `wait`, `shot`, `stats`,
`timeline`, `announce`, and `inspect`. Assertions are `assert`,
`assertValue`, `assertChecked`, `assertSelected`, `assertExpanded`,
`assertBusy`, `assertDescription`, `assertRange`, and `assertFocus`.

Each action and assertion step names a role and an accessible name, separated by
colons. The last field carries the value, delta, key, or path:

```rnx
let status = app.runScript("press:button:Save;wait:2;assertValue:textbox:Name:Ada")
println!(status)
```

`app.show()` honors `RNX_UI_SCRIPT`; when it is set, the script runs instead of
opening a window. That gives CI the same entry point as a normal run.

## MCP

`Mcp.on(driver)` projects the driver as tools: `ui.list`, `ui.press`,
`ui.increment`, `ui.setValue`, `ui.screenshot`, `ui.tree`, `ui.openLink`, and
the assertion tools. `tools()` returns the schema. `call(tool, role, name,
value, delta)` returns a JSON result with `isError` and a text content block.

This projection is opt-in. The app chooses the transport and passes the JSON
where it wants. A screenshot call passes its path as `value`.

## Screenshots and goldens

`App.screenshot(path)` and `App.screenshotBytes()` render on the CPU, so they do
not need a window or a compositor screenshot. The encoder uses stored deflate and
has no compression dependency. `Surface.pixel`, `rgbaAt`, and `pixels` let a test
assert exact colors.

## Keep going

Read [Native wrappers](native-wrappers) to build the libraries the app runtime
calls.
