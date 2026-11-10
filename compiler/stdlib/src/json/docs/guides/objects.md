---
title: "Objects and maps"
description: Read JSON objects with parseObject and asMap, and keep ownership of the parsed tree correct.
section: "Objects"
order: 30
---

# Objects and maps

A JSON object does not decode to a `Map` on its own. `JSON.parse()` returns an
opaque handle so the parser can keep the document in one compact buffer. To
read fields, wrap the handle in a map. `parseObject()` does both steps at
once; `asMap()` wraps a handle you already have.

## Parse an object into a map

```rnx
import { JSON } from "@std/json";

let m = JSON.parseObject("\{\"count\": 42, \"name\": \"rasmalai\"}");
println!(m.get("name") ?? "missing");
println!(m.len(), m.keys().join(","));
```

```text
rasmalai
2 count,name
```

The keys arrive in byte-sorted order. `get()` returns the value or null when
the key is absent. `get()`, `has()`, and `len()` read the parsed document
directly.

`parseObject()` requires an object at the document root. A different root
aborts with `json object expected`.

## Wrap an existing handle

`asMap()` is for an object handle that came from `JSON.parse()` or from a
field of another object. It takes the handle over, so wrap each handle once
and stop reading the value it came from.

```rnx
import { JSON } from "@std/json";

let doc: Any = JSON.parse("\{\"ok\": true, \"nested\": \{\"k\": 1}}");
let view = JSON.asMap(doc);
println!(view.get("ok"));

let nested = view.get("nested");
if nested != null {
    let inner = JSON.asMap(nested);
    println!(inner.get("k"));
}
```

```text
true
1
```

A handle read out of a nested object keeps the shared document alive on its
own. The wrapper takes ownership of the handle it is given, and the document
is freed when its owning map goes out of scope.

## What the map can do

`get`, `has`, and `len` scan the parsed pairs directly. `set`, `delete`,
`clear`, `keys`, and `values` first materialize the document into a hash map,
so they cost more than a read. The map is still a normal `Map<String, Any>`
after that.

Do not keep the original `Any` handle and a wrapper for the same object at the
same time. Choose one owner and read through it.
