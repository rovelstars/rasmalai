---
title: State and reactive
description: Build a signal graph, schedule effects, and rebuild only the components that read a value.
section: State
order: 60
---

# State and reactive

A plain class field is enough for state that one screen owns. When several
parts of a screen read the same value, `Reactive` gives you a graph with
explicit edges. Nothing observes the graph behind your back, and a test can use
its own graph without touching another test.

```rnx
import { Reactive } from "@std/ui"

let rx = new Reactive()
let count = rx.signal(0)
let doubled = rx.computed<Int>(() => count.get() * 2)

rx.effect(() => {
    println!("count", count.get())
})

count.set(2)
rx.drain()
println!(doubled.get())
```

`Signal.set` records the new value and schedules dependent effects. It does not
run them synchronously. `drain` runs the queued effects once in stable id order,
so the output order does not depend on hash iteration or insertion accidents.

`Computed.get` is lazy. The first read runs the closure, and later reads reuse
the value until a dependency version changes. `batch` coalesces a group of
writes and drains once at the end:

```rnx
rx.batch(() => {
    count.set(1)
    count.set(2)
})
```

The graph is per object. Two apps, or two tests, never share dependencies.

## Components

`Reactive.component` marks a keyed subtree boundary. The builder runs inside the
component's computation scope, so every signal read in the builder subscribes
that component and nothing else:

```rnx
import { Button, Page, Reactive, Signal, Text, View } from "@std/ui"

fn Counter(rx: Reactive, count: Signal<Int>): View {
    return Page(title: "Counter") {
        rx.component("label", () => {
            let n = count.get() as Int
            return Text(n.toString() + " clicks")
        })
        Button("Add", () => { count.set(count.get() + 1) })
    }
}
```

Writing `count` dirties the `label` component. A clean component returns its
cached `View` and does not run the builder, which keeps a sibling panel with its
own signal untouched. An empty key mounts the builder with no caching.

The explicit `as Int` is needed inside a nested closure in a native build. The
generic `Signal` value has no concrete type there, so the cast gives the call
site something to resolve.

## Connecting to the app

`App.bindReactive(rx)` connects the graph to the frame loop. At the start of
each frame the app drains queued effects, and while a component is dirty or an
effect is queued it keeps requesting frames:

```rnx
import { App } from "@std/app"
import { Button, Page, Reactive, Signal, Text, View } from "@std/ui"

fn Counter(rx: Reactive, count: Signal<Int>): View {
    return Page(title: "Counter") {
        rx.component("label", () => {
            let n = count.get() as Int
            return Text(n.toString() + " clicks")
        })
        Button("Add", () => { count.set(count.get() + 1) })
    }
}

fn Main(): Int {
    let rx = new Reactive()
    let count = rx.signal(0)
    let app = App.launchView("Counter", 320, 200, () => Counter(rx, count))
    app.bindReactive(rx)
    return app.show()
}
```

Without `bindReactive`, the graph still works, but you own the drain call and
the frame request. The reactive test in [Testing](testing) shows that form.

`Signal.dispose`, `Computed.dispose`, and `Effect.dispose` drop a handle and its
edges. A disposed signal keeps its last value and ignores writes. `Reactive`
also reports `frameNeeded()`, `componentDirty(key)`, and
`componentRebuilds(key)`, which are useful assertions in a test.
