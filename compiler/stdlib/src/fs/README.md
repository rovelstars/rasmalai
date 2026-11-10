# @std/fs

One-shot file reads and writes, cursor handles, directory listing, path helpers, and memory maps.

```rnx
import fs, { Path } from "@std/fs";

let path = Path.join("demo", "hello.txt");
fs.mkdirAll("demo");
fs.writeText(path, "hello", .Overwrite);
println!(fs.readText(path).unwrapOr("missing"));
println!(fs.stat(path).unwrap().size);
fs.removeAll("demo");
```

Read the [overview](/docs/@std/fs/overview) or open the [API reference](/docs/@std/fs/api).
