---
title: Native wrappers
description: Build the C ABI libraries behind the window, text, render, and media runtime.
section: Native wrappers
order: 80
---

# Native wrappers

The runtime calls native code through small C ABI libraries. App code never
imports a wrapper and never sees a raw pointer, a surface handle, or a present
call. It imports the curated surface:

```rnx
import { App } from "@std/app"
```

Each directory under `wrappers/` is a Rust crate. The app loads four C ABI
libraries directly: `host`, `parley`, `cpu`, and `media`. `host` speaks xdg-shell
through Wayland and presents through wgpu and vello. `parley` shapes text. `cpu`
is the CPU renderer. `media` adds audio, video, camera, and a file picker. The
`wgpu`, `vello`, `taffy`, and `scene` crates sit behind the host and its layout
and paint probes.

The package declares the four libraries the app loads directly in
`Project.config`:

```rnx
export default {
    project: { name: "@std/app", version: "0.3.1" },
    artifacts: "ghcr.io/rovelstars/rnx-stdlib-app",
    dependencies: {
        cpu_cabi: { native: "cpu_cabi", from: "libcpu_cabi.so", version: ">=0.1.0" },
        host_cabi: { native: "host_cabi", from: "libhost_cabi.so", version: ">=0.1.0" },
        parley_cabi: { native: "parley_cabi", from: "libparley_cabi.so", version: ">=0.1.0" },
        media_cabi: { native: "media_cabi", from: "libmedia_cabi.so", version: ">=0.1.0" },
    },
}
```

`artifacts` names the OCI image that carries the four runtime libraries. Each
`from` is the blob title inside that image. `rnx add` verifies the image index
digest pinned in the lock, downloads only the titles the package declares, and
checks every blob digest before linking. The image is tagged `latest`, so the
pinned index digest is what makes an install reproducible.

Use `RNX_PREFER_NATIVE=system` to test against host-installed wrappers
instead of the image.

## Build and install

`tools/build-wrappers.sh` builds the crates and installs the `.so` and `.pc`
files into `.rnx-cache/wrappers` by default. Set `WRAPPER_PREFIX` to install
elsewhere, or `CARGO_TARGET_DIR` to move the Cargo output. Run these from the
repository root:

```sh
compiler/stdlib/src/app/tools/build-wrappers.sh
export PKG_CONFIG_PATH="$PWD/.rnx-cache/wrappers:$PKG_CONFIG_PATH"
export LD_LIBRARY_PATH="$PWD/.rnx-cache/wrappers:$LD_LIBRARY_PATH"
rnx run --backend cranelift
```

The script exits on a non-Linux host. The window host, accessibility bridge,
audio output, camera capture, and file picker are Linux backends. The language,
compiler, and headless image and scene paths stay available on other platforms,
but there is no window host or accessibility bridge yet.

`tools/ci-wrappers.sh` is the full local check. It stages the stdlib vendor
tree, builds every wrapper, regenerates the bindings, runs the wrapper unit
tests, runs the probe programs, and runs the app tests on Cranelift and LLVM. It
also runs the surface socket probe, and the multi-window probe when
`RNX_MULTI_PROBE=1` and a Wayland session are present.

## Bindings

The declarations under `.rnx-cache/generated/<lib>/bindgen.rnx` are generated,
not committed. `rnx task wrappers` runs `tools/bindgen.sh` and regenerates them
from every header:

```sh
rnx task wrappers
```

The underlying command is:

```sh
rnx bindgen \
    --header wrappers/host/include/host_cabi.h \
    --native-lib host_cabi \
    --pkg-name std-app \
    --out .rnx-cache/generated/host_cabi \
    --case camel
```

Keep generated output out of git. CI regenerates it before a job that needs it,
and a drift check compares against the generated artifact, not a checked-in copy.

## The 64-bit return rule

A wrapper function that the app reads as `Int` must return `int64_t` in its
header and `i64` in Rust. The call site reads the full 64-bit return register. A
32-bit `int` return is zero-extended, so `-1` arrives as `4294967295`. That
breaks every negative status code: a wheel-up delta can clamp a list to its
maximum, a GPU failure never reaches the CPU fallback, and a `-4` present retry
is read as success.

```c
int64_t host_present(host h, const uint8_t* list, int32_t len, const int32_t* damage, int32_t count);
```

Keep the header, the Rust signature, and the rnx declaration on the same width.

## Runtime notes

The interpreter's foreign-call path accepts `Int`, `Bool`, and `Pointer`
arguments but not `Float`. Use a compiled backend for an app that calls the
runtime. `rnx run` uses Cranelift by default; pass `--backend llvm` for the LLVM
path.

`App.setSurfaceSocket(path)` and `RNX_SURFACE_SOCKET` route presented frames
through `host_surface_socket_open`, `host_surface_socket_commit`,
`host_surface_socket_id`, and `host_surface_socket_close`. That is the same
`host_cabi` seam a runixOS surface host uses in place of the Wayland window.

## Keep going

Read [Packaging](packaging) to declare an app and write its manifest.
