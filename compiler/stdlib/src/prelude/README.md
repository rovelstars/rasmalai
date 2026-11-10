# @std/prelude

The names every Rasmalai compilation unit sees without an import, and how a target replaces them.

```rnx
println!("no import needed");
let found = [1, 2, 3].find((n: Int): Bool => n > 1);
println!(found ?? -1);
assert("abc".contains("b"), "prelude");
```

Read the [overview](/docs/@std/prelude/overview) or open the [API reference](/docs/@std/prelude/api).
