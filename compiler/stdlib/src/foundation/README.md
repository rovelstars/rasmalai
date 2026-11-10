# @std/foundation

The core value types, results, iteration, and array and string helpers shared by every Rasmalai target.

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
    println!(it.next() ?? 0);
    println!(firstEven([1, 2, 3]).unwrapOr(0));
    return 0;
}
```

Read the [overview](/docs/@std/foundation/overview) or open the [API reference](/docs/@std/foundation/api).
