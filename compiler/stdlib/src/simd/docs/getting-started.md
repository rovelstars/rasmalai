---
title: "Getting started with @std/simd"
description: "Import Vec4f and Vec4i and run a first vector program."
section: Getting started
order: 10
---

# Getting started with @std/simd

There is no install step. The package ships inside the compiler. Import the vector types you need:

```rnx
import { Vec4f } from "@std/simd";
```

## Copy this

Save it as the entry file of a project and run `rnx run`, or start with `rnx init` and replace `src/main.rnx`:

```rnx
import { Vec4f } from "@std/simd";

let v = Vec4f.splat(2.0);
println!(v.x() == 2.0, v.w() == 2.0);
let sum = v + Vec4f.splat(3.0);
println!(sum.z());
```

## What to expect

The first line is `true true`, because `splat(2.0)` sets all four lanes to 2.0. The second line is the `z` lane of the sum, which is 5.
