---
title: "From TypeScript"
description: "Dynamic types become structural classes with native 64-bit numbers."
sourceLang: "typescript"
---

# From TypeScript: Familiar Shape, Bare-Metal Runtime

You like modules, curly braces, and fast iteration. Rasmalai keeps the shape of the code and swaps the runtime: a stripped native binary instead of V8, milliseconds instead of `node_modules`.

## Modules you already recognize

Imports name what they take and where they live. The standard library resolves from the compiler itself — nothing to install:

```typescript
class Vec2 {
    x: number;
    y: number;
    constructor(x: number, y: number) {
        this.x = x;
        this.y = y;
    }
    length(): number {
        return Math.sqrt(this.x * this.x + this.y * this.y);
    }
}

const v = new Vec2(3, 4);
console.log(v.length());
```

```rnx
import { Vec2 } from "@std/math";

let v = new Vec2(3.0, 4.0);
print(v.length());
```

## Numbers with hardware sizes

Every `number` in TypeScript is a double. Rasmalai sizes numbers like the machine does: `Int` is 64 bits, `Float` is IEEE-754 double, and mixing the relaxed `FastFloat` without an explicit conversion is a compile error instead of a silent precision change:

```typescript
const a: number = 0.5;
console.log(Math.trunc(3.9), a * 2);
```

```rnx
let a: Float = 0.5;
let fast = a.asFast();
print(Int(3.9), fast * 2.0.asFast());
```

## Null-safety without undefined

There is no `undefined`, and absence is explicit: a missing value is `null` with a nullable type (`T?`), and the operators you know transfer directly:

```typescript
const port = process.env["PORT"] ?? "8080";
console.log(`listening on ${port}`);
```

```rnx
import { Env } from "@std/env";

Env.set("PORT", "8080");
let port = Env.get("PORT");
print("listening on {port}");
```

## Ship one file

No bundler split-chunks, no runtime flags, no container layer for the interpreter:

```sh
rnx build --release
```

One stripped native binary, zero dependencies. Next: the [Guide](/guide/01-introduction) from the top, or [Modules and Packages](/guide/08-modules-and-packages) for the manifest and registry.
