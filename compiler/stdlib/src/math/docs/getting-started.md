---
title: "Getting started with @std/math"
description: "Import Math and Vec2 and run a first numeric program."
section: Getting started
order: 10
---

# Getting started with @std/math

There is no install step. The package ships inside the compiler. Import what you need:

```rnx
import { Math, Vec2 } from "@std/math";
```

## Copy this

Save it as the entry file of a project and run `rnx run`, or start with `rnx init` and replace `src/main.rnx`:

```rnx
import { Math, Vec2 } from "@std/math";

let v = new Vec2(3.0, 4.0);
println!(v.length());
println!(Math.sin(Math.PI() / 2.0));
```

## What to expect

The program prints two lines. The first is `5`, the Euclidean length of `(3.0, 4.0)`. The second is the sine of 90 degrees, which is 1.
