# animation-demo

Three animations on one frame clock:

- a 3D cube with perspective projection and per-face shading
- an isometric 2D cube rotating in the plane
- a ball under gravity with fixed-step integration and restitution

The bottom button pauses and plays all three. While paused the app stops asking
for frames, the frame clock stops advancing, and the window redraws only on
input, so every animation freezes in place.

Build the wrappers once, then run:

```sh
sh compiler/stdlib/src/app/tools/build-wrappers.sh
sh examples/animation-demo/run.sh
```

Set `RUNIX_GL=1` to use the GL backend. Set `RUNIX_SMOKE=1` to present three
frames and exit, and `RUNIX_HEADLESS=1` to render one frame offscreen:

```sh
RUNIX_HEADLESS=1 sh examples/animation-demo/run.sh
```

`RUNIX_DUMP=/path/frame.rgba` writes the RGBA8 frame buffer from the headless
run. The size is 960 by 470.

`RUNIX_WARM=1` builds three frames before dumping, so the dump exercises the
damage path. `RUNIX_PAUSED=1` starts paused to check that an idle window uses no
CPU. `RUNIX_DAMAGE=1` outlines changed regions and `RUNIX_TRACE=1` prints the
damage rectangles and present timing.
