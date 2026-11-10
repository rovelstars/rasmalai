# @std/json

Parse and serialize JSON, including typed decoding and object maps.

```rnx
import { JSON } from "@std/json";

println!(JSON.stringify(JSON.parse("12345")));

let obj = JSON.parseObject("\{\"count\": 41}");
println!(obj.get("count"));
println!(obj.len(), obj.keys().join(","));
```

Read the [overview](/docs/@std/json/overview) or open the [API reference](/docs/@std/json/api).
