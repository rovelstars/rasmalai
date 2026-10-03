---
title: "Coming from C/C++: Native Speed Without Undefined Behavior"
description: "-O3 performance and direct C-ABI binding without headers, macros, or CMake."
sourceLang: "cpp"
---

# Coming from C/C++: Native Speed Without Undefined Behavior

Rasmalai compiles through LLVM `-O3` to native code and calls C functions
directly through `from native "c"` imports - no headers, no macros, no
CMake, one `rnx` binary for build/test/fmt/lint. The trade: no templates, no
undefined-behavior escape hatches, no control over object layout. This page
shows what your C++ patterns look like without the toolchain around them.

## No headers, no macros, no CMake

One binary replaces the toolchain sprawl: build, test, format, lint, docs,
and completions ship together:

```cpp
// C++: CMakeLists.txt + vcpkg.json + headers + TUs, then wait
// cmake -B build -DCMAKE_BUILD_TYPE=Release && cmake --build build
```

```sh
rnx build sim.rnx --release
rnx test
rnx fmt
```

## Vectors without intrinsics headers

No `<immintrin.h>`, no `_mm_add_ps` spelling bee, no alignment attributes to
get wrong. The type is the instruction:

```cpp
#include <array>
#include <cstdio>

int main() {
    std::array<float, 4> a = {1.0f, 2.0f, 3.0f, 4.0f};
    float x = a[0] * 2.0f;
    float w = a[3] * 2.0f;
    std::printf("%g %g\n", x, w);
    return 0;
}
```

```rnx
import { Vec4f } from "@std/simd";

let a = new Vec4f(1.0, 2.0, 3.0, 4.0);
let b = Vec4f.splat(2.0);
let c = a * b;
print(c.x(), c.w());
```

## Bounds checked, then proven away

Out-of-bounds indexing traps instead of reading your neighbor's secrets.
Then Bounds Check Elimination hoists the proof out of hot loops, so the
safety costs nothing at `-O3`:

```cpp
#include <cstdio>
#include <vector>

int main() {
    std::vector<int> arr = {10, 20, 30};
    int sum = 0;
    for (int x : arr) {
        sum += x;
    }
    std::printf("%d %d\n", sum, arr.at(2));
    return 0;
}
```

```rnx
let arr = [10, 20, 30];
let sum = 0;
for x in arr {
    sum = sum + x;
}
print(sum, arr[2]);
```

## Allocation without malloc/free

Heap arrays are refcounted values with destructors the compiler writes.
There is no `new` without `delete`, because there is no manual pairing at
all:

```cpp
#include <cstdio>
#include <vector>

int total(int n) {
    std::vector<int> arr;
    for (int i = 0; i < n; i++) {
        arr.push_back(i);
    }
    int sum = 0;
    for (int x : arr) {
        sum += x;
    }
    return sum;
}

int main() {
    std::printf("%d\n", total(5));
    return 0;
}
```

```rnx
fn total(n: Int): Int {
    let arr: Array<Int> = [];
    let i = 0;
    while i < n {
        arr.push(i);
        i = i + 1;
    }
    let sum = 0;
    for x in arr {
        sum = sum + x;
    }
    return sum;
}

print(total(5));
```

Keep the `-O3` and the pointer-free hot loop. Lose the undefined behavior.
