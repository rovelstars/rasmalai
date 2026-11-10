# @std/bytes

Fixed-size byte buffers with explicit byte order for integers, floats, and text.

```rnx
import { ByteBuffer } from "@std/bytes";

let header = ByteBuffer.allocate(16);
header.writeUInt32BE(0, 0x89504e47);
let n = header.writeString(4, "Rasmalai");
println!(header.readUInt32BE(0));
println!(n);
println!(header.readString(4, n));
```

Read the [overview](/docs/@std/bytes/overview) or open the [API reference](/docs/@std/bytes/api).
