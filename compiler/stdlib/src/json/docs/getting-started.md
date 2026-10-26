# Getting started with @std/json

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { JSON } from "@std/json";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { JSON } from "@std/json";

print(JSON.stringify(JSON.parse("12345")));
let obj = JSON.parseObject("{\"count\": 41}");
print(obj.get("count"));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
