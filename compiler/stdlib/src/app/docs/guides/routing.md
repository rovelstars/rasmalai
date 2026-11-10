---
title: Routing
description: Map paths to views with flat or nested routes, params, history, and transitions.
section: Routing
order: 60
---

# Routing

`ViewRouter` turns a path into a `View`. It keeps its own route table and history
stack, so two routers in one app do not share navigation state. Patterns hold
exact segments, `:name` parameters, and a final `*` wildcard.

## A flat route table

```rnx
import { App, ViewRouter } from "@std/app"
import { Page, Text } from "@std/ui"
import { Map } from "@std/collections"

fn Main(): Int {
    let router = new ViewRouter()
    router.route("/", (_p: Map<String, String>) => Page(title: "Home") {
        Text("Home")
    })
    router.route("/tasks/:id", (p: Map<String, String>) => Page(title: "Task") {
        Text("Task " + (p.get("id") ?? ""))
    })
    router.navigate("/tasks/7")
    let app = App.launchView("Router", 480, 320, () => router.view())
    return app.show()
}
```

The builder receives the matched params as a `Map<String, String>`.
`router.view()` builds the current path. `router.currentPath()` reads it,
`navigate` pushes the previous path, `back` pops it, `replace` changes the path
without growing the stack, and `depth()` reports the stack length.
`router.params()` returns the params of the current path.

Matching prefers an exact segment, then a `:param`, then a `*` wildcard at each
level. A query string is parsed into the same map, so `/tasks/7?tab=notes` has
both `id` and `tab`. `router.resolve(path)` builds a path without changing
history, which is useful for tests.

## Nested layouts

A `Route` can own children. The parent builder receives a `RouteMatch`, reads
`params()`, and places `outlet()` where the child belongs:

```rnx
import { Route, RouteMatch, ViewRouter } from "@std/app"
import { Page, Text, View } from "@std/ui"

fn Main(): Int {
    let root = new Route("/", (m: RouteMatch): View => Page(title: "App", padding: 0, gap: 0, children: [
        Text("Layout"),
        m.outlet(),
    ]), [
        new Route("/tasks", (m: RouteMatch): View => Page(title: "Tasks", padding: 0, gap: 0, children: [
            Text("Tasks"),
            m.outlet(),
        ]), [
            new Route("/", (_m: RouteMatch): View => Text("List"), []),
            new Route("/:id", (m: RouteMatch): View => Text("Task " + (m.params().get("id") ?? "")), []),
        ]),
        new Route("/*", (_m: RouteMatch): View => Text("Not found"), []),
    ])

    let router = ViewRouter.fromRoute(root)
    router.navigate("/tasks/7")
    let app = App.launchView("Nested", 480, 320, () => router.view())
    return app.show()
}
```

`RouteMatch.path()` is the full requested path, `params()` holds merged parent
and child params plus the query string, and `outlet()` returns the child view for
the remaining path. A child key shadows the parent key. A missing child becomes
the not-found page inside the parent layout.

`ViewRouter.nested(root)` attaches a nested root to an existing router.
`withTransition(Transition.fast())` wraps a path change in a crossfade: the
outgoing view stays mounted under its old generation key while it fades out, and
the incoming view fades in over it. `fromRoute` is the short form when the nested
root is the whole table.

The older `Router` class builds `fn(): Node` views with the same `route`,
`navigate`, `replace`, `back`, `currentPath`, and `view` methods. Prefer
`ViewRouter` in value-based app code.

## Keep going

Read [Testing](testing) to drive routes and controls headless.
