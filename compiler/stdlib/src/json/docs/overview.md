# @std/json

JSON parsing and serialization over the native tape parser.

`JSON.parse` turns a document into plain values: `null` decodes to
`null`, `true` and `false` to `Bool`, numbers to `Int` or `Float`,
strings to `String`, arrays to `Array<Any>`, and objects to opaque
handles. Wrap an object handle in a `Map` with `parseObject` or
`asMap` and read fields with `Map.get`.

```rnx
import { JSON } from "@std/json";

print(JSON.stringify(JSON.parse("12345")));
```

## What is inside

- `JSON`

## When to use

Reach for @std/json at every process boundary that speaks JSON: parsing request bodies and config files with JSON.parse, emitting responses with JSON.stringify, or pulling typed maps out with JSON.parseObject and JSON.asMap.

It covers: JSON.parse/stringify; parseObject to Map; asMap/asArray casts.
