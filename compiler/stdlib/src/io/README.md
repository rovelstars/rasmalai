# @std/io

Standard streams, pretty-printed output, line input, terminal size, colors, and raw mode.

```rnx
import io, { ColorLevel } from "@std/io";

io.write("hello");
io.write([1, 2, 3]);
println!("width", io.width(), "height", io.height());
println!(io.colorProfile() == ColorLevel.Ascii);
```

Read the [overview](/docs/@std/io/overview) or open the [API reference](/docs/@std/io/api).
