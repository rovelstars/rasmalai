---
title: "From Go"
description: "Goroutines and the GC become threadpools and zero-pause ARC."
sourceLang: "go"
---

# From Go: Short Programs, No GC Pause

You like short programs that build fast. Rasmalai keeps the build speed and the readability, then removes the stop-the-world pauses and the ceiling on hardware control.

## No runtime, no pauses

Go ships a scheduler and a collector with every binary. Rasmalai ships your code: reference counts the compiler inserts, freed at scope ends the source already shows. Worst-case GC pause: 0.00 ms, because there is no collector:

```go
package main

import "fmt"

func main() {
    served := 0
    i := 0
    for i < 1000 {
        served++
        i++
    }
    fmt.Println(served)
}
```

```rnx
let served = 0;
let i = 0;
while i < 1000 {
    served = served + 1;
    i = i + 1;
}
print(served);
```

`defer` stacks exactly like Go's, and `print` takes multiple arguments like `fmt.Println` without the format-string ceremony.

## Structs and methods

Structs bundle fields with a free memberwise initializer; methods attach directly. No embedded-interface promotion puzzles:

```go
package main

import "fmt"

type Meter struct {
    Name    string
    Reading int
}

func main() {
    m := Meter{Name: "heat", Reading: 21}
    fmt.Println(m.Name, m.Reading)
}
```

```rnx
struct Meter {
    let name: String;
    let reading: Int;
}

let m = Meter("heat", 21);
print(m.name, m.reading);
```

## Real SIMD instead of assembly shims

Go reaches the vector unit through assembly stubs or compiler intrinsics. `Vec4f` is a first-class 128-bit value: no heap, no ARC traffic, plain operators:

```go
package main

import "fmt"

func main() {
    a := [4]float64{1.0, 2.0, 3.0, 4.0}
    var c [4]float64
    for i := range a {
        c[i] = a[i] * 2.0
    }
    fmt.Println(c[3])
}
```

```rnx
import { Vec4f } from "@std/simd";

let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = Vec4f.splat(2.0);
let c = a + b;
print(c.get(3));
```

## Errors without if err != nil

Returning `(T, error)` on every call litters logic with repetitive guards. Rasmalai marks fallible functions `throws` and handles them once, at the boundary:

```go
package main

import (
    "errors"
    "fmt"
)

func load(ok bool) (int, error) {
    if ok {
        return 42, nil
    }
    return 0, errors.New("nope")
}

func main() {
    v, err := load(true)
    if err != nil {
        fmt.Println("unreachable")
        return
    }
    fmt.Println(v)
}
```

```rnx
fn load(ok: Bool): Int throws {
    if ok { return 42; }
    throw 0;
}

try {
    print(load(true));
} catch (err) {
    print("unreachable");
}
```

Keep the flat, readable functions. Lose the collector and the error ceremony. Next: the [Guide](/guide/01-introduction) from the top, or the [Manual](/manual/14-concurrency-and-threads) for the threading rules.
