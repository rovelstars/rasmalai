# @std/env

Read command-line arguments, environment variables, and the working directory of the running process.

```rnx
import { Env } from "@std/env";

let args = Env.args();
println!("program:", args[0]);
println!("arguments:", args.len() - 1);
Env.set("MODE", "demo");
println!("mode:", Env.get("MODE"));
println!("cwd:", Env.cwd());
```

Read the [overview](/docs/@std/env/overview) or open the [API reference](/docs/@std/env/api).
