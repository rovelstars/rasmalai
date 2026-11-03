# app-window

A live window on Wayland. Build the wrappers once, then run:

```sh
sh compiler/stdlib/src/app/tools/build-wrappers.sh
sh examples/app-window/run.sh
```

Click the button, press Tab and Enter, or press Escape to close. Set
`RUNIX_SMOKE=1` to present three frames and exit:

```sh
RUNIX_SMOKE=1 sh examples/app-window/run.sh
```

The window maps as `app_id=dev.runix.app`. The host is `wrappers/host`; the app
only calls `App.show` and `App.showFor`.

The view calls `b.animate(false)` because nothing here moves on its own. The
window then redraws once per click or key press and sleeps in between, instead
of presenting identical frames.

Set `RUNIX_HEADLESS=1` to render one frame offscreen and exit. The wrapper CI
uses that instead of opening a window.
