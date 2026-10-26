# Getting started with @std/bytes

No install step: the module ships inside the compiler. Import what you need:

```rnx
import { ByteBuffer } from "@std/bytes";
```

## Copy this

Paste it into a file or run it straight in the playground:

```rnx
import { ByteBuffer } from "@std/bytes";

let buf = ByteBuffer.allocate(16);
buf.writeInt32BE(0, 0x01020304);
print(buf.readInt32BE(0));
print(buf.writeString(4, "Rasmalai"));
print(buf.readString(4, 8));
```

## Keep going

Read the [overview](overview) for the full picture, then open the API reference for exact signatures.
