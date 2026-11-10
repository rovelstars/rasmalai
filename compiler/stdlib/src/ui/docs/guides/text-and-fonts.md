---
title: Text and fonts
description: Size and align text, shape styled spans, add links, and register font faces.
section: Text
order: 50
---

# Text and fonts

Text is a view, not a drawing call. `Text`, `Heading`, and `DisplayText` create
text nodes, and the layout pass shapes them. A paragraph wraps to the width it
is given, while a button label stays on one line and clips with an ellipsis.

The example screen reads a `Report` with a `body` string and a `total` string:

```rnx
import { Align, DisplayText, Page, Text, View } from "@std/ui"

fn Report(state: Report): View {
    let title = Text("Quarterly report", align: Align.start)
    title.fontWeight = 700
    title.textSize = 28

    let body = Text(state.body, align: Align.start)
    body.lineHeight = 1.6
    body.letterSpacing = 0.5

    return Page(title: "Report", padding: 24, gap: 16) {
        title
        DisplayText(state.total, align: Align.end, width: 240)
        body
    }
}
```

`fontWeight` runs from 100 to 900. `italic` switches the face. `lineHeight` is
a multiplier, and `0` keeps the normal height. `letterSpacing` is measured in
logical pixels. `textSize` is optional and overrides the role size.

`Heading` uses the heading role and sets level 2 by default. `DisplayText` uses
the display role, which is 56 pixels unless the theme changes it, and takes a
`width` so a large number can be aligned in a fixed slot.

## Styled runs

A paragraph can mix styles with `RichText`. Each `TextSpan` covers a character
range, and a later span wins where ranges overlap:

```rnx
import { Align, FontFamily, RichText, TextDecoration, TextSpan, Theme, View } from "@std/ui"

fn Styled(): View {
    let theme = Theme.dark()
    let spans = [
        new TextSpan(0, 4, 700),
        new TextSpan(5, 11, 400, true),
        new TextSpan(12, 17, 400, false, 0.0, theme.accent),
        new TextSpan(18, 26, 400, false, 2.0),
        new TextSpan(26, 31, 400, false, 0.0, -1, FontFamily.serif),
        new TextSpan(32, 41, 400, false, 0.0, -1, FontFamily.sans, TextDecoration.underline),
    ]
    return RichText("Bold italic color tracked serif underline", spans, align: Align.start)
}
```

The span constructor takes `start`, `end`, `weight`, `italic`, `tracking`,
`color`, `family`, `decoration`, `font`, and `link`. Offsets count characters,
the same way `String.slice` does. A color of `-1` keeps the role color, so the
default text color follows the theme.

`RichTextField` keeps the same span list while the text changes. The handler
receives both the new value and the adjusted spans, so the app can store the two
together:

```rnx
import { RichTextField, TextSpan } from "@std/ui"

class Note {
    let value: String = ""
    let spans: Array<TextSpan> = []
}

let note = new Note()
let editor = RichTextField(note.value, note.spans, (value: String, spans: Array<TextSpan>) => {
    note.value = value
    note.spans = spans
}, "Type here")
```

## Links

A span with a link URL paints as a link and becomes a link node in the semantic
tree. It uses the theme accent and an underline unless the span sets its own
color or decoration:

```rnx
import { FontFamily, RichText, TextDecoration, TextSpan, View } from "@std/ui"

fn ReleaseNote(): View {
    let spans = [
        new TextSpan(9, 17, 500, false, 0.0, -1, FontFamily.sans, TextDecoration.none, null, "https://example.test/releases"),
    ]
    return RichText("Read the releases.", spans)
}
```

`Tree.linkAt(x, y)` returns the URL under a point and `Tree.linkRects(id)`
returns the bounds of each link. `@std/app` forwards a click to `App.onLink`
once, and the accessibility tree exposes the link text as the name and the URL
as the value. [Accessibility](accessibility) covers that projection.

## Named fonts

`FontRegistry` assigns ids from 100 and caps at 16 faces and 8 MB of font data.
Register a face once, then pass it to text spans:

```rnx
import { FontRegistry, FontFamily, RichText, TextDecoration, TextSpan, View } from "@std/ui"
import fs from "@std/fs"

fn Brand(path: String): View {
    let registry = new FontRegistry()
    let face = registry.register("Inter", fs.readBytes(path).unwrap())
    let spans = [
        new TextSpan(0, 5, 400, false, 0.0, -1, FontFamily.sans, TextDecoration.none, face),
    ]
    return RichText("Inter", spans)
}
```

`registry.register` returns null when the name is empty, the bytes are empty, or
a cap is hit. Check the result before you rely on it.
`registerFonts(registry)` from `@std/app` hands every face to both renderers and
returns how many were accepted; font bytes register once, not once per frame.
