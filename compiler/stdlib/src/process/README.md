# @std/process

Control the host process and run child processes, with captured output or live pipes.

```rnx
import { Process } from "@std/process";

let out = Process.run("echo", ["hello"]);
println!(out.exitCode);
println!(out.stdoutText());
println!(out.stderrText().length());
```

Read the [overview](/docs/@std/process/overview) or open the [API reference](/docs/@std/process/api).
