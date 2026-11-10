---
title: Animation
description: Fade views with Presence, layer screens with Crossfade, and step motion values.
section: Animation
order: 70
---

# Animation

A view rebuilds from state, so an animation is another value derived from time.
`@std/ui` gives you two layers: declarative presence for enter and leave, and
pure motion values for everything else.

## Presence

`Presence(show, key, transition)` keeps its children mounted while they fade
out, then removes them when the leave finishes. On the next show it fades them
back in. The key survives view rebuilds, so the fade does not restart on every
frame.

```rnx
import { Card, Presence, Text, Transition, View } from "@std/ui"

fn Toast(show: Bool): View {
    return Presence(show, "toast", Transition.fast()) {
        Card(title: "Saved") {
            Text("Your changes are on disk.")
        }
    }
}
```

`Transition.fast()`, `Transition.normal()`, and `Transition.slow()` are the
three built-in duration and curve pairs. A custom transition takes a duration, a
curve, and a fade flag:

```rnx
import { Curve, Transition } from "@std/ui"

let slow = new Transition(0.4, Curve.emphasized(), true)
```

An empty key turns the transition off and mounts the children directly. The app
keeps requesting frames while a presence transition is active, so a fading view
does not need its own timer.

`Crossfade` overlays its children on the same origin and sizes to the larger
child. Use it when two screens must share one box, such as an outgoing and an
incoming route view:

```rnx
import { Crossfade, View } from "@std/ui"

fn Screen(list: View, detail: View, showDetail: Bool): View {
    if showDetail {
        return Crossfade([list, detail])
    }
    return list
}
```

## Motion values

`Spring`, `Curve`, `Decay`, and `Motion` never read global time inside `step`.
You pass the frame delta, which keeps the result deterministic in a test:

```rnx
import { Motion, Spring } from "@std/ui"

fn Slide(dt: Float): Float {
    let x = Motion.ofSpring(Spring.fromResponse(0.45, 0.55, 0.0))
    x.to(600.0)
    x.step(dt)
    return x.value()
}
```

`Spring.fromResponse` takes a response time and a damping fraction;
`Spring.fromBounce` takes a duration and a bounce ratio. `retarget` keeps the
present value and velocity, so an interrupted animation continues instead of
restarting. `Motion.ofCurve(curve, duration)` runs a timed transition, and
`Motion.ofDecay(decay)` runs a friction fling.

`Curve` covers `linear`, `easeIn`, `easeOut`, `easeInOut`, `standard`,
`emphasized`, `cubic`, and `steps`. `Decay.normal` and `Decay.fast` are the
scroll friction profiles.

Keyframes are a pure track, sampled at an absolute elapsed time:

```rnx
import { Curve, Keyframe, KeyframeTrack } from "@std/ui"

fn Track(): Float {
    let track = new KeyframeTrack([
        new Keyframe(0.0, 40.0),
        new Keyframe(0.5, 340.0),
        new Keyframe(1.0, 640.0, Curve.easeOut()),
    ], true)
    return track.valueAt(0.25)
}
```

Frames sort by time, the outgoing frame's curve shapes each segment, and a
looping track wraps at `duration()`. Sampling the same time always returns the
same value because the track owns no clock.

## Frame deltas

Motion values are stepped by the caller. In a builder view, read the clock and
declare whether frames are still needed:

```rnx
import { Build, Node } from "@std/ui"

class Sweep {
    let x: Float = 0.0
    fn advance(dt: Float) { this.x += dt }
    fn settled(): Bool { return this.x > 4.0 }
    fn label(): String { return this.x.toString() }
}

fn Pump(b: Build, state: Sweep): Node {
    state.advance(b.frameClock().dt())
    b.animate(!state.settled())
    return b.column() {
        b.text(state.label())
    }
}
```

`b.frameClock().dt()` is the seconds since the previous frame; `elapsed()` and
`frames()` are the running totals. `b.animate(false)` stops continuous frames,
so the app redraws only on input. A value screen that uses `Presence` also keeps
frames coming while a transition runs, which covers the declarative fades in
this guide. `View.onMount` exposes the same tree through its `Tree` argument if
you need `frameDt()` there.
