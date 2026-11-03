# wgpu-cabi

A flat C ABI over `wgpu`. The Rust side owns the objects; the Rasmalai side sees
`Pointer<Byte>` handles, integers, and caller-owned buffers.

This is the pilot for the native stack behind `@std/app`. It proves the whole
pipeline: wrapper crate, static and shared library, `from native` import,
symbol resolution, GPU execution, and readback.

## Run the probe

```sh
RNX_BIN=/path/to/rnx ./probe/build.sh
```

Expected output:

```
adapters: 2
name bytes: 39
rc: 0 rgba: 10 20 30 255
triangle rc: 0 center: 51 204 102 255
abi: 1
```

`adapters` depends on the machine. Two results matter: a 2x1 RGBA8 texture was
cleared to `(10, 20, 30, 255)` and copied back byte exact, and a WGSL pipeline
drew a triangle whose center pixel is `(51, 204, 102, 255)`, which is the shader
color quantized to eight bits.

## ABI rules this follows

- Handles are `Pointer<Byte>`, created by one function and destroyed by another.
  No structs cross by value.
- Results come back through caller-owned buffers, because the host owns the
  memory and there is no allocation return in the ABI.
- Every call returns a status `Int`. Negative means failure, and the code names
  which stage failed.
- No callbacks. The Rasmalai side drives, the Rust side answers.

## Linking

For development the probe uses `system: true` plus a pkg-config file and
`LD_LIBRARY_PATH`, because the JIT resolves symbols through the host loader.

For release the crate builds a `staticlib` and the package ships it as an
`artifacts` blob, which `rnx build` links into the executable. `rnx run` keeps
using the shared object.

## Why a wrapper and not bindgen alone

`rnx bindgen` can read a header, but `wgpu` has no C header and no stable C ABI.
Forking it is out of the question, so the wrapper crate is the ABI: it fixes the
signatures, owns the lifetimes, and stays small enough to audit.
