---
title: "@std/json"
description: Parse and serialize JSON, including typed decoding and object maps.
section: Overview
order: 0
---

# @std/json

`@std/json` is the JSON codec. `JSON.parse()` turns a document into plain
values, `JSON.stringify()` writes a value back as canonical JSON, and
`JSON.parseObject()` or `JSON.asMap()` gives an object document a `Map`
interface.

```rnx
import { JSON } from "@std/json";

println!(JSON.stringify(JSON.parse("\{\"b\": 2, \"a\": 1}")));
```

The output is `{"a":1,"b":2}`. Keys come out in byte-sorted order, so the
same document always serializes to the same text. A number with no fraction
and no exponent decodes to `Int` while it fits a signed 64-bit integer;
larger integers and every other number decode to `Float`. Strings decode to
`String`, arrays to `Array<Any>`, and objects to opaque handles.

An object handle is not a `Map` until you wrap it. `parseObject()` parses an
object document and returns a map that owns the parsed tree. `asMap()` views
an existing handle as a map and takes the handle over. Read fields with
`Map.get()`, `has()`, `len()`, `keys()`, and `values()`.

Documents may nest 64 levels deep. A repeated key keeps its last value.
Malformed input aborts with a parse message and offset rather than returning
an error value.

For structs and classes, `JSON.parse<T>()` and `JSON.decode<T>()` read an
object document straight into `T` without a tape or a map. Keys are matched
against declared fields in any order, unknown keys are skipped, and missing
keys decode as null.

## What is inside

- `JSON`: `parse()`, `parse<T>()`, `decode<T>()`, `stringify()`,
  `stringifyInto()`, `parseObject()`, `asMap()`.

## Guides

- [Parsing](guides/parsing) covers value parsing, typed decoding, and the
  errors a document can produce.
- [Objects and maps](guides/objects) covers `parseObject()`, `asMap()`, and
  the ownership rule for parsed trees.
- [Serialization](guides/serialization) covers canonical output and writing
  into a byte buffer.
