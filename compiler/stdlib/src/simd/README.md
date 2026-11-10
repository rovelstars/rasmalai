# @std/simd

128-bit float and integer vectors that stay in registers and allocate nothing.

```rnx
import { Vec4f } from "@std/simd";

let v = Vec4f.splat(2.0);
println!(v.x() == 2.0, v.w() == 2.0);
let sum = v + Vec4f.splat(3.0);
println!(sum.z());
```

Read the [overview](/docs/@std/simd/overview) or open the [API reference](/docs/@std/simd/api).
