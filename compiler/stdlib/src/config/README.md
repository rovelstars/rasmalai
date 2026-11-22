# @std/config

Parse and write the sandboxed `.config` document format, with no filesystem
dependency.

```rnx
import { Config } from "@std/config"

let doc = Config.parse("export default \{ name: \"demo\", version: \"0.1.0\" }")
println!(doc.text("name"))
println!(Config.stringify(doc))
```

Read the [overview](/docs/@std/config/overview) or open the [API reference](/docs/@std/config/api).
