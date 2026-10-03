---
title: "Coming from TypeScript: From V8 JIT to Bare Metal"
description: "Familiar module ergonomics compiling to a 356 KB native binary."
sourceLang: "typescript"
---

# Coming from TypeScript: From V8 JIT to Bare Metal

Rasmalai will feel familiar: annotated bindings (`let x: Int`), interfaces,
enums with payloads, generics that erase at runtime, `async`/`await` over
`Promise<T>`, and `"${}"` string interpolation all work the way TypeScript
taught you. What changes is the substrate: `number` splits into
hardware-sized `Int`/`Float`, and V8 plus `node_modules` collapses into a
~356 KB native binary. This page maps each TypeScript habit to its
equivalent, flagging where the type system is stricter.

## Modules you already recognize

Imports name what they take and where it lives. The standard library resolves
from the compiler itself — nothing to install:

```ts
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

Every `number` in TypeScript is a double. Rasmalai sizes numbers like the
machine does: `Int` is 64 bits, `Float` is IEEE-754 double, and mixing the
relaxed `FastFloat` without a cast is a compile error instead of a silent
precision change:

```ts
const a: number = 0.5;
console.log(Math.trunc(3.9), a * 2);
```

```rnx
let a: Float = 0.5;
let fast = a.asFast();
print(Int(3.9), fast * 2.0.asFast());
```

## Config from the environment

No `process.env` dynamics, no `dotenv` chain. `Env` reads and writes process
state through one namespace:

```ts
process.env["MODE"] = "bare";
console.log(process.env["MODE"]);
```

```rnx
import { Env } from "@std/env";

Env.set("MODE", "bare");
print(Env.get("MODE"));
```

## Ship one file

No bundler split-chunks, no runtime flags, no container layer for the
interpreter. `rnx build --release` emits the binary; `rnx dist` thinking is
`scp` thinking:

```sh
rnx build app.rnx --release   # stripped ~356 KB binary, zero dependencies
```

Keep the modules and the iteration speed. Lose the runtime.
