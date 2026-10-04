---
title: "The 15-Minute Systems Crash Course"
description: "Memory, dual-engine builds, SIMD, and FFI for experienced systems engineers."
track: "fast-track"
---

# The 15-Minute Systems Crash Course

You already ship systems code. Here is the whole model, densely.

## Memory: deterministic reclamation

ARC by default. The compiler inserts atomic retains/releases; non-escaping
arguments pass as borrows with no traffic. Cyclic edges use `GenRef<T>`
generational handles (`.get()` yields `null` after the owner frees).
Intentional strong cycles opt out per field with `#[Allow(CyclicReference)]`
plus manual clearing. Destructors synthesize recursively; locals drop at scope
end. Raw pointers and C-ABI calls live in `unsafe {}` only.

## Builds: two engines

Dev loop runs unoptimized LLVM codegen: a cold dev build lands in
35-60 ms on the current baseline (`benches/data/benchmarks.json`,
2026-09-29), a hot-reload swap turns around in about 7 ms, and `rnx
check` answers with no codegen at all. Release
runs LLVM `-O3` over per-function sections, then links with section garbage
collection into a stripped ~356 KB binary. Both modes are measured on the
proving-ground chart, as `rnx run` and `rnx build sim.rnx --release`:

```sh
rnx run sim.rnx          # interpreter / JIT dev loop
rnx build sim.rnx --release  # LLVM -O3 AOT binary
```

## SIMD: lanes, not intrinsics

`Vec4f` is a 128-bit value type with no heap allocation and no ARC traffic.
Construct it, combine it with plain operators, read lanes back out. Vectors
cross function boundaries as params and returns on all three backends:

```rnx
import { Vec4f } from "@std/simd";

fn dbl(v: Vec4f): Vec4f {
    return v * Vec4f.splat(2.0);
}

print(dbl(new Vec4f(1.0, 2.0, 3.0, 4.0)).get(2));
```

## C-ABI: headers in, symbols out

`rnx build --lib` compiles top-level `export fn` items to C-ABI wrappers in a
static archive plus a `<basename>.h` header. No binding generator, no
wrapper crate, no build script:

```sh
rnx build physics.rnx --lib -o libphysics.a
```

Integers, floats, bools, and `String` (as `const char*`) cross the boundary.
Anything else in a `pub` signature fails with `E108`.

## Diagnostics and tooling

One binary: `check`, `run`, `build`, `test` (per-test timings, tree-hook
failure boxes), `bench`, `lint --fix`, `fmt`, `doc --json`, `lsp`,
`explain <code>`. Errors render as tree-hook codeframes with `▲` pointers
and `got:`/`expected:` lines.

Done. Fifteen minutes. Go build.
