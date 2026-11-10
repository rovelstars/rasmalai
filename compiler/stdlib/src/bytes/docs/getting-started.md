---
title: Getting started with @std/bytes
description: Import ByteBuffer and write your first binary header.
section: Getting started
order: 10
---

# Getting started with @std/bytes

The module ships inside the compiler, so there is nothing to install. Import the class you need:

```rnx
import { ByteBuffer } from "@std/bytes";
```

## Copy this

Save it as `src/main.rnx` in a project, or paste it into the playground:

```rnx
import { ByteBuffer } from "@std/bytes";

let header = ByteBuffer.allocate(16);
header.writeUInt32BE(0, 0x89504e47);
let n = header.writeString(4, "Rasmalai");
println!(header.readUInt32BE(0));
println!(n);
println!(header.readString(4, n));
```

Run it with `rnx run`. The three lines are:

```text
2303741511
8
Rasmalai
```

`0x89504e47` is stored most significant byte first, and reading it with the same order returns the number. `writeString` returns the number of UTF-8 bytes it wrote, not the number of characters, and `readString` takes that byte count.
