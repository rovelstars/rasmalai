# @std/os

Read the platform, architecture, CPU count, directories, and uptime of the host.

```rnx
import { OS } from "@std/os";

println!(OS.platform(), OS.arch());
println!(OS.cpuCount());
println!(OS.tmpdir());
```

Read the [overview](/docs/@std/os/overview) or open the [API reference](/docs/@std/os/api).
