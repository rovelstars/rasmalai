# ui-kit

A headless component gallery with real text shaping and a virtual list.

- buttons, a disabled button, checkbox, toggle, radio group
- a slider with a value callback
- a text field with caret, selection, and grapheme editing
- a 10,000-row virtual list that keeps about ten row nodes alive
- keyboard shortcuts: `Ctrl+K` focuses the field, `Ctrl+R` resets, `F1` opens
  the help panel, `PageUp` and `PageDown` scroll the list, `Tab` and
  `Shift+Tab` move focus, `Escape` closes the panel or the window

Build the wrappers once, then run:

```sh
sh compiler/stdlib/src/app/tools/build-wrappers.sh
sh examples/ui-kit/run.sh
```

Scroll the list with the wheel or `PageUp` and `PageDown`, click rows, type in
the field, and drag the slider. The status line shows the live node count and
the list's scroll offset; the list stays near ten rows no matter how far it
scrolls.

Set `RUNIX_GL=1` for the GL backend. `RUNIX_HEADLESS=1` renders one frame
offscreen, `RUNIX_WARM=1` builds three frames, `RUNIX_DAMAGE=1` outlines
changed regions, and `RUNIX_TRACE=1` prints damage rectangles and present
timing. `RUNIX_DUMP=/path/frame.rgba` writes the RGBA8 buffer at 940 by 780.
