# host wrapper

The Linux development window. It speaks xdg-shell through `wayland-client`,
creates a wgpu surface from the raw `wl_display` and `wl_surface` pointers,
renders a display list with vello, and presents it.

No winit, no smithay-client-toolkit. The crate owns the event loop, configure and
ack, seat, pointer, keyboard, and close handling. `host_cabi.h` is the surface.

```sh
cargo build --release
```

`App.show` in `@std/app` is the only caller. The wrapper does not run on runixOS:
Epiclese provides the surface there behind the same `host_cabi` call set.

Dependencies are pinned to `wayland-client` 0.31.15, `wayland-protocols`
0.32.13, `raw-window-handle` 0.6.2, wgpu 30, and vello 0.11. The `system`
feature makes `wayland-client` use the system libwayland, which is required for
the raw display pointer.

wgpu is built Vulkan-only and Vello compiles Area AA only. That avoids loading
Mesa's GL/gallium path for an app that only presents through Vulkan. The surface
uses one frame of latency and an opaque alpha mode when the driver offers it.

Set `RUNIX_TRACE=1` to print display-list parse, surface acquire, and total
present time for each frame to stderr.
