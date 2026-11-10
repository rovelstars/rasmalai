---
title: Reactive state
description: Use signals, computed values, effects, and keyed components to rebuild only what changed.
section: State
order: 50
---

# Reactive state

A view runs again on every frame. Rebuilding an unchanged subtree is wasted work,
and rebuilding a text field can move focus. The reactive graph lets a component
cache its `View` and rerun its builder only when a signal it read changes.

The graph is explicit. `new Reactive()` creates one, and each app owns one. There
is no process-global observer, so two app surfaces in one process do not share
dependencies.

## A counter

```rnx
import { App } from "@std/app"
import { Button, Page, Panel, Reactive, Text } from "@std/ui"

fn Main(): Int {
    let rx = new Reactive()
    let count = rx.signal(0)
    let app = App.launchView("Reactive", 320, 220, () => Page(title: "Reactive") {
        rx.component("counter", () => {
            let n = count.get() as Int
            return Panel(padding: 0, gap: 8) {
                Text("Count " + n.toString())
                Button("Add", () => { count.set(n + 1) })
            }
        })
    })
    app.bindReactive(rx)
    return app.show()
}
```

`rx.component(key, build)` is a keyed subtree boundary. The builder runs inside
the reactive scope, so a signal read there subscribes that component and nothing
else. When `count` changes, only the `counter` component is dirty. A clean
component returns its cached `View` and does not run the builder.

`app.bindReactive(rx)` connects the graph to the frame loop. The app drains
queued effects at the start of a frame and keeps requesting frames while a
component is dirty or an effect is queued.

## Signals, computed values, and effects

`rx.signal(value)` creates a `Signal<T>`. `get()` reads the value and records a
dependency when a computation is running. `set(value)` schedules dependent
effects instead of running them synchronously. `update(f)` maps the current value
through `f` and writes the result.

`rx.computed<T>(f)` is lazy. `get()` recomputes only when a dependency version
changed:

```rnx
let rx = new Reactive()
let count = rx.signal(0)
let doubled = rx.computed<Int>(() => count.get() * 2)

println!(doubled.get())
```

`rx.effect(f)` queues `f` to run once during a drain. Effects run in stable id
order. `rx.batch(f)` coalesces a group of writes until the block ends:

```rnx
let rx = new Reactive()
let count = rx.signal(0)

rx.effect(() => {
    println!("count", count.get())
})

count.set(2)
rx.drain()
```

In a live app, `bindReactive` drains at frame start, so an effect usually needs
no manual `drain`. In a test or a plain function, call `rx.drain()` yourself.

An empty component key mounts the builder without caching. Use a real key when
the subtree should keep its nodes across rebuilds.

## Keep going

Read [Routing](routing) to switch whole views while keeping keyed components and
history intact.
