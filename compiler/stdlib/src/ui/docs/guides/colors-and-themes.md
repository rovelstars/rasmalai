---
title: Colors and themes
description: Pack colors with rgba, read the Theme scales, and set surface effects.
section: Theming
order: 80
---

# Colors and themes

A color is an `Int` in `0xRRGGBBAA` order. `rgba(r, g, b, a)` packs the four
channels and clamps each one to 0 through 255, so a computed channel can be
passed without a guard:

```rnx
import { Card, ElevationToken, Page, Text, Theme, View, rgba } from "@std/ui"

fn Branded(): View {
    let theme = Theme.dark()
    let card = Card(title: "Brand", elevation: ElevationToken.level2)
    card.shadowColor = rgba(0, 0, 0, 170)
    card.shadowDx = 0.0
    card.shadowDy = 10.0
    card.shadowBlur = 12.0
    card.shadowRadius = 12.0
    return Page(padding: 20, gap: 12) {
        Text("Accent " + theme.accent.toString())
        card
    }
}
```

`Theme.dark()`, `Theme.light()`, and `Theme.highContrast()` are data. They share
the same scales and differ in color and borders. The semantic colors are
`background`, `surface`, `surfaceRaised`, `surfaceSunken`, `text`, `textMuted`,
`accent`, `accentHover`, `accentText`, `muted`, `border`, `focus`, `danger`,
`success`, `warning`, and `disabled`.

## Scales

The theme carries five scales as typed values, not string lookups:

```rnx
import { ElevationToken, MotionToken, RadiusToken, SpaceScale, SpaceToken, Theme, TypeToken } from "@std/ui"

fn Tokens(): Int {
    let theme = Theme.light()
    let body = theme.type(TypeToken.body)
    let gap = theme.space(SpaceToken.md)
    let radius = theme.radius(RadiusToken.md)
    let lift = theme.elevation(ElevationToken.level2)
    let normal = theme.motion(MotionToken.normal)
    let compact = theme.withSpaceScale(SpaceScale.compact())
    let tighter = compact.withSpace(SpaceToken.lg, 12)
    return gap + Int(radius) + Int(lift.dy) + Int(normal.duration * 100.0) + tighter.space(SpaceToken.md) + Int(body.size)
}
```

`TypeStyle` carries size, weight, line height, and tracking. `Elevation` carries
offset, blur, and alpha. `MotionSpec` carries a duration and curve, or the spring
constants for the spring token.

`withType`, `withSpace`, `withRadius`, `withElevation`, and `withMotion` replace
one token and return a new theme. The whole-scale variants,
`withTypeScale`, `withSpaceScale`, `withRadiusScale`, `withElevationScale`, and
`withMotionScale`, swap a complete scale. Nothing mutates in place, so a theme
can be derived without affecting another screen.

The default scale matches the geometry the roles used before tokens existed, so
adopting a token does not move a screen.

## Tones and elevation

Controls read the theme through their role rather than through a hardcoded
color. `Button(tone:)` selects neutral, accent, danger, or ghost.
`Card(elevation:)` selects one of six shadow steps, from flat to level 5.
`Button(size:)` resolves padding and height through the space scale:

```rnx
import { Button, ButtonSize, ButtonTone, Card, ElevationToken, Stack, View } from "@std/ui"

fn Actions(): View {
    return Stack(horizontal: true, gap: 8) {
        Button("Save", () => {}, tone: ButtonTone.accent, size: ButtonSize.lg)
        Button("Delete", () => {}, tone: ButtonTone.danger)
        Card(title: "Raised", elevation: ElevationToken.level3) {
            Button("Open", () => {}, tone: ButtonTone.ghost)
        }
    }
}
```

## Surfaces and compositing

`opacity` applies once to the composited group, not to each child, so two
overlapping children do not darken twice. `blend` combines that group with the
backdrop. Both are `View` attributes:

```rnx
import { BlendMode, Card } from "@std/ui"

let panel = Card(title: "Floating")
panel.opacity = 0.92
panel.blend = BlendMode.screen
```

`clipRadius` rounds a clip. `maskRadius` cuts an alpha mask, so pixels outside
the rounded shape are removed rather than clipped. `blurSigma` applies a
Gaussian blur to the whole subtree:

```rnx
import { Canvas, Card, Scene } from "@std/ui"

let cut = Card(padding: 0) {
    Scene("art", 360, 160, (c: Canvas) => { c.fill(0) })
}
cut.maskRadius = 24.0
cut.blurSigma = 6.0
```

A shadow is a blurred silhouette drawn before the node, controlled by
`shadowColor`, `shadowDx`, `shadowDy`, `shadowBlur`, and `shadowRadius`.
`ElevationToken` resolves the same values from the theme, so a card can follow
the scale instead of picking numbers by hand.
