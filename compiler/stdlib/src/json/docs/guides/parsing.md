---
title: "Parsing"
description: Decode JSON values, read typed documents, and understand parse failures.
section: "Parsing"
order: 20
---

# Parsing

A JSON document can hold a scalar, an array, or an object, and each one
decodes to a different Rasmalai value. `JSON.parse()` returns `Any`, so code
that reads the result has to know which shape it expects. Typed decoding skips
that step when the document is an object.

## Decode a value

```rnx
import { JSON } from "@std/json";

let values: Any = JSON.parse("[1, 2.5, true, null, \"hi\"]");
println!(JSON.stringify(values));

let n: Any = JSON.parse("7");
let f: Any = JSON.parse("7.0");
println!(typeOf(n), typeOf(f));
```

```text
[1,2.5,true,null,"hi"]
Int Float
```

`null` decodes to `null`, `true` and `false` to `Bool`, strings to `String`,
and arrays to `Array<Any>`. A number with no fraction and no exponent decodes
to `Int` while it fits in the signed 64-bit range. Every other number,
including a larger integer, decodes to `Float`.

Object documents decode to opaque handles. Read one with `parseObject()` or
`asMap()` as described in [Objects and maps](objects).

Documents may nest 64 levels deep. A repeated key keeps its last value.
Inside a Rasmalai string, write JSON braces as `\{` and `\}` so they are not
read as interpolation.

## Decode into a struct

`JSON.parse<T>()` reads an object document directly into a struct or class.
`JSON.decode<T>()` is the same call under a second name.

```rnx
import { JSON } from "@std/json";

struct Point {
    let x: Int = 0;
    let y: Int = 0;
}

let p = JSON.parse<Point>("\{\"y\": 7, \"x\": 3}");
println!(p.x, p.y);
```

```text
3 7
```

Keys match declared fields in any order. Unknown keys are skipped, and a
missing key decodes as null. Field types are not checked against the document,
so declare a field as `Any` when the value can be an object or an array. `T`
must have at least one field and no custom `init`.

Typed decoding needs an object at the document root. A top-level array,
string, or number aborts with `typed decode needs a JSON object`.

## Parse failures

`JSON.parse()` aborts instead of returning an error value. Malformed input
reports `json parse error: <reason> at offset <n>`. Text after the top-level
value is an error too. More than 64 levels of nesting reports
`json max depth exceeded`.

Because a failure aborts, treat parsing as an operation on trusted input.
Check a document that comes from a user or the network before you parse it.
There is no `Result` form of the parser.
