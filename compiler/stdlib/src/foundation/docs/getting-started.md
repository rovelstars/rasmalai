# Getting started with @std/foundation

Most programs never import this package directly. The stock prelude
re-exports its names, so `Result`, `Array`, `String`, and the rest are
already available.

Import it explicitly when writing a prelude or a library that must not
depend on the stock prelude's console and logging surface:

```rnx
import { Result, ArrayIter } from "@std/foundation";

fn firstEven(values: Array<Int>): Result<Int, String> {
    let found = values.find((n: Int): Bool => n % 2 == 0);
    if found == null {
        return Result.Err<Int, String>("no even value");
    }
    return Result.Ok<Int, String>(found as Int);
}

fn Main(): Int {
    let it = new ArrayIter<Int>([2, 4]);
    let next = it.next() ?? 0;
    return firstEven([1, 2, 3]).unwrapOr(next);
}
```

A custom prelude re-exports the foundation names and adds only the types its
target provides:

```rnx
export { Int, Float, Bool, String, Array, Result } from "@std/foundation";
```
