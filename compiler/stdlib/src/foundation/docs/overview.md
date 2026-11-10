# @std/foundation

Foundation types shared by every target prelude. The package declares the
language-level catalog (`Int`, `Float`, `FastFloat`, `Bool`, `String`,
`Char`, `Array`, `Map`, `Set`, `Error`, `Any`, `Void`, `GenRef`, `Result`,
`Iterator`, `Iterable`, and `ArrayIter`) plus the `Array` and `String`
extension methods.

The package imports only the native `runtime "bytes"` and `runtime "core"`
shims. It does not import `@std/prelude`, `@std/io`, or `@std/log`, so a
target prelude can provide the foundation names without pulling console or
logging runtime code.

The stock `@std/prelude` re-exports these names and adds `Date`, the promise
types, `RnxHost`, the print and log macros, and the `IO`, `Thread`, and
`ThreadPool` catalog stubs.

```rnx
fn parse(text: String): Result<Int, String> {
    if text == "7" {
        return Result.Ok<Int, String>(7);
    }
    return Result.Err<Int, String>("not a number: " + text);
}

fn Main(): Int {
    let values = [3, 7, 9];
    return values.find((n: Int): Bool => n == 7) ?? -1;
}
```
