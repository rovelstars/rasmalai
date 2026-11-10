---
title: "Serialization"
description: Write canonical JSON text, or write it directly into a byte buffer.
section: "Serialization"
order: 40
---

# Serialization

Two programs that serialize the same data should produce the same bytes.
`JSON.stringify()` writes compact JSON with object keys in byte-sorted order,
so a document that arrived in a different order comes back canonical.

## Write canonical text

```rnx
import { JSON } from "@std/json";

let doc: Any = JSON.parse("\{\"q\": \"a\\\"b\", \"t\": \"x\\ty\"}");
println!(JSON.stringify(doc));
```

```text
{"q":"a\"b","t":"x\ty"}
```

There are no spaces between tokens. `"` and `\` are escaped, and control
characters use `\n`, `\r`, `\t`, `\b`, `\f`, or `\u00xx`. A float that is
not finite writes as `null`.

Pass a scalar, an array or object that came from `parse`, or a value read out
of one. A `Map` or class instance is not a JSON document, and `stringify`
writes `null` for it. Read the values out of the map and serialize those
instead.

## Write into a byte buffer

`JSON.stringifyInto()` produces the same bytes as `stringify()` without the
intermediate `String`. Give it a destination `ByteBuffer` and the first byte
to write. It returns the number of bytes written, not counting the offset.

```rnx
import { JSON } from "@std/json";
import { ByteBuffer } from "@std/bytes";

let buf = ByteBuffer.allocate(32);
let n = JSON.stringifyInto(JSON.parse("\{\"a\": 1}"), buf, 0);
println!(n, buf.readString(0, n));
```

```text
7 {"a":1}
```

The buffer must already have room for the output from `pos`. If it does not,
the call aborts with `byte buffer out of bounds`. More than 64 levels of
nesting aborts while writing.
